//! TIX Master — network listener and command dispatcher.
//!
//! `TixMaster` accepts a single slave connection, tracks requests via
//! [`MasterState`], and relays events to the TUI through an
//! `mpsc::UnboundedSender<MasterEvent>`.

pub type Master = TixMaster;

use std::time::Duration;

use tix_core::{Command, Connection, ConnectionInfo, MasterState, Packet};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::app::MasterEvent;

/// Default timeout applied to all outbound requests (seconds).
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

/// A tix listener that accepts a single slave connection and manages
/// the request / response lifecycle through [`MasterState`].
#[derive(Debug)]
pub struct TixMaster {
    listener: TcpListener,
    conn: Option<Connection>,
    master_conn_info: Option<ConnectionInfo>,
    slave_conn_info: Option<ConnectionInfo>,
    state: MasterState,
    ui_tx: mpsc::UnboundedSender<MasterEvent>,
    /// Monotonically increasing request ID counter.
    next_req_id: u64,
}

impl TixMaster {
    /// Bind the listener and prepare a new master instance.
    pub async fn listen(
        conn_info: ConnectionInfo,
        ui_tx: mpsc::UnboundedSender<MasterEvent>,
    ) -> Result<Self, std::io::Error> {
        let listener = TcpListener::bind(conn_info.to_socket_string()).await?;

        let mut state = MasterState::new();
        state.set_default_timeout(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS));

        Ok(Self {
            listener,
            conn: None,
            master_conn_info: Some(conn_info),
            slave_conn_info: None,
            state,
            ui_tx,
            next_req_id: 1,
        })
    }

    // ── Connection management ────────────────────────────────────

    /// Accept exactly one incoming connection.
    pub async fn accept_one(&mut self) -> Result<(), std::io::Error> {
        let (stream, _) = self.listener.accept().await?;
        let slave_info = ConnectionInfo::new(
            stream.peer_addr()?.ip().to_string(),
            stream.peer_addr()?.port(),
        );
        self.slave_conn_info = Some(slave_info.clone());
        self.conn = Some(Connection::new(stream));

        // Advance connection phase
        let _ = self.state.phase_mut().begin_connect();
        let _ = self.state.phase_mut().begin_handshake();
        let _ = self.state.phase_mut().complete_handshake();

        let _ = self
            .ui_tx
            .send(MasterEvent::SlaveConnected(format!("{}", slave_info)));
        Ok(())
    }

    /// Read and handle one inbound packet, if available.
    pub async fn process_connection(&mut self) -> Result<(), std::io::Error> {
        let conn = match self.conn.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        match conn.recv().await {
            Some(packet) => {
                let req_id = packet.request_id();
                if req_id > 0 && self.state.is_request_pending(req_id) {
                    match self.process_packet(&packet) {
                        Ok(response) => {
                            self.state.resolve(req_id);
                            let _ = self
                                .ui_tx
                                .send(MasterEvent::Log(format!("- Slave: {}", response)));
                            let _ = self.ui_tx.send(MasterEvent::TaskUpdate {
                                id: req_id,
                                status: "Solved".to_string(),
                            });
                        }
                        Err(e) => {
                            self.state.resolve(req_id);
                            let _ = self
                                .ui_tx
                                .send(MasterEvent::Log(format!("- Slave Error: {}", e)));
                            let _ = self.ui_tx.send(MasterEvent::TaskUpdate {
                                id: req_id,
                                status: "Failed".to_string(),
                            });
                        }
                    }
                }
            }
            None => {
                // Connection dropped — reset state
                self.conn = None;
                self.slave_conn_info = None;
                self.state = MasterState::new();
                self.state
                    .set_default_timeout(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS));
                let _ = self
                    .ui_tx
                    .send(MasterEvent::Log("Slave disconnected".to_string()));
                let _ = self
                    .ui_tx
                    .send(MasterEvent::SlaveConnected("Not Connected".to_string()));
            }
        }
        Ok(())
    }

    /// Drain any requests whose deadline has expired and notify the UI.
    pub fn check_timeouts(&mut self) {
        let expired = self.state.drain_expired();
        for (id, req) in expired {
            let cmd = req.packet.command().ok();
            let _ = self.ui_tx.send(MasterEvent::Log(format!(
                "[TOUT] ReqID {}: {:?} timed out after {:.1}s",
                id,
                cmd,
                req.elapsed().as_secs_f64(),
            )));
            let _ = self.ui_tx.send(MasterEvent::TaskUpdate {
                id,
                status: "Timed out".to_string(),
            });
        }
    }

    // ── Packet interpretation ────────────────────────────────────

    fn process_packet(&self, packet: &Packet) -> Result<String, std::io::Error> {
        let cmd = packet
            .command()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        match cmd {
            Command::Ping => Ok("Pong".to_string()),

            Command::ShellExecute => {
                let output = String::from_utf8_lossy(packet.payload());
                Ok(format!("{}", output))
            }

            Command::Copy => {
                let result_str = String::from_utf8_lossy(packet.payload());
                let _ = self.ui_tx.send(MasterEvent::RefreshTree { is_slave: true });
                Ok(format!("{}", result_str))
            }

            Command::ListDrives => {
                let drives_str = String::from_utf8_lossy(packet.payload()).to_string();
                let _ = self.ui_tx.send(MasterEvent::TreeData {
                    is_slave: true,
                    path: "drives".to_string(),
                    data: drives_str.clone(),
                });
                Ok(format!("Drives: {}", drives_str))
            }

            Command::ListDir => {
                let data_str = String::from_utf8_lossy(packet.payload()).to_string();
                let _ = self.ui_tx.send(MasterEvent::TreeData {
                    is_slave: true,
                    path: "dir_listing".to_string(),
                    data: data_str,
                });
                Ok("Directory listing received".to_string())
            }

            Command::Upload => {
                let _ = self.ui_tx.send(MasterEvent::RefreshTree { is_slave: true });
                Ok("Upload complete".to_string())
            }

            Command::Download => {
                let _ = self
                    .ui_tx
                    .send(MasterEvent::RefreshTree { is_slave: false });
                Ok("Download complete".to_string())
            }

            Command::SystemAction => {
                let msg = String::from_utf8_lossy(packet.payload()).to_string();
                Ok(format!("System action: {}", msg))
            }

            _ => Err(std::io::Error::other(format!(
                "Unhandled command: {:?}",
                cmd
            ))),
        }
    }

    // ── Command dispatch ─────────────────────────────────────────

    /// Parse a text command from the TUI and send the corresponding
    /// packet to the connected slave.
    pub async fn execute_command(&mut self, cmd: String) -> Result<(), std::io::Error> {
        if self.conn.is_none() {
            let _ = self
                .ui_tx
                .send(MasterEvent::Log("Error: No slave connected".to_string()));
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No slave connected",
            ));
        }

        let cmd_trimmed = cmd.trim();
        if cmd_trimmed.is_empty() {
            return Ok(());
        }

        let (tix_cmd, payload) = match Self::parse_command(cmd_trimmed) {
            Ok(pair) => pair,
            Err(msg) => {
                let _ = self.ui_tx.send(MasterEvent::Log(format!("Error: {}", msg)));
                return Err(std::io::Error::other(msg));
            }
        };

        let req_id = self.next_req_id;
        self.next_req_id += 1;

        let _ = self.ui_tx.send(MasterEvent::Log(format!(
            "[SEND] ReqID {}: Sending {:?} to slave...",
            req_id, tix_cmd
        )));

        let packet = Packet::new_command(req_id, tix_cmd, payload)
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        // Track in MasterState before sending
        self.state.track(req_id, packet.clone());

        if let Err(e) = self.conn.as_ref().unwrap().send(packet).await {
            self.state.resolve(req_id);
            let _ = self.ui_tx.send(MasterEvent::Log(format!(
                "[ERR ] ReqID {}: Failed to send packet: {}",
                req_id, e
            )));
            return Err(std::io::Error::other(e.to_string()));
        }

        let _ = self.ui_tx.send(MasterEvent::Log(format!(
            "[SEND] ReqID {}: Packet sent successfully",
            req_id
        )));
        let _ = self.ui_tx.send(MasterEvent::TaskUpdate {
            id: req_id,
            status: "Waiting...".to_string(),
        });
        Ok(())
    }

    /// Parse a user-entered command string into a `(Command, payload)`.
    fn parse_command(input: &str) -> Result<(Command, Vec<u8>), String> {
        if input == "Ping" {
            return Ok((Command::Ping, Vec::new()));
        }

        if let Some(rest) = input.strip_prefix("ShellExecute") {
            let arg = rest.trim_start();
            if arg.is_empty() {
                return Err("ShellExecute requires a command".to_string());
            }
            return Ok((Command::ShellExecute, arg.as_bytes().to_vec()));
        }

        if let Some(rest) = input.strip_prefix("Copy") {
            let arg = rest.trim_start();
            if arg.is_empty() {
                return Err("Copy requires <src> <dest>".to_string());
            }
            return Ok((Command::Copy, arg.as_bytes().to_vec()));
        }

        if input.starts_with("ListDrives") {
            return Ok((Command::ListDrives, Vec::new()));
        }

        if let Some(rest) = input.strip_prefix("ListDir") {
            let path = rest.trim_start();
            let path = if path.is_empty() { "." } else { path };
            return Ok((Command::ListDir, path.as_bytes().to_vec()));
        }

        if let Some(rest) = input.strip_prefix("Upload") {
            let arg = rest.trim_start();
            if arg.is_empty() {
                return Err("Upload requires <local>|<remote>".to_string());
            }
            return Ok((Command::Upload, arg.as_bytes().to_vec()));
        }

        if let Some(rest) = input.strip_prefix("Download") {
            let arg = rest.trim_start();
            if arg.is_empty() {
                return Err("Download requires <remote>|<local>".to_string());
            }
            return Ok((Command::Download, arg.as_bytes().to_vec()));
        }

        if let Some(rest) = input.strip_prefix("SystemAction") {
            let action = rest.trim_start();
            if action.is_empty() {
                return Err("SystemAction requires an action name".to_string());
            }
            return Ok((Command::SystemAction, action.as_bytes().to_vec()));
        }

        Err(format!("Unknown command: '{}'", input))
    }

    // ── Accessors ────────────────────────────────────────────────

    /// Display string for the connected slave.
    pub fn get_client_host_str(&self) -> String {
        self.slave_conn_info
            .as_ref()
            .map(|c| format!("{}", c))
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Display string for the master's own address.
    pub fn get_master_host_str(&self) -> String {
        self.master_conn_info
            .as_ref()
            .map(|c| format!("{}", c))
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Whether a slave is currently connected.
    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    /// Number of in-flight requests awaiting a response.
    pub fn pending_request_count(&self) -> usize {
        self.state.pending_count()
    }
}
