//! TIX Slave — connects to a master and executes commands.
//!
//! Handles shell execution, file operations, directory listing,
//! system actions, and more. Automatically reconnects on disconnect
//! with exponential backoff.

use fs_extra::dir::CopyOptions;
use std::path::Path;
use std::time::Duration;
use tix_core::{
    Command, Connection, ConnectionInfo, ConnectionSender, SlaveState, TaskError, TaskEvent,
    TaskPool,
};

// ── Constants ────────────────────────────────────────────────────

/// Base delay between reconnection attempts.
const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(1);
/// Maximum delay between reconnection attempts.
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);
/// Maximum number of consecutive reconnection attempts before giving up.
const MAX_RECONNECT_ATTEMPTS: u32 = 50;

// ── Helpers ──────────────────────────────────────────────────────

/// Copy a file or directory robustly, with validation.
async fn perform_robust_copy(src: &str, dest: &str) -> Result<String, String> {
    let src_path = Path::new(src);
    let mut dest_path = Path::new(dest).to_path_buf();

    if !src_path.exists() {
        return Err(format!("Source path '{}' does not exist", src));
    }

    if let Ok(abs_src) = std::fs::canonicalize(src_path)
        && let Ok(abs_dest) = std::fs::canonicalize(&dest_path)
        && abs_src == abs_dest
    {
        return Err("Source and destination are the same location".to_string());
    }

    if dest_path.is_dir()
        && let Some(file_name) = src_path.file_name()
    {
        dest_path.push(file_name);
    }

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
            Ok(_) => Ok(format!(
                "File '{}' copied to '{}'",
                src,
                dest_path.display()
            )),
            Err(e) => Err(format!("File copy failed: {}", e)),
        }
    }
}

// ── TixSlave ─────────────────────────────────────────────────────

pub struct TixSlave {
    /// The slave connection.
    conn: Connection,
    /// State tracking (phase, capabilities, active tasks).
    state: SlaveState,
    /// Task pool for spawning concurrent work.
    task_pool: TaskPool,
}

impl TixSlave {
    /// Connect to the master at the given address.
    pub async fn connect(conn_info: &ConnectionInfo) -> Result<Self, std::io::Error> {
        let conn = Connection::connect(conn_info).await?;
        let mut state = SlaveState::new();
        // Advance through the connection phases
        let _ = state.phase_mut().begin_connect();
        let _ = state.phase_mut().begin_handshake();
        let _ = state.phase_mut().complete_handshake();
        Ok(Self {
            conn,
            state,
            task_pool: TaskPool::new(),
        })
    }

    /// Run the main loop: handle packets and task events.
    pub async fn run(&mut self) -> std::io::Result<()> {
        loop {
            tokio::select! {
                packet = self.conn.recv() => {
                    match packet {
                        Some(pkt) => self.handle_packet(pkt).await?,
                        None => {
                            println!("[DISC] Connection to master lost");
                            return Ok(());
                        }
                    }
                }

                Some(task_event) = self.task_pool.recv() => {
                    self.task_pool.process_event(task_event).await;
                }
            }
        }
    }

    /// Dispatch a received packet to the appropriate handler.
    async fn handle_packet(&mut self, packet: tix_core::Packet) -> std::io::Result<()> {
        let cmd = packet
            .command()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let req_id = packet.request_id();
        println!("[RECV] Command: {:?}, ReqID: {}", cmd, req_id);

        // Register the task in SlaveState
        self.state.register_task(req_id);

        match cmd {
            Command::ShellExecute => {
                self.handle_shell_execute(req_id, packet.payload());
                Ok(())
            }
            Command::Copy => {
                self.handle_copy(req_id, packet.payload());
                Ok(())
            }
            Command::ListDrives => {
                self.handle_list_drives(req_id);
                Ok(())
            }
            Command::ListDir => {
                self.handle_list_dir(req_id, packet.payload());
                Ok(())
            }
            Command::Upload => {
                self.handle_upload(req_id, packet.payload());
                Ok(())
            }
            Command::Download => {
                self.handle_download(req_id, packet.payload());
                Ok(())
            }
            Command::SystemAction => {
                self.handle_system_action(req_id, packet.payload());
                Ok(())
            }
            Command::Ping => self.handle_ping(req_id).await,
            _ => {
                println!("[WARN] Unknown command: {:?} (ReqID: {})", cmd, req_id);
                self.state.complete_task(req_id);
                Ok(())
            }
        }
    }

    // ── Command handlers ─────────────────────────────────────────

    fn handle_shell_execute(&mut self, req_id: u64, payload: &[u8]) {
        let tx: ConnectionSender = self.conn.sender();
        let payload = payload.to_vec();
        let task_pool_tx = self.task_pool.event_sender();

        println!("[TASK] Spawning ShellExecute task for ReqID: {}", req_id);
        self.task_pool
            .spawn(tx, req_id, payload, |tx, req_id, payload| async move {
                let payload_str = String::from_utf8_lossy(&payload);
                println!("[EXEC] ReqID {}: cmd /c \"{}\"", req_id, payload_str);

                let output = tokio::process::Command::new("cmd")
                    .arg("/c")
                    .arg(payload_str.as_ref())
                    .output()
                    .await;

                match output {
                    Err(e) => {
                        println!("[ERR ] ReqID {} failed to start: {}", req_id, e);
                        let _ = task_pool_tx
                            .send(TaskEvent::Error(req_id, TaskError::Failed(e.to_string())))
                            .await;
                    }
                    Ok(output) => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let exit_code = output.status.code().unwrap_or(1);

                        println!("[DONE] ReqID {} finished with code {}", req_id, exit_code);
                        if !stdout.is_empty() {
                            println!("[OUT ] ReqID {}: {}", req_id, stdout.trim());
                        }
                        if !stderr.is_empty() {
                            println!("[ERR ] ReqID {}: {}", req_id, stderr.trim());
                        }

                        let response = format!(
                            "stdout: {}\nstderr: {}\nExit Code: {}",
                            stdout, stderr, exit_code
                        );
                        if let Ok(pkt) = tix_core::Packet::new_response(
                            req_id,
                            Command::ShellExecute,
                            response.into_bytes(),
                        ) && let Err(e) = tx.send(pkt).await
                        {
                            println!("[ERR ] ReqID {} failed to send response: {}", req_id, e);
                        }
                    }
                }
            });
    }

    fn handle_copy(&mut self, req_id: u64, payload: &[u8]) {
        let tx: ConnectionSender = self.conn.sender();
        let payload = payload.to_vec();
        let task_pool_tx = self.task_pool.event_sender();

        println!("[TASK] Spawning Copy task for ReqID: {}", req_id);
        self.task_pool
            .spawn(tx, req_id, payload, |tx, req_id, payload| async move {
                let payload_str = String::from_utf8_lossy(&payload);
                let args: Vec<&str> = payload_str.splitn(2, ' ').collect();

                if args.len() < 2 {
                    let err_msg =
                        "Invalid arguments for Copy. Expected: Copy <src> <dest>".to_string();
                    println!("[ERR ] ReqID {}: {}", req_id, err_msg);
                    let _ = task_pool_tx
                        .send(TaskEvent::Error(req_id, TaskError::Failed(err_msg.clone())))
                        .await;
                    if let Ok(pkt) =
                        tix_core::Packet::new_response(req_id, Command::Copy, err_msg.into_bytes())
                    {
                        let _ = tx.send(pkt).await;
                    }
                    return;
                }

                let src = args[0].trim_matches('"');
                let dest = args[1].trim_matches('"');
                println!("[EXEC] ReqID {}: Copying '{}' to '{}'", req_id, src, dest);

                let result = perform_robust_copy(src, dest).await;
                let msg = match &result {
                    Ok(m) => {
                        println!("[DONE] ReqID {}: {}", req_id, m);
                        m.clone()
                    }
                    Err(e) => {
                        println!("[ERR ] ReqID {}: {}", req_id, e);
                        e.clone()
                    }
                };

                if let Ok(pkt) =
                    tix_core::Packet::new_response(req_id, Command::Copy, msg.into_bytes())
                {
                    let _ = tx.send(pkt).await;
                }
            });
    }

    fn handle_list_drives(&self, req_id: u64) {
        let tx: ConnectionSender = self.conn.sender();
        tokio::spawn(async move {
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
            if let Ok(pkt) =
                tix_core::Packet::new_response(req_id, Command::ListDrives, response.into_bytes())
            {
                let _ = tx.send(pkt).await;
            }
        });
    }

    fn handle_list_dir(&self, req_id: u64, payload: &[u8]) {
        let tx: ConnectionSender = self.conn.sender();
        let payload = payload.to_vec();
        tokio::spawn(async move {
            let path_str = String::from_utf8_lossy(&payload);
            let path = Path::new(path_str.as_ref());

            let mut entries = Vec::new();
            entries.push(format!("PATH|{}", path_str));

            if let Ok(read_dir) = std::fs::read_dir(path) {
                for entry in read_dir.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.path().is_dir();
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    entries.push(format!(
                        "{}|{}|{}",
                        name,
                        if is_dir { "1" } else { "0" },
                        size
                    ));
                }
            }

            let response = entries.join(";");
            if let Ok(pkt) =
                tix_core::Packet::new_response(req_id, Command::ListDir, response.into_bytes())
            {
                let _ = tx.send(pkt).await;
            }
        });
    }

    fn handle_upload(&self, req_id: u64, payload: &[u8]) {
        let tx: ConnectionSender = self.conn.sender();
        let payload = payload.to_vec();
        tokio::spawn(async move {
            let payload_str = String::from_utf8_lossy(&payload);
            let parts: Vec<&str> = payload_str.split('|').collect();
            if parts.len() < 2 {
                if let Ok(pkt) = tix_core::Packet::new_response(
                    req_id,
                    Command::Upload,
                    b"Invalid upload args".to_vec(),
                ) {
                    let _ = tx.send(pkt).await;
                }
                return;
            }
            let result = match perform_robust_copy(parts[0], parts[1]).await {
                Ok(msg) => format!("Upload successful: {}", msg),
                Err(e) => format!("Upload failed: {}", e),
            };
            if let Ok(pkt) =
                tix_core::Packet::new_response(req_id, Command::Upload, result.into_bytes())
            {
                let _ = tx.send(pkt).await;
            }
        });
    }

    fn handle_download(&self, req_id: u64, payload: &[u8]) {
        let tx: ConnectionSender = self.conn.sender();
        let payload = payload.to_vec();
        tokio::spawn(async move {
            let payload_str = String::from_utf8_lossy(&payload);
            let parts: Vec<&str> = payload_str.split('|').collect();
            if parts.len() < 2 {
                if let Ok(pkt) = tix_core::Packet::new_response(
                    req_id,
                    Command::Download,
                    b"Invalid download args".to_vec(),
                ) {
                    let _ = tx.send(pkt).await;
                }
                return;
            }
            let result = match perform_robust_copy(parts[0], parts[1]).await {
                Ok(msg) => format!("Download successful: {}", msg),
                Err(e) => format!("Download failed: {}", e),
            };
            if let Ok(pkt) =
                tix_core::Packet::new_response(req_id, Command::Download, result.into_bytes())
            {
                let _ = tx.send(pkt).await;
            }
        });
    }

    fn handle_system_action(&self, req_id: u64, payload: &[u8]) {
        let tx: ConnectionSender = self.conn.sender();
        let payload = payload.to_vec();
        tokio::spawn(async move {
            let action = String::from_utf8_lossy(&payload);
            let result = match action.as_ref() {
                "shutdown" => {
                    #[cfg(windows)]
                    {
                        let _ = std::process::Command::new("shutdown")
                            .args(["/s", "/t", "60"])
                            .spawn();
                        "Shutdown initiated in 60s".to_string()
                    }
                    #[cfg(not(windows))]
                    {
                        "Shutdown not supported on this OS".to_string()
                    }
                }
                "reboot" => {
                    #[cfg(windows)]
                    {
                        let _ = std::process::Command::new("shutdown")
                            .args(["/r", "/t", "60"])
                            .spawn();
                        "Reboot initiated in 60s".to_string()
                    }
                    #[cfg(not(windows))]
                    {
                        "Reboot not supported on this OS".to_string()
                    }
                }
                "sleep" => {
                    #[cfg(windows)]
                    {
                        let _ = std::process::Command::new("rundll32.exe")
                            .args(["powrprof.dll,SetSuspendState", "0,1,0"])
                            .spawn();
                        "Sleep initiated".to_string()
                    }
                    #[cfg(not(windows))]
                    {
                        "Sleep not supported on this OS".to_string()
                    }
                }
                _ => format!("Unknown system action: {}", action),
            };
            if let Ok(pkt) =
                tix_core::Packet::new_response(req_id, Command::SystemAction, result.into_bytes())
            {
                let _ = tx.send(pkt).await;
            }
        });
    }

    async fn handle_ping(&mut self, req_id: u64) -> std::io::Result<()> {
        println!("[PING] Received Ping, sending Pong for ReqID: {}", req_id);
        let tx: ConnectionSender = self.conn.sender();
        if let Ok(pkt) = tix_core::Packet::new_response(req_id, Command::Ping, b"Pong".to_vec()) {
            if let Err(e) = tx.send(pkt).await {
                println!("[ERR ] ReqID {} failed to send Pong: {}", req_id, e);
            } else {
                println!("[SEND] ReqID {} Pong sent", req_id);
            }
        }
        self.state.complete_task(req_id);
        Ok(())
    }
}

// ── Reconnection loop ────────────────────────────────────────────

/// Connect to the master with exponential backoff, then run the main
/// loop.  On disconnect, reconnect automatically until
/// `MAX_RECONNECT_ATTEMPTS` consecutive failures.
async fn run_with_reconnect(conn_info: &ConnectionInfo) -> std::io::Result<()> {
    let mut consecutive_failures: u32 = 0;

    loop {
        println!("[INIT] Connecting to Master at {}...", conn_info);

        match TixSlave::connect(conn_info).await {
            Ok(mut slave) => {
                println!("[CONN] Successfully connected to Master");
                consecutive_failures = 0;

                if let Err(e) = slave.run().await {
                    println!("[ERR ] Connection loop error: {}", e);
                }
                // run() returned — connection was lost
            }
            Err(e) => {
                consecutive_failures += 1;
                println!(
                    "[FAIL] Connection attempt {}/{} failed: {}",
                    consecutive_failures, MAX_RECONNECT_ATTEMPTS, e
                );

                if consecutive_failures >= MAX_RECONNECT_ATTEMPTS {
                    println!("[FATAL] Max reconnection attempts reached — exiting");
                    return Err(e);
                }
            }
        }

        // Exponential backoff with cap
        let backoff = std::cmp::min(
            RECONNECT_BASE_DELAY * 2u32.saturating_pow(consecutive_failures.min(5)),
            RECONNECT_MAX_DELAY,
        );
        println!("[WAIT] Reconnecting in {:.1}s...", backoff.as_secs_f64());
        tokio::time::sleep(backoff).await;
    }
}

// ── Entry point ──────────────────────────────────────────────────

#[tokio::main]
pub async fn main() -> std::io::Result<()> {
    println!("Starting UP TIX Slave...");
    let conn_info = ConnectionInfo::new("127.0.0.1".to_string(), 4321);
    run_with_reconnect(&conn_info).await
}
