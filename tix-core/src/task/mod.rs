use std::{collections::HashMap, future::Future, pin::Pin};

/// Types
pub type Task = TixTask;
pub type TaskPool = TixTaskPool;
pub type TaskEventSender = tokio::sync::mpsc::Sender<TaskEvent>;
type TaskFinishedCallback = Box<dyn Fn(u64) + Send + Sync + 'static>;

#[derive(Debug)]
pub enum TaskEvent {
    Finished(u64),
    Error(u64, String),
}

pub struct TixTask {
    req_id: u64,
    handle: tokio::task::JoinHandle<()>,
}

impl TixTask {
    /// Spawns a new task that will execute the given function `f` on the event loop.
    /// 
    /// # Arguments
    /// 
    /// * `tx` - The sender half of the connection channel to send task events to.
    /// * `req_id` - The unique identifier for the task.
    /// * `payload` - The payload data to be passed to the task function.
    /// * `f` - The async function to be executed as the task.
    /// * `event_tx` - The sender half of the task event channel to send task events to.
    pub fn spawn<F, Fut>(
        tx: crate::TixConnectionSender,
        req_id: u64,
        payload: Vec<u8>,
        f: F,
        event_tx: TaskEventSender,
    ) -> Self
    where
        F: FnOnce(crate::TixConnectionSender, u64, Vec<u8>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self {
            req_id,
            handle: tokio::spawn(async move {
                f(tx, req_id, payload).await;
                let _ = event_tx.send(TaskEvent::Finished(req_id)).await;
            }),
        }
    }

    /// Alternative: Using Boxed Future for more flexibility
    pub fn spawn_boxed(
        tx: crate::TixConnectionSender,
        req_id: u64,
        payload: Vec<u8>,
        f: Box<dyn FnOnce(crate::TixConnectionSender, u64, Vec<u8>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>,
        event_tx: TaskEventSender,
    ) -> Self {
        Self {
            req_id,
            handle: tokio::spawn(async move {
                f(tx, req_id, payload).await;
                let _ = event_tx.send(TaskEvent::Finished(req_id)).await;
            }),
        }
    }
}

pub struct TixTaskPool {
    tasks: HashMap<u64, Task>,
    pool_rx: tokio::sync::mpsc::Receiver<TaskEvent>,
    pool_tx: tokio::sync::mpsc::Sender<TaskEvent>,
    finished_callbacks: Vec<TaskFinishedCallback>,
}

impl TixTaskPool {
    pub fn new() -> Self {
        let (pool_tx, pool_rx) = tokio::sync::mpsc::channel(1024);
        Self {
            tasks: HashMap::new(),
            pool_rx,
            pool_tx,
            finished_callbacks: Vec::new(),
        }
    }

    /// Spawn a task with an async function
    pub fn task_spawn<F, Fut>(
        &mut self,
        tx: crate::TixConnectionSender,
        req_id: u64,
        payload: Vec<u8>,
        f: F,
    ) where
        F: FnOnce(crate::TixConnectionSender, u64, Vec<u8>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let task = TixTask::spawn(tx, req_id, payload, f, self.pool_tx.clone());
        self.tasks.insert(req_id, task);
    }

    /// Spawn a task with a boxed future (more flexible, less generic)
    pub fn task_spawn_boxed(
        &mut self,
        tx: crate::TixConnectionSender,
        req_id: u64,
        payload: Vec<u8>,
        f: Box<dyn FnOnce(crate::TixConnectionSender, u64, Vec<u8>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>,
    ) {
        let task = TixTask::spawn_boxed(tx, req_id, payload, f, self.pool_tx.clone());
        self.tasks.insert(req_id, task);
    }

    /// Helper for synchronous functions (wraps them in async)
    pub fn task_spawn_sync<F>(
        &mut self,
        tx: crate::TixConnectionSender,
        req_id: u64,
        payload: Vec<u8>,
        f: F,
    ) where
        F: FnOnce(crate::TixConnectionSender, u64, Vec<u8>) + Send + 'static,
    {
        let task = TixTask::spawn(tx, req_id, payload, move |tx, req_id, payload| {
            async move {
                f(tx, req_id, payload);
            }
        }, self.pool_tx.clone());
        self.tasks.insert(req_id, task);
    }

    pub fn on_task_finished<F>(&mut self, f: F)
    where
        F: Fn(u64) + Send + Sync + 'static,
    {
        self.finished_callbacks.push(Box::new(f));
    }

    pub async fn process_events(&mut self) {
        while let Some(event) = self.pool_rx.recv().await {
            self.process_event(event).await;
        }
    }

    pub async fn recv(&mut self) -> Option<TaskEvent> {
        self.pool_rx.recv().await
    }

    pub async fn process_event(&mut self, event: TaskEvent) {
        match event {
            TaskEvent::Finished(req_id) => {
                self.tasks.remove(&req_id);
                for callback in &self.finished_callbacks {
                    callback(req_id);
                }
            }
            TaskEvent::Error(req_id, err) => {
                self.tasks.remove(&req_id);
                eprintln!("Task {} failed: {}", req_id, err);
                for callback in &self.finished_callbacks {
                    callback(req_id);
                }
            }
        }
    }

    pub fn start(mut self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.process_events().await;
        })
    }

    /// Wait for a specific task to complete
    pub async fn wait_for(&mut self, req_id: u64) -> Option<()> {
        while let Some(event) = self.pool_rx.recv().await {
            if let TaskEvent::Finished(id) | TaskEvent::Error(id, _) = &event {
                if *id == req_id {
                    self.process_event(event).await;
                    return Some(());
                }
            }
            self.process_event(event).await;
        }
        None
    }

    pub fn clone_tx(&self) -> TaskEventSender {
        self.pool_tx.clone()
    }
}