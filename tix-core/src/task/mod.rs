//! Async task pool for managing spawned work (shell commands, file ops, etc.).
//!
//! Each task is tracked by its `request_id` and reports completion or
//! failure through a dedicated event channel.
//!
//! ## Features
//!
//! - **CancellationToken**: every task receives a token; `cancel_task()`
//!   or `cancel_all()` signal cooperative cancellation.
//! - **Per-task timeout**: optionally auto-cancel after a deadline.
//! - **Typed errors**: `TaskEvent::Error` carries a [`TaskError`] enum.
//! - **Metadata**: spawned time, optional name, active count.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

/// A boxed async task factory: takes a connection sender, request ID, and
/// payload, returning a pinned future. Used for trait-object-friendly task
/// spawning.
pub type BoxedTaskFn = Box<
    dyn FnOnce(ConnectionSender, u64, Vec<u8>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send,
>;

use crate::error::TaskError;
use crate::network::ConnectionSender;

// ── TaskEvent ────────────────────────────────────────────────────

/// Sender half of the task-event channel.
pub type TaskEventSender = tokio::sync::mpsc::Sender<TaskEvent>;

/// Events emitted by tasks to signal completion or failure.
#[derive(Debug)]
pub enum TaskEvent {
    /// The task completed successfully.
    Finished(u64),
    /// The task failed with a typed error.
    Error(u64, TaskError),
}

// ── TaskOptions ──────────────────────────────────────────────────

/// Configuration for a spawned task.
#[derive(Debug, Clone, Default)]
pub struct TaskOptions {
    /// Human-readable name for logging / diagnostics.
    pub name: Option<String>,
    /// If set, the task is auto-cancelled after this duration.
    pub timeout: Option<Duration>,
}

impl TaskOptions {
    /// Create default options (no name, no timeout).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a human-readable name for diagnostics.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set an automatic cancellation timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

// ── Task ─────────────────────────────────────────────────────────

/// A handle to a spawned task.
pub struct Task {
    _req_id: u64,
    _handle: tokio::task::JoinHandle<()>,
    /// Token used to signal cooperative cancellation.
    token: CancellationToken,
    /// When the task was spawned.
    spawned_at: Instant,
    /// Optional human-readable name.
    name: Option<String>,
}

impl Task {
    /// Spawn a new task from an async closure.
    ///
    /// The closure runs inside a `tokio::select!` against the
    /// cancellation token so it can be stopped cooperatively.
    pub fn spawn<F, Fut>(
        tx: ConnectionSender,
        req_id: u64,
        payload: Vec<u8>,
        f: F,
        event_tx: TaskEventSender,
        options: TaskOptions,
    ) -> Self
    where
        F: FnOnce(ConnectionSender, u64, Vec<u8>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let token = CancellationToken::new();
        let child_token = token.child_token();
        let timeout = options.timeout;

        let handle = tokio::spawn(async move {
            let work = f(tx, req_id, payload);

            match timeout {
                Some(dur) => {
                    tokio::select! {
                        biased;
                        _ = child_token.cancelled() => {
                            let _ = event_tx.send(TaskEvent::Error(req_id, TaskError::Cancelled)).await;
                            return;
                        }
                        _ = tokio::time::sleep(dur) => {
                            let _ = event_tx.send(TaskEvent::Error(req_id, TaskError::Timeout(dur))).await;
                            return;
                        }
                        () = work => {}
                    }
                }
                None => {
                    tokio::select! {
                        biased;
                        _ = child_token.cancelled() => {
                            let _ = event_tx.send(TaskEvent::Error(req_id, TaskError::Cancelled)).await;
                            return;
                        }
                        () = work => {}
                    }
                }
            }

            let _ = event_tx.send(TaskEvent::Finished(req_id)).await;
        });

        Self {
            _req_id: req_id,
            _handle: handle,
            token,
            spawned_at: Instant::now(),
            name: options.name,
        }
    }

    /// Spawn from a boxed future (trait-object friendly).
    pub fn spawn_boxed(
        tx: ConnectionSender,
        req_id: u64,
        payload: Vec<u8>,
        f: BoxedTaskFn,
        event_tx: TaskEventSender,
        options: TaskOptions,
    ) -> Self {
        let token = CancellationToken::new();
        let child_token = token.child_token();
        let timeout = options.timeout;

        let handle = tokio::spawn(async move {
            let work = f(tx, req_id, payload);

            match timeout {
                Some(dur) => {
                    tokio::select! {
                        biased;
                        _ = child_token.cancelled() => {
                            let _ = event_tx.send(TaskEvent::Error(req_id, TaskError::Cancelled)).await;
                            return;
                        }
                        _ = tokio::time::sleep(dur) => {
                            let _ = event_tx.send(TaskEvent::Error(req_id, TaskError::Timeout(dur))).await;
                            return;
                        }
                        () = work => {}
                    }
                }
                None => {
                    tokio::select! {
                        biased;
                        _ = child_token.cancelled() => {
                            let _ = event_tx.send(TaskEvent::Error(req_id, TaskError::Cancelled)).await;
                            return;
                        }
                        () = work => {}
                    }
                }
            }

            let _ = event_tx.send(TaskEvent::Finished(req_id)).await;
        });

        Self {
            _req_id: req_id,
            _handle: handle,
            token,
            spawned_at: Instant::now(),
            name: options.name,
        }
    }

    /// Signal cooperative cancellation of this task.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// When this task was spawned.
    pub fn spawned_at(&self) -> Instant {
        self.spawned_at
    }

    /// Optional human-readable name.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Get a child token that downstream work can use to check for
    /// cancellation.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.token.child_token()
    }
}

// ── TaskPool ─────────────────────────────────────────────────────

/// Pool that tracks in-flight tasks and dispatches events.
pub struct TaskPool {
    tasks: HashMap<u64, Task>,
    pool_rx: tokio::sync::mpsc::Receiver<TaskEvent>,
    pool_tx: tokio::sync::mpsc::Sender<TaskEvent>,
    finished_callbacks: Vec<Box<dyn Fn(u64) + Send + Sync + 'static>>,
}

impl TaskPool {
    /// Create an empty task pool with a 1024-slot event channel.
    pub fn new() -> Self {
        let (pool_tx, pool_rx) = tokio::sync::mpsc::channel(1024);
        Self {
            tasks: HashMap::new(),
            pool_rx,
            pool_tx,
            finished_callbacks: Vec::new(),
        }
    }

    /// Spawn a task with a generic async function (backward-compatible).
    ///
    /// Uses default options (no timeout, no name).
    pub fn spawn<F, Fut>(&mut self, tx: ConnectionSender, req_id: u64, payload: Vec<u8>, f: F)
    where
        F: FnOnce(ConnectionSender, u64, Vec<u8>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.spawn_with_options(tx, req_id, payload, f, TaskOptions::default());
    }

    /// Spawn a task with explicit options (name, timeout).
    pub fn spawn_with_options<F, Fut>(
        &mut self,
        tx: ConnectionSender,
        req_id: u64,
        payload: Vec<u8>,
        f: F,
        options: TaskOptions,
    ) where
        F: FnOnce(ConnectionSender, u64, Vec<u8>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let task = Task::spawn(tx, req_id, payload, f, self.pool_tx.clone(), options);
        self.tasks.insert(req_id, task);
    }

    /// Spawn with a boxed future (backward-compatible).
    pub fn spawn_boxed(
        &mut self,
        tx: ConnectionSender,
        req_id: u64,
        payload: Vec<u8>,
        f: BoxedTaskFn,
    ) {
        let task = Task::spawn_boxed(
            tx,
            req_id,
            payload,
            f,
            self.pool_tx.clone(),
            TaskOptions::default(),
        );
        self.tasks.insert(req_id, task);
    }

    /// Spawn boxed with explicit options.
    pub fn spawn_boxed_with_options(
        &mut self,
        tx: ConnectionSender,
        req_id: u64,
        payload: Vec<u8>,
        f: BoxedTaskFn,
        options: TaskOptions,
    ) {
        let task = Task::spawn_boxed(tx, req_id, payload, f, self.pool_tx.clone(), options);
        self.tasks.insert(req_id, task);
    }

    // ── Cancellation ──────────────────────────────────────────────

    /// Cancel a single task by its request ID.
    ///
    /// Returns `true` if the task was found and signalled.
    pub fn cancel_task(&self, req_id: u64) -> bool {
        if let Some(task) = self.tasks.get(&req_id) {
            task.cancel();
            true
        } else {
            false
        }
    }

    /// Cancel all in-flight tasks.
    pub fn cancel_all(&self) {
        for task in self.tasks.values() {
            task.cancel();
        }
    }

    // ── Query ─────────────────────────────────────────────────────

    /// Number of tasks currently tracked.
    pub fn active_count(&self) -> usize {
        self.tasks.len()
    }

    /// Check whether a task with the given ID is tracked.
    pub fn is_active(&self, req_id: u64) -> bool {
        self.tasks.contains_key(&req_id)
    }

    /// Returns metadata about a tracked task.
    pub fn get_task(&self, req_id: u64) -> Option<&Task> {
        self.tasks.get(&req_id)
    }

    // ── Callbacks & Events ────────────────────────────────────────

    /// Register a callback invoked when any task finishes.
    pub fn on_finished<F>(&mut self, f: F)
    where
        F: Fn(u64) + Send + Sync + 'static,
    {
        self.finished_callbacks.push(Box::new(f));
    }

    /// Receive the next event, or `None` if all senders dropped.
    pub async fn recv(&mut self) -> Option<TaskEvent> {
        self.pool_rx.recv().await
    }

    /// Process a single task event.
    pub async fn process_event(&mut self, event: TaskEvent) {
        match &event {
            TaskEvent::Finished(id) | TaskEvent::Error(id, _) => {
                self.tasks.remove(id);
                for cb in &self.finished_callbacks {
                    cb(*id);
                }
                if let TaskEvent::Error(id, err) = &event {
                    eprintln!("[TASK] {id} failed: {err}");
                }
            }
        }
    }

    /// Consume the pool into a background processing loop.
    pub fn start(mut self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(event) = self.pool_rx.recv().await {
                self.process_event(event).await;
            }
        })
    }

    /// Clone the event sender for use in spawned tasks.
    pub fn event_sender(&self) -> TaskEventSender {
        self.pool_tx.clone()
    }
}

impl Default for TaskPool {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    /// Helper: create a dummy ConnectionSender.
    fn dummy_sender() -> ConnectionSender {
        let (tx, _rx) = mpsc::channel(1);
        tx
    }

    #[tokio::test]
    async fn spawn_and_finish() {
        let mut pool = TaskPool::new();
        let tx = dummy_sender();

        pool.spawn(tx, 1, Vec::new(), |_tx, _req, _payload| async {});

        assert_eq!(pool.active_count(), 1);
        assert!(pool.is_active(1));

        // Wait for finish event
        let event = pool.recv().await.unwrap();
        assert!(matches!(event, TaskEvent::Finished(1)));
        pool.process_event(event).await;
        assert_eq!(pool.active_count(), 0);
    }

    #[tokio::test]
    async fn cancel_task_signals_cancelled() {
        let mut pool = TaskPool::new();
        let tx = dummy_sender();

        pool.spawn(tx, 42, Vec::new(), |_tx, _req, _payload| async {
            // Long-running task
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        assert!(pool.cancel_task(42));

        let event = pool.recv().await.unwrap();
        match event {
            TaskEvent::Error(id, ref err) => {
                assert_eq!(id, 42);
                assert!(matches!(err, TaskError::Cancelled));
            }
            _ => panic!("expected Error(Cancelled)"),
        }
    }

    #[tokio::test]
    async fn cancel_all_cancels_everything() {
        let mut pool = TaskPool::new();

        for i in 1..=3 {
            let tx = dummy_sender();
            pool.spawn(tx, i, Vec::new(), |_tx, _req, _payload| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
            });
        }

        assert_eq!(pool.active_count(), 3);
        pool.cancel_all();

        // Drain all 3 cancellation events
        for _ in 0..3 {
            let event = pool.recv().await.unwrap();
            assert!(matches!(event, TaskEvent::Error(_, TaskError::Cancelled)));
            pool.process_event(event).await;
        }
        assert_eq!(pool.active_count(), 0);
    }

    #[tokio::test]
    async fn timeout_auto_cancels() {
        let mut pool = TaskPool::new();
        let tx = dummy_sender();
        let opts = TaskOptions::new().with_timeout(Duration::from_millis(10));

        pool.spawn_with_options(
            tx,
            99,
            Vec::new(),
            |_tx, _req, _payload| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
            },
            opts,
        );

        let event = pool.recv().await.unwrap();
        match event {
            TaskEvent::Error(id, ref err) => {
                assert_eq!(id, 99);
                assert!(matches!(err, TaskError::Timeout(_)));
            }
            _ => panic!("expected Error(Timeout)"),
        }
    }

    #[tokio::test]
    async fn task_name_metadata() {
        let mut pool = TaskPool::new();
        let tx = dummy_sender();
        let opts = TaskOptions::new().with_name("shell-exec");

        pool.spawn_with_options(
            tx,
            7,
            Vec::new(),
            |_tx, _req, _payload| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
            },
            opts,
        );

        let task = pool.get_task(7).unwrap();
        assert_eq!(task.name(), Some("shell-exec"));
        assert!(task.spawned_at().elapsed() < Duration::from_secs(1));

        // cleanup
        pool.cancel_task(7);
        let _ = pool.recv().await;
    }

    #[test]
    fn cancel_unknown_returns_false() {
        let pool = TaskPool::new();
        assert!(!pool.cancel_task(999));
    }

    #[tokio::test]
    async fn finished_callback_invoked() {
        let mut pool = TaskPool::new();
        let (cb_tx, mut cb_rx) = mpsc::channel::<u64>(8);

        pool.on_finished(move |id| {
            let _ = cb_tx.try_send(id);
        });

        let tx = dummy_sender();
        pool.spawn(tx, 5, Vec::new(), |_tx, _req, _payload| async {});

        let event = pool.recv().await.unwrap();
        pool.process_event(event).await;

        let finished_id = cb_rx.recv().await.unwrap();
        assert_eq!(finished_id, 5);
    }
}
