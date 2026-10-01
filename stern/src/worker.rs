use std::{cell::RefCell, marker::PhantomData, pin::Pin, rc::Rc, sync::Arc};

use futures_util::FutureExt;
use tokio::sync::{mpsc, oneshot};
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

    /// Creates new [`BackgroundExecutorDropGuard`].
    ///
    /// Returned guard will shuts down the background worker thread on drop.
    #[must_use = "returned guard need to be hold"]
    pub fn drop_guard(&self) -> BackgroundExecutorDropGuard {
        BackgroundExecutorDropGuard {
            _guard: self.cancel_tok.clone().drop_guard(),
        }
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

/// Shuts down background worker thread on drop.
pub struct BackgroundExecutorDropGuard {
    _guard: tokio_util::sync::DropGuard,
}

pub trait ForegroundExecutor {
    type SpawnedHandle;

    fn spawn<Fut>(&self, fut: Fut) -> Self::SpawnedHandle
    where
        Fut: Future<Output = ()> + 'static;
}

/// Executor using [`slint`]'s event loop.
///
/// This type implements `!Send` because [`slint::spawn_local()`] cannot be called from non-main thread.
#[derive(Debug, Clone)]
pub struct SlintExecutor {
    // FIXME: using marker type with Rc until negative_impls will be stabilized
    _marker: PhantomData<Rc<()>>,
}

impl SlintExecutor {
    /// Creates new slint executor.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl ForegroundExecutor for SlintExecutor {
    type SpawnedHandle = slint::JoinHandle<()>;

    /// Spawns future using [`slint::spawn_local()`].
    ///
    /// Panics when [`slint::spawn_local()`] returned error.
    fn spawn<Fut>(&self, fut: Fut) -> Self::SpawnedHandle
    where
        Fut: Future<Output = ()> + 'static,
    {
        slint::spawn_local(fut).expect("slint executor panicked")
    }
}

/// Foreground executor backed by [`async_executor::LocalExecutor`].
///
/// This executor is designed to emulate [`slint::spawn_local()`].
///
/// For example, it
/// - panics when you called tokio's runtime-dependent function (e.g. [`tokio::time::sleep()`]),
/// without workarounds like [async-compat](https://crates.io/crates/async-compat).
/// - propagates panic occured in a future passed to [`spawn()`][ForegroundExecutor::spawn()].
#[derive(Debug, Clone)]
pub struct SmolExecutor {
    cancel_tok: CancellationToken,
    ex: Rc<async_executor::LocalExecutor<'static>>,
    spawned_tasks: Rc<RefCell<Vec<SmolTask>>>,
}

impl SmolExecutor {
    pub fn new() -> Self {
        let ex = Rc::new(async_executor::LocalExecutor::new());
        let cancel_tok = CancellationToken::new();
        Self {
            cancel_tok,
            ex,
            spawned_tasks: Default::default(),
        }
    }

    /// Starts foreground worker like [`slint::run_event_loop()`].
    pub fn start(&self) {
        async_io::block_on(async {
            loop {
                let handle_tick = async || {
                    let idx = self
                        .spawned_tasks
                        .borrow()
                        .iter()
                        .enumerate()
                        .find_map(|(idx, t)| t.0.is_finished().then_some(idx));
                    if let Some(idx) = idx {
                        let task = self.spawned_tasks.borrow_mut().remove(idx);
                        task.0.await;
                        // notify to `SmolTaskHandle` that the task finished.
                        // ignore `Err` when the handle is already dropped.
                        let _ = task.1.send(());
                    }
                };
                tokio::select! {
                    _ = self.cancel_tok.cancelled() => break,
                    _ = self.ex.tick() => handle_tick().await,
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
    type SpawnedHandle = SmolTaskHandle;

    /// Spawns future using [`async_executor::LocalExecutor::spawn()`].
    ///
    /// The function does not return [`async_task::Task`] but `()`.
    ///
    /// [`Task::detach()`] is called inside the function because if the caller does not keep
    /// the returned value alive, spawned task is cancelled.
    ///
    /// The caller does not know whether the actual executor is [`SlintExecutor`] or [`SmolExecutor`] and so
    /// returned value need to be kept alive or not.
    ///
    /// [`Task::detach()`]: async_task::Task::detach
    fn spawn<Fut>(&self, fut: Fut) -> Self::SpawnedHandle
    where
        Fut: Future<Output = ()> + 'static,
    {
        let task = self.ex.spawn(fut);
        let (tx, rx) = oneshot::channel();
        self.spawned_tasks.borrow_mut().push(SmolTask(task, tx));
        SmolTaskHandle(rx)
    }
}

#[derive(Debug)]
struct SmolTask(async_task::Task<()>, oneshot::Sender<()>);

pub struct SmolTaskHandle(oneshot::Receiver<()>);

impl Future for SmolTaskHandle {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        match self.0.poll_unpin(cx) {
            Poll::Ready(Ok(_)) => Poll::Ready(()),
            Poll::Ready(Err(e)) => {
                panic!("foreground executor unexpectedly closed channel: {}", e)
            }
            Poll::Pending => Poll::Pending,
        }
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
    pub fn spawn_local<Fut>(&self, fut: Fut) -> E::SpawnedHandle
    where
        Fut: Future<Output = ()> + 'static,
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
    use std::time::Duration;

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
    #[should_panic = "Boom!"]
    fn smol_propagates_panic() {
        let worker = WorkerThread::new_smol(EmptyContext(()));
        worker.spawn_local({
            let guard = worker.background_executor().drop_guard();
            async move {
                let _guard = guard;
                panic!("Boom!");
            }
        });
        worker.foreground_executor().start();
    }

    #[test]
    #[should_panic = "there is no reactor running, must be called from the context of a Tokio 1.x runtime"]
    fn smol_panics_with_tokio_func() {
        let ex = SmolExecutor::new();
        ex.spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        ex.start();
    }
}
