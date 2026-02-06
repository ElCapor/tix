// /*
//  ######  ##          ###    ##     ## ########
// ##    ## ##         ## ##   ##     ## ##      
// ##       ##        ##   ##  ##     ## ##      
//  ######  ##       ##     ## ##     ## ######  
//       ## ##       #########  ##   ##  ##      
// ##    ## ##       ##     ##   ## ##   ##      
//  ######  ######## ##     ##    ###    ########
// */

// use tix_core::TixConnection;
// use tokio::net::{TcpListener, TcpStream};
// use tokio_util::codec::Framed;
// use tokio::io::{AsyncRead, AsyncWrite};
// use futures::{SinkExt, StreamExt};
// use core::marker::Send;
// use tokio::sync::mpsc;


// #[tokio::main]
// async fn main() -> std::io::Result<()> {
//     println!("Starting UP TIX Slave...");

//     let socket = TcpStream::connect("127.0.0.1:4321").await?;
//     println!("Connected to server");

//     let mut connection = TixConnection::new(socket);

//     while let Some(packet) = connection.recv().await {
//         println!("Received packet: {:?}", packet);
//         match packet.get_command() {
//             tix_core::Command::ShellExecute =>  {
//                 let tx : tix_core::TixConnectionSender = connection.get_sender().await;
//                 let req_id = packet.get_request_id();
//                 let payload = packet.get_payload().to_vec();
                
//                 let _ = tokio::spawn(async move {
//                     let payload_str = String::from_utf8_lossy(&payload);
//                     println!("ShellExecute: {:?}", payload_str);

//                     let output = tokio::process::Command::new("cmd")
//                         .arg("/c")
//                         .arg(payload_str.as_ref())
//                         .output().await
//                         .expect("Failed to execute command");
//                     let stdout = String::from_utf8_lossy(&output.stdout);
//                     let stderr = String::from_utf8_lossy(&output.stderr);
//                     let exit_code = output.status.code().unwrap_or(1);
//                     let response = format!("stdout: {}\nstderr: {}\nExit Code: {}", stdout, stderr, exit_code);
//                     let response = response.into_bytes();
//                     if let Ok(response_packet) = tix_core::Packet::new_response(req_id, tix_core::Command::ShellExecute, response) {
//                         let _ = tx.send(response_packet).await;
//                     }
//                 });


//             }

//             _ => {
//                 println!("Unknown command: {:?}", packet.get_command());
//             }
//         }
//     }
    
//     println!("Recieve task finished");
//     Ok(())
    
    
// }

use tix_core::{Connection, ConnectionInfo, TaskEvent, TaskPool, TixConnectionSender};
use std::path::Path;
use fs_extra::dir::CopyOptions;

async fn perform_robust_copy(src: &str, dest: &str) -> Result<String, String> {
    let src_path = Path::new(src);
    let mut dest_path = Path::new(dest).to_path_buf();

    // 1. Validation: Source existence
    if !src_path.exists() {
        return Err(format!("Source path '{}' does not exist", src));
    }

    // 2. Validation: Copying to same location
    if let Ok(abs_src) = std::fs::canonicalize(src_path) {
        if let Ok(abs_dest) = std::fs::canonicalize(&dest_path) {
            if abs_src == abs_dest {
                return Err("Source and destination are the same location".to_string());
            }
        }
    }

    // 3. Determine destination behavior
    if dest_path.is_dir() {
        // If destination is a directory, copy source INTO it
        if let Some(file_name) = src_path.file_name() {
            dest_path.push(file_name);
        }
    }

    // 4. Disk space check (Simplified: check if we can write a small file)
    // In a real scenario, we'd use a crate like `sysinfo` or `fs2` for exact space.
    // Here we'll rely on the actual copy error if it fails due to space.

    // 5. Perform Copy
    if src_path.is_dir() {
        let mut options = CopyOptions::new();
        options.overwrite = true;
        options.copy_inside = true;
        
        match fs_extra::dir::copy(src_path, dest, &options) {
            Ok(_) => Ok(format!("Directory '{}' copied to '{}'", src, dest)),
            Err(e) => Err(format!("Directory copy failed: {}", e)),
        }
    } else {
        match std::fs::copy(src_path, &dest_path) {
            Ok(_) => Ok(format!("File '{}' copied to '{}'", src, dest_path.display())),
            Err(e) => Err(format!("File copy failed: {}", e)),
        }
    }
}

pub type Slave = TixSlave;
pub type SlaveState = TixSlaveState;

pub struct TixSlaveState {
   
}

pub struct TixSlave {
    /// The slave connection
    conn: Connection,
    /// Task pool to handle tasks
    task_pool: TaskPool,
    
}

impl TixSlave {
    pub async fn new(conn_info: &ConnectionInfo) -> Result<Self, std::io::Error> {
        let conn = Connection::connect(conn_info).await?;
        Ok(Self { conn, task_pool: TaskPool::new() })
    }

    pub async fn main_loop(&mut self) -> std::io::Result<()> {
        tokio::select! {
            Some(packet) = self.conn.recv() => {
                self.handle_packet(packet).await?;
            }

            Some(task_event) = self.task_pool.recv() => {
                self.task_pool.process_event(task_event).await;
            }
        }
        Ok(())
    }

    pub async fn handle_packet(&mut self, packet: tix_core::Packet) -> std::io::Result<()>
    {
        let cmd = packet.get_command();
        let req_id = packet.get_request_id();
        println!("[RECV] Command: {:?}, ReqID: {}", cmd, req_id);

        match cmd {
            tix_core::Command::ShellExecute => {
                let tx: TixConnectionSender = self.conn.get_sender().await;
                let payload = packet.get_payload().to_vec();
                let task_pool_tx = self.task_pool.clone_tx();

                println!("[TASK] Spawning ShellExecute task for ReqID: {}", req_id);
                self.task_pool.task_spawn(tx, req_id, payload, |tx, req_id, payload| async move {
                    let payload_str = String::from_utf8_lossy(&payload);
                    println!("[EXEC] ReqID {}: cmd /c \"{}\"", req_id, payload_str);

                    let output = tokio::process::Command::new("cmd")
                        .arg("/c")
                        .arg(payload_str.as_ref())
                        .output().await;

                    match output {
                        Err(e) => {
                            println!("[ERR ] ReqID {} failed to start: {}", req_id, e);
                            let _ = task_pool_tx.send(TaskEvent::Error(req_id, e.to_string())).await;
                        }
                        Ok(output) => {
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            let exit_code = output.status.code().unwrap_or(1);
                            
                            println!("[DONE] ReqID {} finished with code {}", req_id, exit_code);
                            if !stdout.is_empty() { println!("[OUT ] ReqID {}: {}", req_id, stdout.trim()); }
                            if !stderr.is_empty() { println!("[ERR ] ReqID {}: {}", req_id, stderr.trim()); }

                            let response = format!("stdout: {}\nstderr: {}\nExit Code: {}", stdout, stderr, exit_code);
                            let response_bytes = response.into_bytes();
                            
                            if let Ok(response_packet) = tix_core::Packet::new_response(req_id, tix_core::Command::ShellExecute, response_bytes) {
                                if let Err(e) = tx.send(response_packet).await {
                                    println!("[ERR ] ReqID {} failed to send response: {}", req_id, e);
                                } else {
                                    println!("[SEND] ReqID {} response sent to master", req_id);
                                }
                            }
                        }
                    }
                });
            }

            tix_core::Command::Copy => {
                let tx: TixConnectionSender = self.conn.get_sender().await;
                let payload = packet.get_payload().to_vec();
                let task_pool_tx = self.task_pool.clone_tx();

                println!("[TASK] Spawning Copy task for ReqID: {}", req_id);
                self.task_pool.task_spawn(tx, req_id, payload, |tx, req_id, payload| async move {
                    let payload_str = String::from_utf8_lossy(&payload);
                    let args = payload_str.splitn(2, ' ').collect::<Vec<_>>();
                    
                    if args.len() < 2 {
                        let err_msg = "Invalid arguments for Copy. Expected: Copy <src> <dest>".to_string();
                        println!("[ERR ] ReqID {}: {}", req_id, err_msg);
                        let _ = task_pool_tx.send(TaskEvent::Error(req_id, err_msg.clone())).await;
                        if let Ok(response_packet) = tix_core::Packet::new_response(req_id, tix_core::Command::Copy, err_msg.into_bytes()) {
                            let _ = tx.send(response_packet).await;
                        }
                        return;
                    }

                    let src = args[0].trim_matches('"');
                    let dest = args[1].trim_matches('"');
                    
                    println!("[EXEC] ReqID {}: Copying '{}' to '{}'", req_id, src, dest);

                    let result = match perform_robust_copy(src, dest).await {
                        Ok(msg) => {
                            println!("[DONE] ReqID {}: {}", req_id, msg);
                            Ok(msg)
                        }
                        Err(e) => {
                            println!("[ERR ] ReqID {}: {}", req_id, e);
                            Err(e)
                        }
                    };

                    match result {
                        Ok(msg) => {
                            if let Ok(response_packet) = tix_core::Packet::new_response(req_id, tix_core::Command::Copy, msg.into_bytes()) {
                                if let Err(e) = tx.send(response_packet).await {
                                    println!("[ERR ] ReqID {} failed to send response: {}", req_id, e);
                                }
                            }
                        }
                        Err(e) => {
                            if let Ok(response_packet) = tix_core::Packet::new_response(req_id, tix_core::Command::Copy, e.into_bytes()) {
                                let _ = tx.send(response_packet).await;
                            }
                        }
                    }
                });
            }

            tix_core::Command::ListDrives => {
                let tx: TixConnectionSender = self.conn.get_sender().await;
                let _ = tokio::spawn(async move {
                    let mut drives = Vec::new();
                    #[cfg(windows)]
                    {
                        for drive in b'A'..=b'Z' {
                            let drive_str = format!("{}:\\", drive as char);
                            if Path::new(&drive_str).exists() {
                                drives.push(drive_str);
                            }
                        }
                    }
                    #[cfg(not(windows))]
                    {
                        drives.push("/".to_string());
                    }

                    let response = drives.join(",");
                    if let Ok(packet) = tix_core::Packet::new_response(req_id, tix_core::Command::ListDrives, response.into_bytes()) {
                        let _ = tx.send(packet).await;
                    }
                });
            }

            tix_core::Command::ListDir => {
                let tx: TixConnectionSender = self.conn.get_sender().await;
                let payload = packet.get_payload().to_vec();
                let _ = tokio::spawn(async move {
                    let path_str = String::from_utf8_lossy(&payload);
                    let path = Path::new(path_str.as_ref());
                    
                    let mut entries = Vec::new();
                    // First entry is the path itself to help UI identify it
                    entries.push(format!("PATH|{}", path_str));
                    
                    if let Ok(read_dir) = std::fs::read_dir(path) {
                        for entry in read_dir.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            let is_dir = entry.path().is_dir();
                            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                            entries.push(format!("{}|{}|{}", name, if is_dir { "1" } else { "0" }, size));
                        }
                    }

                    let response = entries.join(";");
                    if let Ok(packet) = tix_core::Packet::new_response(req_id, tix_core::Command::ListDir, response.into_bytes()) {
                        let _ = tx.send(packet).await;
                    }
                });
            }

            tix_core::Command::Upload => {
                let tx: TixConnectionSender = self.conn.get_sender().await;
                let payload = packet.get_payload().to_vec();
                let _ = tokio::spawn(async move {
                    let payload_str = String::from_utf8_lossy(&payload);
                    let parts: Vec<&str> = payload_str.split('|').collect();
                    if parts.len() < 2 {
                        let _ = tx.send(tix_core::Packet::new_response(req_id, tix_core::Command::Upload, b"Invalid upload args".to_vec()).unwrap()).await;
                        return;
                    }
                    // For now, just simulate or use copy logic if it's on the same machine
                    let src = parts[0];
                    let dest = parts[1];
                    let result = match perform_robust_copy(src, dest).await {
                        Ok(msg) => format!("Upload successful: {}", msg),
                        Err(e) => format!("Upload failed: {}", e),
                    };
                    if let Ok(response_packet) = tix_core::Packet::new_response(req_id, tix_core::Command::Upload, result.into_bytes()) {
                        let _ = tx.send(response_packet).await;
                    }
                });
            }

            tix_core::Command::Download => {
                let tx: TixConnectionSender = self.conn.get_sender().await;
                let payload = packet.get_payload().to_vec();
                let _ = tokio::spawn(async move {
                    let payload_str = String::from_utf8_lossy(&payload);
                    let parts: Vec<&str> = payload_str.split('|').collect();
                    if parts.len() < 2 {
                        let _ = tx.send(tix_core::Packet::new_response(req_id, tix_core::Command::Download, b"Invalid download args".to_vec()).unwrap()).await;
                        return;
                    }
                    let src = parts[0];
                    let dest = parts[1];
                    let result = match perform_robust_copy(src, dest).await {
                        Ok(msg) => format!("Download successful: {}", msg),
                        Err(e) => format!("Download failed: {}", e),
                    };
                    if let Ok(response_packet) = tix_core::Packet::new_response(req_id, tix_core::Command::Download, result.into_bytes()) {
                        let _ = tx.send(response_packet).await;
                    }
                });
            }

            tix_core::Command::SystemAction => {
                let tx: TixConnectionSender = self.conn.get_sender().await;
                let payload = packet.get_payload().to_vec();
                let _ = tokio::spawn(async move {
                    let action = String::from_utf8_lossy(&payload);
                    let result = match action.as_ref() {
                        "shutdown" => {
                            #[cfg(windows)]
                            {
                                let _ = std::process::Command::new("shutdown").args(&["/s", "/t", "60"]).spawn();
                                "Shutdown initiated in 60s".to_string()
                            }
                            #[cfg(not(windows))]
                            { "Shutdown not supported on this OS".to_string() }
                        }
                        "reboot" => {
                            #[cfg(windows)]
                            {
                                let _ = std::process::Command::new("shutdown").args(&["/r", "/t", "60"]).spawn();
                                "Reboot initiated in 60s".to_string()
                            }
                            #[cfg(not(windows))]
                            { "Reboot not supported on this OS".to_string() }
                        }
                        "sleep" => {
                            #[cfg(windows)]
                            {
                                let _ = std::process::Command::new("rundll32.exe").args(&["powrprof.dll,SetSuspendState", "0,1,0"]).spawn();
                                "Sleep initiated".to_string()
                            }
                            #[cfg(not(windows))]
                            { "Sleep not supported on this OS".to_string() }
                        }
                        _ => format!("Unknown system action: {}", action),
                    };
                    if let Ok(response_packet) = tix_core::Packet::new_response(req_id, tix_core::Command::SystemAction, result.into_bytes()) {
                        let _ = tx.send(response_packet).await;
                    }
                });
            }

            tix_core::Command::Ping => {
                println!("[PING] Received Ping, sending Pong for ReqID: {}", req_id);
                let tx: TixConnectionSender = self.conn.get_sender().await;
                if let Ok(response_packet) = tix_core::Packet::new_response(req_id, tix_core::Command::Ping, b"Pong".to_vec()) {
                    if let Err(e) = tx.send(response_packet).await {
                        println!("[ERR ] ReqID {} failed to send Pong: {}", req_id, e);
                    } else {
                        println!("[SEND] ReqID {} Pong sent", req_id);
                    }
                }
            }

            _ => {
                println!("[WARN] Unknown command: {:?} (ReqID: {})", packet.get_command(), req_id);
            }
        }
        Ok(())
    }
}

#[tokio::main]
pub async fn main() -> std::io::Result<()>
{
    println!("Starting UP TIX Slave...");
    let conn_info = ConnectionInfo::new("127.0.0.1".to_string(), 4321);
    
    println!("[INIT] Connecting to Master at {}...", conn_info.to_string());
    let mut slave = match TixSlave::new(&conn_info).await {
        Ok(s) => {
            println!("[CONN] Successfully connected to Master");
            s
        }
        Err(e) => {
            println!("[FATAL] Failed to connect: {}", e);
            return Err(e);
        }
    };

    loop {
        if let Err(e) = slave.main_loop().await {
            println!("[ERR ] Loop error: {}", e);
            break;
        }
    }

    Ok(())
}