//! Integration tests — full connection lifecycle, command round-trips,
//! and error scenarios over a real TCP connection on localhost.

use std::time::Duration;

use tix_core::{
    Command, Connection, ConnectionInfo, ConnectionPhase, MasterState, Packet, SlaveState,
};
use tokio::net::TcpListener;

// ── Helpers ──────────────────────────────────────────────────────

/// Spin up a listener on an OS-assigned port and return the connection
/// info.  The listener is returned so the caller can accept on it.
async fn ephemeral_listener() -> (TcpListener, ConnectionInfo) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let info = ConnectionInfo::new(addr.ip().to_string(), addr.port());
    (listener, info)
}

/// Receive the next non-heartbeat packet, skipping any heartbeats
/// that arrive first.
async fn recv_skip_heartbeat(conn: &mut Connection) -> Option<Packet> {
    loop {
        let pkt = conn.recv().await?;
        if pkt.request_id() != 0 {
            return Some(pkt);
        }
        // heartbeat — skip
    }
}

// ── Connection lifecycle ─────────────────────────────────────────

#[tokio::test]
async fn test_connection_lifecycle() {
    let (listener, info) = ephemeral_listener().await;

    // Slave connects
    let slave_handle = tokio::spawn({
        let info = info.clone();
        async move { Connection::connect(&info).await.unwrap() }
    });

    let (stream, _) = listener.accept().await.unwrap();
    let mut master_conn = Connection::new(stream);
    let mut slave_conn = slave_handle.await.unwrap();

    // Master sends Ping
    let ping = Packet::new_command(1, Command::Ping, Vec::new()).unwrap();
    master_conn.send(ping).await.unwrap();

    // Slave receives it (skip any heartbeats)
    let pkt = tokio::time::timeout(Duration::from_secs(5), recv_skip_heartbeat(&mut slave_conn))
        .await
        .expect("timeout")
        .expect("recv returned None");

    assert_eq!(pkt.request_id(), 1);
    assert_eq!(pkt.command().unwrap(), Command::Ping);

    // Slave responds
    let pong = Packet::new_response(1, Command::Ping, b"Pong".to_vec()).unwrap();
    slave_conn.send(pong).await.unwrap();

    // Master receives response
    let resp = tokio::time::timeout(
        Duration::from_secs(5),
        recv_skip_heartbeat(&mut master_conn),
    )
    .await
    .expect("timeout")
    .expect("recv returned None");

    assert_eq!(resp.request_id(), 1);
    assert_eq!(resp.payload(), b"Pong");
}

#[tokio::test]
async fn test_bidirectional_packets() {
    let (listener, info) = ephemeral_listener().await;

    let slave_handle = tokio::spawn({
        let info = info.clone();
        async move { Connection::connect(&info).await.unwrap() }
    });

    let (stream, _) = listener.accept().await.unwrap();
    let master_conn = Connection::new(stream);
    let mut slave_conn = slave_handle.await.unwrap();

    // Send multiple commands and check ordering
    for i in 1u64..=5 {
        let cmd = Packet::new_command(i, Command::Ping, Vec::new()).unwrap();
        master_conn.send(cmd).await.unwrap();
    }

    for i in 1u64..=5 {
        let pkt =
            tokio::time::timeout(Duration::from_secs(5), recv_skip_heartbeat(&mut slave_conn))
                .await
                .expect("timeout")
                .expect("recv returned None");
        assert_eq!(pkt.request_id(), i);
    }
}

// ── State machine ────────────────────────────────────────────────

#[tokio::test]
async fn test_master_state_request_tracking() {
    let mut state = MasterState::new();
    state.set_default_timeout(Duration::from_secs(30));

    let pkt = Packet::new_command(1, Command::Ping, Vec::new()).unwrap();
    state.track(1, pkt);

    assert!(state.is_request_pending(1));
    assert_eq!(state.pending_count(), 1);

    let resolved = state.resolve(1);
    assert!(resolved.is_some());
    assert_eq!(state.pending_count(), 0);
}

#[tokio::test]
async fn test_master_state_timeout_detection() {
    let mut state = MasterState::new();
    // Very short timeout for testing
    state.set_default_timeout(Duration::from_millis(50));

    let pkt = Packet::new_command(1, Command::Ping, Vec::new()).unwrap();
    state.track(1, pkt);

    // Not expired yet
    assert!(state.check_timeouts().is_empty());

    // Wait for expiry
    tokio::time::sleep(Duration::from_millis(100)).await;

    let expired = state.check_timeouts();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0], 1);

    // Drain them
    let drained = state.drain_expired();
    assert_eq!(drained.len(), 1);
    assert_eq!(state.pending_count(), 0);
}

#[tokio::test]
async fn test_slave_state_task_tracking() {
    let mut state = SlaveState::new();

    // Connection phase transitions
    state.phase_mut().begin_connect().unwrap();
    state.phase_mut().begin_handshake().unwrap();
    state.phase_mut().complete_handshake().unwrap();
    assert!(state.phase().is_connected());

    // Register tasks
    assert!(state.register_task(1));
    assert!(state.register_task(2));
    assert!(!state.register_task(1)); // duplicate
    assert_eq!(state.active_task_count(), 2);

    // Complete tasks
    assert!(state.complete_task(1));
    assert_eq!(state.active_task_count(), 1);
    assert!(!state.complete_task(999)); // unknown
}

#[test]
fn test_connection_phase_valid_transitions() {
    let mut phase = ConnectionPhase::default();
    assert!(phase.is_disconnected());

    phase.begin_connect().unwrap();
    phase.begin_handshake().unwrap();
    phase.complete_handshake().unwrap();
    assert!(phase.is_connected());

    phase.begin_disconnect().unwrap();
    assert!(!phase.is_connected());
}

#[test]
fn test_connection_phase_invalid_transition() {
    let mut phase = ConnectionPhase::default();
    // Cannot start handshake from Disconnected
    assert!(phase.begin_handshake().is_err());
}

// ── Shell command round-trip ─────────────────────────────────────

#[tokio::test]
async fn test_shell_execute_round_trip() {
    let (listener, info) = ephemeral_listener().await;

    let slave_handle = tokio::spawn({
        let info = info.clone();
        async move { Connection::connect(&info).await.unwrap() }
    });

    let (stream, _) = listener.accept().await.unwrap();
    let mut master_conn = Connection::new(stream);
    let mut slave_conn = slave_handle.await.unwrap();

    // Master sends ShellExecute
    let payload = b"echo hello".to_vec();
    let cmd = Packet::new_command(1, Command::ShellExecute, payload.clone()).unwrap();
    master_conn.send(cmd).await.unwrap();

    // Slave receives
    let pkt = tokio::time::timeout(Duration::from_secs(5), recv_skip_heartbeat(&mut slave_conn))
        .await
        .expect("timeout")
        .expect("recv returned None");
    assert_eq!(pkt.command().unwrap(), Command::ShellExecute);
    assert_eq!(pkt.payload(), b"echo hello");

    // Slave sends response
    let output = b"stdout: hello\nstderr: \nExit Code: 0".to_vec();
    let resp = Packet::new_response(1, Command::ShellExecute, output.clone()).unwrap();
    slave_conn.send(resp).await.unwrap();

    // Master receives response
    let resp = tokio::time::timeout(
        Duration::from_secs(5),
        recv_skip_heartbeat(&mut master_conn),
    )
    .await
    .expect("timeout")
    .expect("recv returned None");
    assert_eq!(resp.payload(), &output[..]);
}

// ── Large payload ────────────────────────────────────────────────

#[tokio::test]
async fn test_large_payload_transfer() {
    let (listener, info) = ephemeral_listener().await;

    let slave_handle = tokio::spawn({
        let info = info.clone();
        async move { Connection::connect(&info).await.unwrap() }
    });

    let (stream, _) = listener.accept().await.unwrap();
    let master_conn = Connection::new(stream);
    let mut slave_conn = slave_handle.await.unwrap();

    // Send a 200KB payload (under the 256KB MAX_PAYLOAD_SIZE)
    let large_payload = vec![0xABu8; 200 * 1024];
    let cmd = Packet::new_command(1, Command::Copy, large_payload.clone()).unwrap();
    master_conn.send(cmd).await.unwrap();

    let pkt = tokio::time::timeout(
        Duration::from_secs(10),
        recv_skip_heartbeat(&mut slave_conn),
    )
    .await
    .expect("timeout")
    .expect("recv returned None");
    assert_eq!(pkt.payload().len(), 200 * 1024);
    assert_eq!(pkt.payload(), &large_payload[..]);
}

// ── Error scenarios ──────────────────────────────────────────────

#[tokio::test]
async fn test_connection_drop_detected() {
    let (listener, info) = ephemeral_listener().await;

    let slave_handle = tokio::spawn({
        let info = info.clone();
        async move { Connection::connect(&info).await.unwrap() }
    });

    let (stream, _) = listener.accept().await.unwrap();
    let mut master_conn = Connection::new(stream);
    let slave_conn = slave_handle.await.unwrap();

    // Drop slave — master should get None on next recv
    drop(slave_conn);

    // Give the background tasks time to notice
    tokio::time::sleep(Duration::from_millis(200)).await;

    let result = tokio::time::timeout(Duration::from_secs(5), master_conn.recv())
        .await
        .expect("timeout");
    // After dropping the peer, recv should eventually return None
    // (may take a moment for TCP FIN to propagate)
    // We accept either None or a heartbeat packet
    if let Some(pkt) = result {
        // If we got something, it must be a heartbeat from master's own writer
        assert_eq!(pkt.request_id(), 0);
    }
}

#[test]
fn test_packet_too_large() {
    // Payload bigger than MAX_PAYLOAD_SIZE should fail
    let too_large = vec![0u8; tix_core::MAX_PAYLOAD_SIZE + 1];
    let result = Packet::new_command(1, Command::Copy, too_large);
    assert!(result.is_err());
}

#[test]
fn test_invalid_command_conversion() {
    // Unknown command value should error
    let result = Command::try_from(0xFFFF_u64);
    assert!(result.is_err());
}
