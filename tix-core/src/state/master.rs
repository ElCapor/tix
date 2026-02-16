//! Master-side state tracking.
//!
//! Tracks the connection phase, negotiated capabilities, and outstanding
//! requests with optional timeout support.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::packet::Packet;
use crate::state::connection::{ConnectionPhase, PeerCapabilities};

// ── TrackedRequest ────────────────────────────────────────────────

/// A pending request that may expire after a deadline.
#[derive(Debug)]
pub struct TrackedRequest {
    /// The original packet that was sent.
    pub packet: Packet,
    /// When the request was submitted.
    pub sent_at: Instant,
    /// Optional deadline; `None` means no timeout.
    pub deadline: Option<Duration>,
}

impl TrackedRequest {
    /// Returns `true` if this request has exceeded its deadline.
    pub fn is_expired(&self) -> bool {
        match self.deadline {
            Some(d) => self.sent_at.elapsed() > d,
            None => false,
        }
    }

    /// How long this request has been in-flight.
    pub fn elapsed(&self) -> Duration {
        self.sent_at.elapsed()
    }
}

// ── MasterState ──────────────────────────────────────────────────

/// Tracks outstanding requests and connection state on the master side.
#[derive(Debug)]
pub struct MasterState {
    /// Current connection lifecycle phase.
    phase: ConnectionPhase,

    /// Capabilities negotiated with the peer (set after handshake).
    negotiated_capabilities: Option<PeerCapabilities>,

    /// Local capabilities advertised to the peer.
    local_capabilities: PeerCapabilities,

    /// Outstanding requests keyed by `request_id`.
    requests: HashMap<u64, TrackedRequest>,

    /// Default deadline applied to requests when none is specified.
    default_timeout: Option<Duration>,
}

impl MasterState {
    pub fn new() -> Self {
        Self {
            phase: ConnectionPhase::default(),
            negotiated_capabilities: None,
            local_capabilities: PeerCapabilities::default(),
            requests: HashMap::new(),
            default_timeout: None,
        }
    }

    // ── Connection Phase ──────────────────────────────────────────

    /// Returns a reference to the current connection phase.
    pub fn phase(&self) -> &ConnectionPhase {
        &self.phase
    }

    /// Returns a mutable reference to the connection phase for transitions.
    pub fn phase_mut(&mut self) -> &mut ConnectionPhase {
        &mut self.phase
    }

    // ── Capabilities ──────────────────────────────────────────────

    /// Returns the locally advertised capabilities.
    pub fn local_capabilities(&self) -> &PeerCapabilities {
        &self.local_capabilities
    }

    /// Sets the locally advertised capabilities.
    pub fn set_local_capabilities(&mut self, caps: PeerCapabilities) {
        self.local_capabilities = caps;
    }

    /// Returns the negotiated capabilities, if handshake completed.
    pub fn negotiated_capabilities(&self) -> Option<&PeerCapabilities> {
        self.negotiated_capabilities.as_ref()
    }

    /// Perform capability negotiation with the remote peer's caps.
    ///
    /// Stores the intersection and returns a reference to it.
    pub fn negotiate_capabilities(&mut self, remote: &PeerCapabilities) -> &PeerCapabilities {
        let negotiated = self.local_capabilities.negotiate(remote);
        self.negotiated_capabilities = Some(negotiated);
        self.negotiated_capabilities.as_ref().unwrap()
    }

    // ── Timeouts ──────────────────────────────────────────────────

    /// Set the default timeout applied to all new requests.
    pub fn set_default_timeout(&mut self, timeout: Duration) {
        self.default_timeout = Some(timeout);
    }

    /// Clear the default timeout (requests will not expire by default).
    pub fn clear_default_timeout(&mut self) {
        self.default_timeout = None;
    }

    // ── Request Tracking ──────────────────────────────────────────

    /// Track a request with the pool's default timeout.
    ///
    /// Backward-compatible with the original `track()` signature.
    pub fn track(&mut self, request_id: u64, packet: Packet) {
        self.track_with_deadline(request_id, packet, self.default_timeout);
    }

    /// Track a request with an explicit timeout.
    pub fn track_with_deadline(
        &mut self,
        request_id: u64,
        packet: Packet,
        deadline: Option<Duration>,
    ) {
        self.requests.insert(
            request_id,
            TrackedRequest {
                packet,
                sent_at: Instant::now(),
                deadline,
            },
        );
    }

    /// Resolve (complete) a request, returning its `Packet` if present.
    pub fn resolve(&mut self, request_id: u64) -> Option<Packet> {
        self.requests.remove(&request_id).map(|r| r.packet)
    }

    /// Number of in-flight requests.
    pub fn pending_count(&self) -> usize {
        self.requests.len()
    }

    /// Check if a specific request is pending.
    pub fn is_request_pending(&self, request_id: u64) -> bool {
        self.requests.contains_key(&request_id)
    }

    /// Returns the `TrackedRequest` for a given ID, if present.
    pub fn get_request(&self, request_id: u64) -> Option<&TrackedRequest> {
        self.requests.get(&request_id)
    }

    /// Returns all request IDs whose deadlines have expired.
    ///
    /// This does **not** remove them — the caller decides how to handle
    /// timed-out requests (e.g. notify the user, retry, or drop).
    pub fn check_timeouts(&self) -> Vec<u64> {
        self.requests
            .iter()
            .filter(|(_, req)| req.is_expired())
            .map(|(&id, _)| id)
            .collect()
    }

    /// Remove and return all expired requests.
    pub fn drain_expired(&mut self) -> Vec<(u64, TrackedRequest)> {
        let expired_ids: Vec<u64> = self.check_timeouts();
        expired_ids
            .into_iter()
            .filter_map(|id| self.requests.remove(&id).map(|r| (id, r)))
            .collect()
    }
}

impl Default for MasterState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Command;

    fn dummy_packet() -> Packet {
        Packet::new_command(1, Command::Ping, Vec::new()).unwrap()
    }

    #[test]
    fn track_and_resolve() {
        let mut state = MasterState::new();
        state.track(42, dummy_packet());
        assert_eq!(state.pending_count(), 1);
        assert!(state.is_request_pending(42));

        let pkt = state.resolve(42);
        assert!(pkt.is_some());
        assert_eq!(state.pending_count(), 0);
    }

    #[test]
    fn resolve_missing_returns_none() {
        let mut state = MasterState::new();
        assert!(state.resolve(999).is_none());
    }

    #[test]
    fn track_with_deadline_expires() {
        let mut state = MasterState::new();
        // Already-expired deadline (zero duration).
        state.track_with_deadline(1, dummy_packet(), Some(Duration::ZERO));
        // Give a tiny bit of time for elapsed() > 0
        std::thread::sleep(Duration::from_millis(1));

        let expired = state.check_timeouts();
        assert_eq!(expired, vec![1]);
    }

    #[test]
    fn track_without_deadline_never_expires() {
        let mut state = MasterState::new();
        state.track_with_deadline(1, dummy_packet(), None);
        assert!(state.check_timeouts().is_empty());
    }

    #[test]
    fn default_timeout_applied() {
        let mut state = MasterState::new();
        state.set_default_timeout(Duration::ZERO);
        state.track(1, dummy_packet());
        std::thread::sleep(Duration::from_millis(1));

        assert!(!state.check_timeouts().is_empty());
    }

    #[test]
    fn drain_expired_removes_entries() {
        let mut state = MasterState::new();
        state.track_with_deadline(1, dummy_packet(), Some(Duration::ZERO));
        state.track_with_deadline(2, dummy_packet(), None); // no timeout
        std::thread::sleep(Duration::from_millis(1));

        let drained = state.drain_expired();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].0, 1);
        assert_eq!(state.pending_count(), 1); // request 2 still alive
    }

    #[test]
    fn phase_starts_disconnected() {
        let state = MasterState::new();
        assert!(state.phase().is_disconnected());
    }

    #[test]
    fn phase_transitions() {
        let mut state = MasterState::new();
        state.phase_mut().begin_connect().unwrap();
        state.phase_mut().begin_handshake().unwrap();
        state.phase_mut().complete_handshake().unwrap();
        assert!(state.phase().is_connected());
    }

    #[test]
    fn negotiate_capabilities() {
        let mut state = MasterState::new();
        let remote = PeerCapabilities {
            screen_capture: false,
            ..Default::default()
        };
        let negotiated = state.negotiate_capabilities(&remote);
        assert!(!negotiated.screen_capture);
        assert!(state.negotiated_capabilities().is_some());
    }

    #[test]
    fn get_request_returns_tracked() {
        let mut state = MasterState::new();
        state.track(10, dummy_packet());
        let req = state.get_request(10).unwrap();
        assert!(req.deadline.is_none());
        assert!(req.elapsed() < Duration::from_secs(1));
    }
}
