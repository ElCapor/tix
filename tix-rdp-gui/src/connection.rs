//! TCP control connection to the slave.
//!
//! Handles the initial handshake (UDP port exchange), and provides
//! a method to send serialised input events over the control stream.

use std::net::SocketAddr;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tracing::info;

use crate::config::GuiConfig;

/// Manages the TCP control connection to the slave.
pub struct SlaveConnection {
    stream: TcpStream,
    /// The slave's UDP port for screen data.
    slave_screen_port: u16,
    /// The local UDP port we will listen on.
    local_udp_port: u16,
}

impl SlaveConnection {
    /// Connect to the slave, exchange UDP ports.
    ///
    /// `local_udp_port` is the port the GUI client will bind for
    /// receiving screen frames.
    pub async fn connect(
        config: &GuiConfig,
        local_udp_port: u16,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let addr: SocketAddr = config.network.slave_address.parse()?;
        let timeout = std::time::Duration::from_millis(config.network.timeout_ms);

        info!("connecting to slave at {addr}");
        let stream = tokio::time::timeout(timeout, TcpStream::connect(addr)).await??;
        stream.set_nodelay(true)?;

        // Send our UDP port.
        stream.writable().await?;
        stream.try_write(&local_udp_port.to_le_bytes())?;

        // Read slave's UDP port.
        let mut buf = [0u8; 2];
        stream.readable().await?;
        let n = stream.try_read(&mut buf)?;
        if n < 2 {
            return Err("slave did not respond with UDP port".into());
        }
        let slave_screen_port = u16::from_le_bytes(buf);

        info!(
            "negotiated UDP ports: local={local_udp_port}, slave={slave_screen_port}"
        );

        Ok(Self {
            stream,
            slave_screen_port,
            local_udp_port,
        })
    }

    /// The slave's UDP screen-data port.
    pub fn slave_screen_port(&self) -> u16 {
        self.slave_screen_port
    }

    /// The slave's IP + screen port as a full address.
    pub fn slave_screen_addr(&self) -> Result<SocketAddr, Box<dyn std::error::Error>> {
        let peer = self.stream.peer_addr()?;
        Ok(SocketAddr::new(peer.ip(), self.slave_screen_port))
    }

    /// Send a mouse event over the control channel.
    ///
    /// Wire format: tag(1) + len(2) + bincode payload.
    pub async fn send_mouse(
        &mut self,
        event: &tix_core::protocol::screen::MouseEvent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let payload = bincode::serialize(event)?;
        self.send_tagged(0, &payload).await
    }

    /// Send a keyboard event over the control channel.
    pub async fn send_keyboard(
        &mut self,
        event: &tix_core::protocol::screen::KeyEvent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let payload = bincode::serialize(event)?;
        self.send_tagged(1, &payload).await
    }

    /// Low-level tagged write.
    async fn send_tagged(
        &mut self,
        tag: u8,
        data: &[u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let len = data.len() as u16;
        let mut header = [0u8; 3];
        header[0] = tag;
        header[1..3].copy_from_slice(&len.to_le_bytes());

        self.stream.write_all(&header).await?;
        self.stream.write_all(data).await?;
        Ok(())
    }

    /// Consume self and return the underlying TCP stream (for
    /// advanced usage or shutdown).
    pub fn into_stream(self) -> TcpStream {
        self.stream
    }
}
