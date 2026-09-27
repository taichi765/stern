use std::{pin::Pin, rc::Rc, sync::Arc};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

const WORKER_CHANNEL_BUF: usize = 8;

/// A thread to run futures depends on tokio runtime.
#[derive(derive_more::Debug)]
pub struct WorkerThread<C, E> {
    #[debug(skip)]
    tx: mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    cx: Arc<C>,
    foreground_executor: E,
    /// Token to stop background worker thread.
    bg_cancel_tok: CancellationToken,
}

pub trait ForegroundExecutor {
    type SpawnedHandle<T>;

    fn spawn<T, Fut>(&self, fut: Fut) -> Self::SpawnedHandle<T>
    where
        Fut: Future<Output = T> + 'static,
        T: 'static;
}

/// Executor using [`slint`]'s event loop.
#[derive(Debug, Clone)]
pub struct SlintExecutor(());

impl SlintExecutor {
    /// Creates new slint executor.
    pub fn new() -> Self {
        Self(())
    }
}

impl ForegroundExecutor for SlintExecutor {
    type SpawnedHandle<T> = slint::JoinHandle<T>;

    /// Spawns future using [`slint::spawn_local()`].
    ///
    /// Panics when [`slint::spawn_local()`] returned error.
    fn spawn<T, Fut>(&self, fut: Fut) -> Self::SpawnedHandle<T>
    where
        Fut: Future<Output = T> + 'static,
    {
        slint::spawn_local(fut).expect("slint executor panicked")
    }
}

#[derive(Debug, Clone)]
pub struct SmolExecutor {
    cancel_tok: CancellationToken,
    ex: Rc<smol::LocalExecutor<'static>>,
}

impl SmolExecutor {
    pub fn new() -> Self {
        let ex = Rc::new(smol::LocalExecutor::new());
        let cancel_tok = CancellationToken::new();
        Self { cancel_tok, ex }
    }

    /// Starts foreground worker like [`slint::run_event_loop()`].
    pub fn start(&self) {
        let cancel_tok = self.cancel_tok.clone();
        let ex = self.ex.clone();
        smol::block_on(async {
            loop {
                let cancelled = cancel_tok.cancelled();
                tokio::select! {
                    _ = cancelled => break,
                    _ = ex.tick() => ()
                }
            }
        });
    }

    /// Stops foreground worker like [`slint::quit_event_loop()`].
    pub fn stop(&self) {
        self.cancel_tok.cancel();
    }
}

impl ForegroundExecutor for SmolExecutor {
    type SpawnedHandle<T> = smol::Task<T>;

    /// Spawns future using [`smol::Executor::spawn()`].
    fn spawn<T, Fut>(&self, fut: Fut) -> smol::Task<T>
    where
        Fut: Future<Output = T> + 'static,
        T: 'static,
    {
        self.ex.spawn(fut)
    }
}

impl<C> WorkerThread<C, SlintExecutor> {
    /// Creates and starts a new worker with default executor.
    ///
    /// To configure executor, use [`new_with_executor()`][Self::new_with_executor].
    pub fn new(cx: C) -> Self {
        Self::new_with_executor(cx, SlintExecutor::new())
    }
}

impl<C> WorkerThread<C, SmolExecutor> {
    /// Creates and starts a new worker with [`smol`] executor.
    ///
    /// To configure executor, use [`new_with_executor()`][Self::new_with_executor].
    pub fn new_smol(cx: C) -> Self {
        Self::new_with_executor(cx, SmolExecutor::new())
    }
}

impl<C, E> WorkerThread<C, E> {
    /// Creates and starts a new worker with provided executor.
    pub fn new_with_executor(cx: C, ex: E) -> Self {
        let (tx, rx) = mpsc::channel(WORKER_CHANNEL_BUF);
        let cancel_tok = CancellationToken::new();
        let _join = std::thread::Builder::new()
            .name("stern-worker".to_string())
            .spawn({
                let cancel_tok = cancel_tok.clone();
                move || Self::run(rx, cancel_tok)
            });
        Self {
            tx,
            cx: Arc::new(cx),
            foreground_executor: ex,
            bg_cancel_tok: cancel_tok,
        }
    }

    /// Returns the context of worker.
    pub fn context(&mut self) -> Arc<C> {
        Arc::clone(&self.cx)
    }

    /// Stops background worker thread.
    pub fn shutdown(&self) {
        self.bg_cancel_tok.cancel();
    }

    /// Returns reference to the foreground executor.
    pub fn foreground_executor(&self) -> &E {
        &self.foreground_executor
    }

    #[tokio::main]
    async fn run(
        mut rx: mpsc::Receiver<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
        cancel_tok: CancellationToken,
    ) {
        loop {
            tokio::select! {
                res = rx.recv() => {
                    let fut = res.expect("all WorkerThread instance has been dropped");
                    let _join = tokio::spawn(fut);
                },
                _ = cancel_tok.cancelled() => {
                    break;
                }
            }
        }
    }
}

impl<C, E> WorkerThread<C, E>
where
    C: Sync + Send + 'static,
    E: ForegroundExecutor,
{
    /// Spawns future on the current thread.
    pub fn spawn_local<T, Fut>(&self, fut: Fut) -> E::SpawnedHandle<T>
    where
        Fut: Future<Output = T> + 'static,
        T: 'static,
    {
        self.foreground_executor.spawn(fut)
    }
}

impl<C, E> WorkerThread<C, E>
where
    C: Sync + Send + 'static,
{
    /// Spawns async function in tokio's worker thread.
    ///
    /// # Example
    /// ```
    /// # use std::time::Duration;
    /// # use stern::WorkerThread;
    ///
    /// let worker = WorkerThread::new(());
    /// worker.spawn_cx(async move |_cx| {
    ///     tokio::time::sleep(Duration::from_secs(1)).await;
    ///     println!("Hello, World!");
    /// })
    /// ```
    pub fn spawn_cx<F, Fut>(&self, f: F)
    where
        F: FnOnce(Arc<C>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let cx = Arc::clone(&self.cx);
        self.tx
            .blocking_send(Box::pin(async move { f(cx).await }))
            .unwrap();
    }

    /// Spawns async function with current span, using [`tracing::Instrument::in_current_span()`].
    ///
    /// Equivalent to the code below using [`spawn_cx()`][WorkerThread::spawn_cx]:
    /// ```
    /// # use stern::WorkerThread;
    /// use tracing::{Instrument, trace};
    ///
    /// let worker = WorkerThread::new(());
    /// worker.spawn_cx(move |_cx| {
    ///     async move {
    ///         trace!("shaving yak");
    ///     }
    ///     .in_current_span()
    /// });
    /// ```
    pub fn spawn_spanned<F, Fut>(&self, f: F)
    where
        F: FnOnce(Arc<C>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let cx = Arc::clone(&self.cx);
        self.tx
            .blocking_send(Box::pin(async move { f(cx).await }.in_current_span()))
            .unwrap();
    }
}

impl<C, E> Clone for WorkerThread<C, E>
where
    E: Clone,
{
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            cx: Arc::clone(&self.cx),
            foreground_executor: self.foreground_executor.clone(),
            bg_cancel_tok: self.bg_cancel_tok.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    struct EmptyContext(());

    #[test]
    fn worker_spawns_task_immediately() {
        let worker = WorkerThread::new(EmptyContext(()));
        let (tx, rx) = oneshot::channel();

        worker.spawn_cx(async move |_| {
            tx.send("Hello!").unwrap();
        });

        let received = rx.blocking_recv().unwrap();
        assert_eq!("Hello!", received);
    }

    #[test]
    fn smol_spawn_local_works_fine() {
        let (tx, rx) = oneshot::channel();
        let worker = WorkerThread::new_smol(EmptyContext(()));

        let _handle = worker.spawn_local({
            let worker = worker.clone();
            async move {
                let msg = rx.await.unwrap();
                assert_eq!(msg, "Roses are red");
                worker.foreground_executor().stop();
            }
        });

        tx.send("Roses are red").unwrap();
        worker.foreground_executor().start();
    }

    #[test]
    fn slint_spawn_local_works_fine() {
        i_slint_backend_testing::init_integration_test_with_system_time();
        let worker = WorkerThread::new(EmptyContext(()));
        let (tx, rx) = oneshot::channel();

        let worker_clone = worker.clone();
        let _ = worker.spawn_local(async move {
            let msg = rx.await.unwrap();
            assert_eq!(msg, "Violets are blue");
            worker_clone.shutdown();
            slint::quit_event_loop().unwrap();
        });

        tx.send("Violets are blue").unwrap();
        slint::run_event_loop().unwrap();
    }
}
