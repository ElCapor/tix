//! Slave-side state tracking.
//!
//! Tracks the connection phase, negotiated capabilities, and the set
//! of actively running task IDs.

use std::collections::HashSet;

use crate::state::connection::{ConnectionPhase, PeerCapabilities};

/// Holds slave-local state: connection lifecycle, capabilities, and
/// which tasks are currently executing.
pub struct SlaveState {
    /// Current connection lifecycle phase.
    phase: ConnectionPhase,

    /// Capabilities advertised by this slave.
    local_capabilities: PeerCapabilities,

    /// Capabilities negotiated with the master (set after handshake).
    negotiated_capabilities: Option<PeerCapabilities>,

    /// Request IDs of tasks currently executing on this slave.
    active_tasks: HashSet<u64>,
}

impl SlaveState {
    pub fn new() -> Self {
        Self {
            phase: ConnectionPhase::default(),
            local_capabilities: PeerCapabilities::default(),
            negotiated_capabilities: None,
            active_tasks: HashSet::new(),
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

    /// Perform capability negotiation with the master's caps.
    ///
    /// Stores the intersection and returns a reference to it.
    pub fn negotiate_capabilities(&mut self, remote: &PeerCapabilities) -> &PeerCapabilities {
        let negotiated = self.local_capabilities.negotiate(remote);
        self.negotiated_capabilities = Some(negotiated);
        self.negotiated_capabilities.as_ref().unwrap()
    }

    // ── Task Tracking ─────────────────────────────────────────────

    /// Register a task as actively running.
    ///
    /// Returns `true` if this is a new task; `false` if the ID was
    /// already tracked (duplicate spawn guard).
    pub fn register_task(&mut self, request_id: u64) -> bool {
        self.active_tasks.insert(request_id)
    }

    /// Mark a task as completed and remove it from the active set.
    ///
    /// Returns `true` if the task was found and removed.
    pub fn complete_task(&mut self, request_id: u64) -> bool {
        self.active_tasks.remove(&request_id)
    }

    /// Number of tasks currently executing.
    pub fn active_task_count(&self) -> usize {
        self.active_tasks.len()
    }

    /// Check whether a specific task is running.
    pub fn is_task_running(&self, request_id: u64) -> bool {
        self.active_tasks.contains(&request_id)
    }

    /// Returns an iterator over all active task IDs.
    pub fn active_task_ids(&self) -> impl Iterator<Item = &u64> {
        self.active_tasks.iter()
    }
}

impl Default for SlaveState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_defaults() {
        let state = SlaveState::new();
        assert!(state.phase().is_disconnected());
        assert_eq!(state.active_task_count(), 0);
        assert!(state.negotiated_capabilities().is_none());
    }

    #[test]
    fn register_and_complete_task() {
        let mut state = SlaveState::new();
        assert!(state.register_task(1));
        assert!(state.register_task(2));
        assert_eq!(state.active_task_count(), 2);
        assert!(state.is_task_running(1));

        assert!(state.complete_task(1));
        assert!(!state.is_task_running(1));
        assert_eq!(state.active_task_count(), 1);
    }

    #[test]
    fn duplicate_register_returns_false() {
        let mut state = SlaveState::new();
        assert!(state.register_task(1));
        assert!(!state.register_task(1)); // already tracked
    }

    #[test]
    fn complete_unknown_task_returns_false() {
        let mut state = SlaveState::new();
        assert!(!state.complete_task(999));
    }

    #[test]
    fn phase_transitions() {
        let mut state = SlaveState::new();
        state.phase_mut().begin_connect().unwrap();
        state.phase_mut().begin_handshake().unwrap();
        state.phase_mut().complete_handshake().unwrap();
        assert!(state.phase().is_connected());
    }

    #[test]
    fn negotiate_capabilities() {
        let mut state = SlaveState::new();
        let remote = PeerCapabilities {
            compression: false,
            ..Default::default()
        };
        let negotiated = state.negotiate_capabilities(&remote);
        assert!(!negotiated.compression);
        assert!(state.negotiated_capabilities().is_some());
    }

    #[test]
    fn active_task_ids_iterator() {
        let mut state = SlaveState::new();
        state.register_task(10);
        state.register_task(20);
        let mut ids: Vec<u64> = state.active_task_ids().copied().collect();
        ids.sort();
        assert_eq!(ids, vec![10, 20]);
    }
}
