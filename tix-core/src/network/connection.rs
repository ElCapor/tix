//! TCP connection management with background reader/writer tasks.
//!
//! `Connection` wraps a `TcpStream` and splits it into two independent
//! background tasks communicating over mpsc channels. This avoids holding
//! a borrow across await points and gives natural back-pressure.

use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::codec::Framed;

use crate::codec::TixCodec;
use crate::error::TixError;
use crate::packet::Packet;

/// Sender half — cheaply cloneable, used to enqueue packets for the
/// background writer task.
pub type ConnectionSender = mpsc::Sender<Packet>;

/// A managed TIX connection to a single peer.
///
/// Internally spawns two Tokio tasks:
/// - **Writer**: drains `tx` and writes packets to the TCP stream.
/// - **Reader**: reads packets from the TCP stream and pushes them to `rx`.
/// - **Heartbeat**: periodically sends a heartbeat packet (reuses a static
///   instance — no cloning per tick).
#[derive(Debug)]
pub struct Connection {
    /// Send packets to the background writer.
    tx: mpsc::Sender<Packet>,
    /// Receive packets from the background reader.
    rx: mpsc::Receiver<Packet>,
}

impl Connection {
    /// Wrap an already-connected `TcpStream`.
    pub fn new(stream: TcpStream) -> Self {
        // Apply low-latency socket options.
        let _ = stream.set_nodelay(true);

        let (mut net_writer, mut net_reader) = Framed::new(stream, TixCodec).split();

        // User → Network
        let (user_tx, mut network_rx) = mpsc::channel::<Packet>(128);
        // Network → User
        let (network_tx, user_rx) = mpsc::channel::<Packet>(128);

        // Writer task
        tokio::spawn(async move {
            while let Some(packet) = network_rx.recv().await {
                if let Err(e) = net_writer.send(packet).await {
                    eprintln!("[NET] write error: {e}");
                    break;
                }
            }
        });

        // Reader task
        tokio::spawn(async move {
            while let Some(result) = net_reader.next().await {
                match result {
                    Ok(packet) => {
                        if network_tx.send(packet).await.is_err() {
                            break; // user_rx dropped
                        }
                    }
                    Err(e) => {
                        eprintln!("[NET] read error: {e}");
                        break;
                    }
                }
            }
        });

        // Heartbeat task — sends a static heartbeat every 5 seconds.
        let heartbeat_tx = user_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                // Build a fresh heartbeat each tick — it's a tiny packet with
                // zero payload and no allocation.
                if heartbeat_tx.send(Packet::heartbeat()).await.is_err() {
                    break;
                }
            }
        });

        Self {
            tx: user_tx,
            rx: user_rx,
        }
    }

    /// Send a packet to the peer.
    pub async fn send(&self, packet: Packet) -> Result<(), TixError> {
        self.tx
            .send(packet)
            .await
            .map_err(|_| TixError::ChannelClosed)
    }

    /// Receive the next packet from the peer, or `None` if the
    /// connection was closed.
    pub async fn recv(&mut self) -> Option<Packet> {
        self.rx.recv().await
    }

    /// Obtain a cloneable sender handle for use in spawned tasks.
    pub fn sender(&self) -> ConnectionSender {
        self.tx.clone()
    }

    /// Connect to a remote peer described by `ConnectionInfo`.
    pub async fn connect(info: &ConnectionInfo) -> Result<Self, std::io::Error> {
        let stream = TcpStream::connect(info.to_socket_string()).await?;
        Ok(Self::new(stream))
    }
}

// ── ConnectionInfo ──────────────────────────────────────────────

/// Describes a remote endpoint by IP and port.
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    ip: String,
    port: u16,
}

impl ConnectionInfo {
    /// Create a new connection descriptor.
    pub fn new(ip: String, port: u16) -> Self {
        Self { ip, port }
    }

    /// The peer's IP address.
    pub fn ip(&self) -> &str {
        &self.ip
    }

    /// The peer's port number.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Format as `"ip:port"` for socket binding / connecting.
    pub fn to_socket_string(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }
}

impl std::fmt::Display for ConnectionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
}
