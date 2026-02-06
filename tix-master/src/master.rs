pub type Master = TixMaster;

use std::collections::HashMap;

use tix_core::{Connection, ConnectionInfo, Packet};
use tokio::net::TcpListener;


#[derive(Debug)]
pub enum RequestState {
    WaitingForResponse,
    Solved,
    Failed,
}

#[derive(Debug)]
pub struct RequestManager {
    // Basically when server sends a request he saves it's state here
    requests: HashMap<u64, RequestState>,
    /// Next request id to use
    next_req_id: u64,
}

impl RequestManager {
    pub fn new() -> Self {
        Self { requests: HashMap::new(), next_req_id: 1 }
    }

    pub fn add_request(&mut self, state: RequestState) -> u64 {
        let req_id = self.next_req_id;
        self.next_req_id += 1;
        self.requests.insert(req_id, state);
        req_id
    }

    pub fn get_request(&self, req_id: u64) -> Option<&RequestState> {
        self.requests.get(&req_id)
    }

    pub fn remove_request(&mut self, req_id: u64) -> Option<RequestState> {
        self.requests.remove(&req_id)
    }

    pub fn solve_request(&mut self, req_id: u64) -> bool {
        if let Some(state) = self.requests.get_mut(&req_id) {
            if let RequestState::WaitingForResponse = state {
                *state = RequestState::Solved;
                return true;
            }
        }
        false
    }

    pub fn fail_request(&mut self, req_id: u64) -> bool {
        if let Some(state) = self.requests.get_mut(&req_id) {
            if let RequestState::WaitingForResponse = state {
                *state = RequestState::Failed;
                return true;
            }
        }
        false
    }

    pub fn get_last_5_requests(&self) -> Vec<(u64, &RequestState)> {
        let mut reqs: Vec<_> = self.requests.iter().map(|(k, v)| (*k, v)).collect();
        reqs.sort_by(|a, b| b.0.cmp(&a.0));
        reqs.truncate(5);
        reqs
    }
}



use crate::app::MasterEvent;
use tokio::sync::mpsc;

/// A tix listener to accept only a single connection
#[derive(Debug)]
pub struct TixMaster {
    listener: TcpListener,
    conn: Option<Connection>,
    master_conn_info: Option<ConnectionInfo>,
    slave_conn_info: Option<ConnectionInfo>,
    req_manager: RequestManager,
    ui_tx: mpsc::UnboundedSender<MasterEvent>,
}

impl TixMaster {
    pub async fn listen(conn_info: ConnectionInfo, ui_tx: mpsc::UnboundedSender<MasterEvent>) -> Result<Self, std::io::Error> {
        let listener = TcpListener::bind(conn_info.to_string()).await?;
        Ok(Self { 
            listener, 
            conn: None, 
            master_conn_info: Some(conn_info), 
            slave_conn_info: None, 
            req_manager: RequestManager::new(),
            ui_tx,
        })
    }

    pub async fn accept_one(&mut self) -> Result<(), std::io::Error> {
        let (conn, _) = self.listener.accept().await?;
        let slave_info = ConnectionInfo::new(conn.peer_addr()?.ip().to_string(), conn.peer_addr()?.port());
        self.slave_conn_info = Some(slave_info.clone());
        self.conn = Some(Connection::new(conn));
        let _ = self.ui_tx.send(MasterEvent::SlaveConnected(slave_info.to_string()));
        Ok(())
    }

    pub async fn process_connection(&mut self) -> Result<(), std::io::Error> {
        if let Some(conn) = self.conn.as_mut() {
            match conn.recv().await {
                Some(packet) => {
                    let req_id = packet.get_request_id();
                    if req_id > 0 {
                        if let Some(req_state) = self.req_manager.get_request(req_id) {
                            match req_state {
                                RequestState::WaitingForResponse => {
                                    match self.process_packet(packet) {
                                        Ok(response) => {
                                            self.req_manager.solve_request(req_id);
                                            let _ = self.ui_tx.send(MasterEvent::Log(format!("- Slave: {}", response)));
                                            let _ = self.ui_tx.send(MasterEvent::TaskUpdate { id: req_id, status: "Solved".to_string() });
                                        }
                                        Err(e) => {
                                            let err_msg = e.to_string();
                                            self.req_manager.fail_request(req_id);
                                            let _ = self.ui_tx.send(MasterEvent::Log(format!("- Slave Error: {}", err_msg)));
                                            let _ = self.ui_tx.send(MasterEvent::TaskUpdate { id: req_id, status: "Failed".to_string() });
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                None => {
                    // Connection closed
                    self.conn = None;
                    self.slave_conn_info = None;
                    let _ = self.ui_tx.send(MasterEvent::Log("Slave disconnected".to_string()));
                    let _ = self.ui_tx.send(MasterEvent::SlaveConnected("Not Connected".to_string()));
                }
            }
        }
        Ok(())
    }

    pub fn process_packet(&mut self, packet: Packet) -> Result<String, std::io::Error> {
        match packet.get_command() {
            tix_core::Command::Ping => {
                Ok("Pong".to_string())
            }
            tix_core::Command::ShellExecute => {
                let cmd = packet.get_payload().to_vec();
                let cmd_str = String::from_utf8_lossy(cmd.as_slice());
                Ok(format!("Executed: {}", cmd_str))
            }
            tix_core::Command::Copy => {
                let payload = packet.get_payload().to_vec();
                let result_str = String::from_utf8_lossy(payload.as_slice());
                let _ = self.ui_tx.send(MasterEvent::RefreshTree { is_slave: true });
                Ok(format!("{}", result_str))
            }
            tix_core::Command::ListDrives => {
                let payload = packet.get_payload().to_vec();
                let drives_str = String::from_utf8_lossy(payload.as_slice()).to_string();
                let _ = self.ui_tx.send(MasterEvent::TreeData { 
                    is_slave: true, 
                    path: "drives".to_string(), 
                    data: drives_str.clone() 
                });
                Ok(format!("Drives: {}", drives_str))
            }
            tix_core::Command::ListDir => {
                let payload = packet.get_payload().to_vec();
                let data_str = String::from_utf8_lossy(payload.as_slice()).to_string();
                // We need to know which path this was for. For now, let's assume the UI knows.
                // Or we could have included the path in the response if we had a more complex protocol.
                let _ = self.ui_tx.send(MasterEvent::TreeData { 
                    is_slave: true, 
                    path: "dir_listing".to_string(), 
                    data: data_str 
                });
                Ok("Directory listing received".to_string())
            }
            tix_core::Command::Upload => {
                let _ = self.ui_tx.send(MasterEvent::RefreshTree { is_slave: true });
                Ok("Upload complete".to_string())
            }
            tix_core::Command::Download => {
                let _ = self.ui_tx.send(MasterEvent::RefreshTree { is_slave: false });
                Ok("Download complete".to_string())
            }
            tix_core::Command::SystemAction => {
                let payload = packet.get_payload().to_vec();
                let msg = String::from_utf8_lossy(payload.as_slice()).to_string();
                Ok(format!("System action: {}", msg))
            }
            _ => {
                Err(std::io::Error::new(std::io::ErrorKind::Other, "Unknown command"))
            }
        }
    }

    pub async fn execute_command(&mut self, cmd: String) -> Result<(), std::io::Error> {
        if self.conn.is_none() {
            let _ = self.ui_tx.send(MasterEvent::Log("Error: No slave connected".to_string()));
            return Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "No slave connected"));
        }

        let cmd_trimmed = cmd.trim();
        if cmd_trimmed.is_empty() {
            return Ok(());
        }

        let (tix_cmd, payload) = if cmd_trimmed == "Ping" {
            (tix_core::Command::Ping, Vec::new())
        } else if cmd_trimmed.starts_with("ShellExecute") {
            let payload_str = if cmd_trimmed.len() > 12 && cmd_trimmed.as_bytes()[12] == b' ' {
                &cmd_trimmed[13..]
            } else if cmd_trimmed.len() == 12 {
                ""
            } else {
                let _ = self.ui_tx.send(MasterEvent::Log(format!("Error: Invalid command '{}'", cmd)));
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "Invalid command"));
            };
            
            if payload_str.is_empty() {
                let _ = self.ui_tx.send(MasterEvent::Log("Error: ShellExecute requires a command".to_string()));
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "ShellExecute requires a command"));
            }
            (tix_core::Command::ShellExecute, payload_str.as_bytes().to_vec())
        } else if cmd_trimmed.starts_with("Copy") {
            let payload_str = if cmd_trimmed.len() > 4 && cmd_trimmed.as_bytes()[4] == b' ' {
                &cmd_trimmed[5..]
            } else {
                let _ = self.ui_tx.send(MasterEvent::Log("Error: Copy requires <src> <dest>".to_string()));
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "Copy requires arguments"));
            };
            
            if payload_str.is_empty() {
                let _ = self.ui_tx.send(MasterEvent::Log("Error: Copy requires <src> <dest>".to_string()));
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "Copy requires arguments"));
            }
            (tix_core::Command::Copy, payload_str.as_bytes().to_vec())
        } else if cmd_trimmed.starts_with("ListDrives") {
            (tix_core::Command::ListDrives, Vec::new())
        } else if cmd_trimmed.starts_with("ListDir") {
            let path = if cmd_trimmed.len() > 7 && cmd_trimmed.as_bytes()[7] == b' ' {
                &cmd_trimmed[8..]
            } else {
                "."
            };
            (tix_core::Command::ListDir, path.as_bytes().to_vec())
        } else if cmd_trimmed.starts_with("Upload") {
            let payload_str = &cmd_trimmed[7..];
            (tix_core::Command::Upload, payload_str.as_bytes().to_vec())
        } else if cmd_trimmed.starts_with("Download") {
            let payload_str = &cmd_trimmed[9..];
            (tix_core::Command::Download, payload_str.as_bytes().to_vec())
        } else if cmd_trimmed.starts_with("SystemAction") {
            let action = &cmd_trimmed[13..];
            (tix_core::Command::SystemAction, action.as_bytes().to_vec())
        } else {
            let _ = self.ui_tx.send(MasterEvent::Log(format!("Error: Invalid command '{}'", cmd)));
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Invalid command"));
        };

        let req_id = self.req_manager.add_request(RequestState::WaitingForResponse);
        let _ = self.ui_tx.send(MasterEvent::Log(format!("[SEND] ReqID {}: Sending command {:?} to slave...", req_id, tix_cmd)));
        
        if let Ok(packet) = Packet::new_command(req_id, tix_cmd, payload) {
            if let Err(e) = self.conn.as_mut().unwrap().send(packet).await {
                let _ = self.ui_tx.send(MasterEvent::Log(format!("[ERR ] ReqID {}: Failed to send packet: {}", req_id, e)));
                self.req_manager.fail_request(req_id);
                return Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
            }
            let _ = self.ui_tx.send(MasterEvent::Log(format!("[SEND] ReqID {}: Packet sent successfully", req_id)));
            let _ = self.ui_tx.send(MasterEvent::TaskUpdate { id: req_id, status: "Waiting...".to_string() });
        }
        Ok(())
    }

    pub fn get_client_host_str(&self) -> String {
        if let Some(conn_info) = self.slave_conn_info.as_ref() {
            conn_info.to_string().clone()
        } else {
            "Unknown".to_string()
        }
    }

    pub fn get_master_host_str(&self) -> String {
        if let Some(conn_info) = self.master_conn_info.as_ref() {
            conn_info.to_string().clone()
        } else {
            "Unknown".to_string()
        }
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    pub fn get_master_conn_info(&self) -> Option<&ConnectionInfo> {
        self.master_conn_info.as_ref()
    }

    pub fn get_slave_conn_info(&self) -> Option<&ConnectionInfo> {
        self.slave_conn_info.as_ref()
    }
}