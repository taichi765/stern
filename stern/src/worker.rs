use std::{pin::Pin, rc::Rc, sync::Arc};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

const WORKER_CHANNEL_BUF: usize = 8;

/// A thread to run futures depends on tokio runtime.
#[derive(Debug)]
pub struct WorkerThread<C, E> {
    foreground_executor: E,
    background_executor: BackgroundExecutor<C>,
}

impl<C, E: Clone> Clone for WorkerThread<C, E> {
    fn clone(&self) -> Self {
        Self {
            foreground_executor: self.foreground_executor.clone(),
            background_executor: self.background_executor.clone(),
        }
    }
}

/// A handle to the background worker thread.
#[derive(derive_more::Debug)]
pub struct BackgroundExecutor<C> {
    #[debug(skip)]
    tx: mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    cx: Arc<C>,
    /// Token to stop background worker thread.
    cancel_tok: CancellationToken,
}

impl<C> Clone for BackgroundExecutor<C> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            cx: Arc::clone(&self.cx),
            cancel_tok: self.cancel_tok.clone(),
        }
    }
}

impl<C> BackgroundExecutor<C> {
    /// Stops the background worker thread.
    pub fn shutdown(&self) {
        self.cancel_tok.cancel();
    }

    /// Returns the context of worker.
    pub fn context(&self) -> Arc<C> {
        Arc::clone(&self.cx)
    }
}

impl<C> BackgroundExecutor<C>
where
    C: Send + Sync + 'static,
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
    pub fn spawn<F, Fut>(&self, f: F)
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
    type SpawnedHandle<T> = ();

    /// Spawns future using [`smol::Executor::spawn()`].
    ///
    /// The function does not return [`smol::Task`] but `()`.
    ///
    /// [`Task::detach()`] is called inside the function because if the caller does not keep
    /// the returned value alive, spawned task is cancelled.
    ///
    /// The caller does not know whether the actual executor is [`SlintExecutor`] or [`SmolExecutor`] and so
    /// returned value need to be kept alive or not.
    ///
    /// [`Task::detach()`]: smol::Task::detach
    fn spawn<T, Fut>(&self, fut: Fut) -> Self::SpawnedHandle<T>
    where
        Fut: Future<Output = T> + 'static,
        T: 'static,
    {
        self.ex.spawn(fut).detach();
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

    /// Stops background worker thread and foreground worker.
    pub fn shutdown_all(&self) {
        self.foreground_executor().stop();
        self.background_executor().shutdown();
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
            foreground_executor: ex,
            background_executor: BackgroundExecutor {
                tx,
                cx: Arc::new(cx),
                cancel_tok,
            },
        }
    }

    /// Returns the context of worker.
    pub fn context(&mut self) -> Arc<C> {
        self.background_executor().context()
    }

    /// Returns reference to the foreground executor.
    pub fn foreground_executor(&self) -> &E {
        &self.foreground_executor
    }

    /// Returns reference to the background executor.
    pub fn background_executor(&self) -> &BackgroundExecutor<C> {
        &self.background_executor
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
        self.background_executor().spawn(f);
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
        self.background_executor().spawn_spanned(f);
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

        worker.spawn_local({
            let worker = worker.clone();
            async move {
                let msg = rx.await.unwrap();
                assert_eq!(msg, "Roses are red");
                worker.shutdown_all();
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

        worker.spawn_local({
            let worker = worker.clone();
            async move {
                let msg = rx.await.unwrap();
                assert_eq!(msg, "Violets are blue");
                worker.background_executor().shutdown();
                slint::quit_event_loop().unwrap();
            }
        });

        tx.send("Violets are blue").unwrap();
        slint::run_event_loop().unwrap();
    }
}
