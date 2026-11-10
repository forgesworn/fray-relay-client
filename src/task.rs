//! The handle a background operation hands back, and the runtime it runs on.

use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

/// The one runtime this crate's workers run on. Created on first use; lives
/// for the process. A layer above may spawn its own work on it, so the process
/// has one runtime, not two.
///
/// It has two worker threads, shared with every relay connection and timer.
/// Spawn only async work that yields: blocking or CPU-heavy work stalls every
/// relay and makes dial and read time limits fire late, so run it with
/// `spawn_blocking` or on a thread of its own. Do not call `block_on` or
/// [`Task::wait`] from a future running on this runtime.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("fray-relay-client")
            .enable_all()
            .build()
            .expect("fray-relay-client: could not start the tokio runtime")
    })
}

/// The result of a background operation, delivered once.
pub struct Task<T> {
    rx: Mutex<mpsc::Receiver<T>>,
}

impl<T> std::fmt::Debug for Task<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Task")
    }
}

impl<T: Send + 'static> Task<T> {
    /// Run `fut` on [`runtime`]; its result arrives once, through the handle.
    /// `fut` shares the relay workers' two threads, so it must not block.
    pub fn spawn<F>(fut: F) -> Task<T>
    where
        F: std::future::Future<Output = T> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        runtime().spawn(async move {
            let _ = tx.send(fut.await);
        });
        Task { rx: Mutex::new(rx) }
    }

    /// A handle whose result is already here.
    pub fn ready(value: T) -> Task<T> {
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(value);
        Task { rx: Mutex::new(rx) }
    }

    /// The result, once (never blocks).
    pub fn poll(&self) -> Option<T> {
        self.rx.lock().ok()?.try_recv().ok()
    }

    /// Block until the result arrives or `timeout` passes (tools and tests;
    /// a frame loop uses [`Task::poll`]). Never from a future running on
    /// [`runtime`]: it would hold a worker thread.
    pub fn wait(&self, timeout: Duration) -> Option<T> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(v) = self.poll() {
                return Some(v);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
