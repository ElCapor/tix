//! RDP slave service core logic.
//!
//! Manages the lifecycle of the screen-capture pipeline and
//! input-injection loop. Can run in either console or Windows
//! service mode.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::net::{TcpListener, UdpSocket};
use tracing::{error, info, warn};

use tix_core::protocol::screen::{KeyEvent, MouseEvent};
use tix_core::rdp::input::InputInjector;
use tix_core::rdp::service::ScreenService;
use tix_core::rdp::transport::ScreenTransport;

use crate::config::SlaveConfig;

// ── RdpSlaveService ──────────────────────────────────────────────

/// The top-level RDP slave service.
///
/// Owns the screen-capture service and a TCP control listener for
/// accepting master connections, negotiating parameters, and
/// forwarding input events.
pub struct RdpSlaveService {
    config: SlaveConfig,
    running: Arc<AtomicBool>,
}

impl RdpSlaveService {
    /// Create a new slave service with the given config.
    pub fn new(config: SlaveConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Obtain a handle that can be used to stop the service from
    /// another task or the Windows SCM handler.
    pub fn stop_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.running)
    }

    /// Run the service until stopped.
    ///
    /// 1. Binds a TCP listener for control (handshake, input relay).
    /// 2. Waits for a master to connect.
    /// 3. Sets up a UDP socket pair and starts `ScreenService`.
    /// 4. Forwards incoming input events to `InputInjector`.
    /// 5. Shuts down cleanly when `running` becomes `false`.
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.running.store(true, Ordering::SeqCst);

        let control_addr: SocketAddr =
            format!("0.0.0.0:{}", self.config.network.control_port).parse()?;
        let listener = TcpListener::bind(control_addr).await?;
        info!("RDP slave listening on {control_addr}");

        // Accept masters until stopped.
        while self.running.load(Ordering::SeqCst) {
            let accept = tokio::select! {
                result = listener.accept() => result,
                _ = Self::wait_for_stop(&self.running) => break,
            };

            let (stream, peer) = match accept {
                Ok(pair) => pair,
                Err(e) => {
                    warn!("accept error: {e}");
                    continue;
                }
            };

            info!("master connected from {peer}");

            // Negotiate control channel (simplified: read the master's
            // UDP port, respond with our UDP listen port).
            let master_screen_addr = self.negotiate_control(&stream, peer).await;
            let master_screen_addr = match master_screen_addr {
                Ok(addr) => addr,
                Err(e) => {
                    warn!("negotiation failed with {peer}: {e}");
                    continue;
                }
            };

            // Bind UDP for screen data.
            let udp_addr: SocketAddr =
                format!("0.0.0.0:{}", self.config.network.listen_port).parse()?;
            let udp = UdpSocket::bind(udp_addr).await?;
            info!("UDP screen transport on {udp_addr} → {master_screen_addr}");

            let transport = ScreenTransport::new(udp, master_screen_addr);
            let svc_config = self.config.to_service_config();

            let mut screen_svc = match ScreenService::with_config(transport, svc_config) {
                Ok(s) => s,
                Err(e) => {
                    error!("failed to initialise screen service: {e}");
                    continue;
                }
            };

            let svc_running = screen_svc.stop_handle();
            let global_running = Arc::clone(&self.running);

            // Spawn screen capture loop.
            let capture_handle = tokio::spawn(async move {
                if let Err(e) = screen_svc.run().await {
                    error!("screen service error: {e}");
                }
            });

            // Run input forwarding on the TCP control stream until
            // the master disconnects or the service is stopped.
            let injector = InputInjector::new();
            self.forward_input(stream, &injector, &global_running).await;

            svc_running.store(false, Ordering::SeqCst);
            let _ = capture_handle.await;
            info!("session with {peer} ended");
        }

        self.running.store(false, Ordering::SeqCst);
        info!("RDP slave service stopped");
        Ok(())
    }

    /// Signal the service to stop.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Whether the service is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    // ── Internal ─────────────────────────────────────────────────

    /// Simple control-channel negotiation.
    ///
    /// Protocol (all little-endian):
    /// 1. Master sends 2-byte UDP port it is listening on.
    /// 2. Slave responds with 2-byte UDP port it will send to.
    ///
    /// Returns the full `SocketAddr` of the master's screen-receive port.
    async fn negotiate_control(
        &self,
        stream: &tokio::net::TcpStream,
        peer: SocketAddr,
    ) -> Result<SocketAddr, Box<dyn std::error::Error>> {
        let mut buf = [0u8; 2];
        stream.readable().await?;
        let n = stream.try_read(&mut buf)?;
        if n < 2 {
            return Err("master did not send UDP port".into());
        }

        let master_udp_port = u16::from_le_bytes(buf);
        let master_screen_addr = SocketAddr::new(peer.ip(), master_udp_port);

        // Respond with our screen UDP port.
        let our_port = self.config.network.listen_port;
        stream.writable().await?;
        stream.try_write(&our_port.to_le_bytes())?;

        Ok(master_screen_addr)
    }

    /// Read input events from the TCP control stream and inject them.
    ///
    /// Wire format per event (little-endian):
    /// ```text
    /// tag:  u8   (0 = mouse, 1 = keyboard)
    /// data: [u8] (bincode-serialised MouseEvent or KeyEvent)
    /// len:  u16  (length of `data`)
    /// ```
    async fn forward_input(
        &self,
        stream: tokio::net::TcpStream,
        injector: &InputInjector,
        running: &Arc<AtomicBool>,
    ) {
        use tokio::io::AsyncReadExt;

        let mut stream = tokio::io::BufReader::new(stream);
        let mut header = [0u8; 3]; // tag(1) + len(2)

        loop {
            if !running.load(Ordering::SeqCst) {
                break;
            }

            let read = tokio::select! {
                r = stream.read_exact(&mut header) => r,
                _ = Self::wait_for_stop(running) => break,
            };

            match read {
                Ok(_) => {}
                Err(e) => {
                    // Connection closed or error.
                    if e.kind() != std::io::ErrorKind::UnexpectedEof {
                        warn!("control stream error: {e}");
                    }
                    break;
                }
            }

            let tag = header[0];
            let len = u16::from_le_bytes([header[1], header[2]]) as usize;

            let mut payload = vec![0u8; len];
            if let Err(e) = stream.read_exact(&mut payload).await {
                warn!("control stream read error: {e}");
                break;
            }

            match tag {
                0 => {
                    // Mouse event.
                    match bincode::deserialize::<MouseEvent>(&payload) {
                        Ok(ev) => {
                            if let Err(e) = injector.inject_mouse(&ev) {
                                warn!("inject_mouse error: {e}");
                            }
                        }
                        Err(e) => warn!("malformed mouse event: {e}"),
                    }
                }
                1 => {
                    // Keyboard event.
                    match bincode::deserialize::<KeyEvent>(&payload) {
                        Ok(ev) => {
                            if let Err(e) = injector.inject_keyboard(&ev) {
                                warn!("inject_keyboard error: {e}");
                            }
                        }
                        Err(e) => warn!("malformed key event: {e}"),
                    }
                }
                _ => {
                    warn!("unknown input tag: {tag}");
                }
            }
        }
    }

    /// Async helper: resolves when `running` becomes false.
    async fn wait_for_stop(running: &Arc<AtomicBool>) {
        loop {
            if !running.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SlaveConfig;

    #[test]
    fn service_creates_with_defaults() {
        let svc = RdpSlaveService::new(SlaveConfig::default());
        assert!(!svc.is_running());
    }

    #[test]
    fn stop_handle_works() {
        let svc = RdpSlaveService::new(SlaveConfig::default());
        let handle = svc.stop_handle();
        handle.store(true, Ordering::SeqCst);
        assert!(svc.is_running());
        svc.stop();
        assert!(!svc.is_running());
    }
}
