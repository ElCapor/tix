use tokio::{net::TcpStream, sync::mpsc};
use tokio_util::codec::Framed;
use futures::{SinkExt, StreamExt};
use crate::TixCodec;

pub type Connection = TixConnection;

/// A tix connection to a single client
#[derive(Debug)]
pub struct TixConnection {
    // Channel to send packets to background writer task
    tx: mpsc::Sender<crate::Packet>,
    // Channel to receive packets from background reader task
    rx: mpsc::Receiver<crate::Packet>,
}

impl TixConnection {

    pub fn new(stream: TcpStream) -> Self {
        let (mut net_writer, mut net_reader) = Framed::new(stream, TixCodec {}).split();

        // User -> Network
        let (user_tx, mut network_rx) = mpsc::channel(100);

        // Network -> User
        let (network_tx, user_rx) = mpsc::channel(100);

        // Writer task: User -> Network
        tokio::spawn(async move {
            while let Some(packet) = network_rx.recv().await {
                if let Err(e) = net_writer.send(packet).await {
                    eprintln!("Network write error: {:?}", e);
                    break;
                }
            }
        });

        // Reader task: Network -> User
        tokio::spawn(async move {
            while let Some(result) = net_reader.next().await {
                match result {
                    Ok(packet) => {
                        if let Err(_) = network_tx.send(packet).await {
                            // user_rx was dropped, stop reading
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Network read error: {:?}", e);
                        break; // Stop on codec/network errors
                    }
                }
            }
        });

        // Heartbeat connection
        let heartbeat_interval = std::time::Duration::from_secs(5);
        let heartbeat_packet = crate::Packet::heartbeat();
        let heartbeat_tx = user_tx.clone(); 
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(heartbeat_interval);
            loop {
                interval.tick().await;
                if let Err(_) = heartbeat_tx.send(heartbeat_packet.clone()).await {
                    break; // Connection handle was dropped, stop heartbeat
                }
            }
        });

        Self {
            tx: user_tx,
            rx: user_rx,
        }
    }

    pub async fn send(&self, packet: crate::Packet) -> Result<(), mpsc::error::SendError<crate::Packet>> {
        self.tx.send(packet).await
    }

    pub async fn recv(&mut self) -> Option<crate::Packet> {
        self.rx.recv().await
    }

    pub async fn get_sender(&self) -> mpsc::Sender<crate::Packet> {
        self.tx.clone()
    }

    pub async fn connect(conn_info: &ConnectionInfo) -> Result<Self, std::io::Error> {
        let stream = TcpStream::connect(conn_info.to_string()).await?;
        let conn = Self::new(stream);
        Ok(conn)
    }
}

pub type TixConnectionSender = mpsc::Sender<crate::Packet>;

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    ip: String,
    port: u16,
}

impl ConnectionInfo {
    pub fn new(ip: String, port: u16) -> Self {
        Self { ip, port }
    }

    pub fn ip(&self) -> &str {
        &self.ip
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn to_string(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }
}
