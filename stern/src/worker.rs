use std::{pin::Pin, sync::Arc};

use parking_lot::RwLock;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const WORKER_CHANNEL_BUF: usize = 8;

/// A thread to run futures depends on tokio runtime.
#[derive(derive_more::Debug)]
pub struct WorkerThread<C> {
    #[debug(skip)]
    tx: mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    cx: Arc<C>,
    cancel_tok: CancellationToken,
}

impl<C> WorkerThread<C> {
    pub fn new(cx: C) -> Self {
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
            cancel_tok,
        }
    }

    /// Returns the context of worker.
    pub fn context(&mut self) -> Arc<C> {
        Arc::clone(&self.cx)
    }

    pub fn spawn<F>(&self, fut: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tx.blocking_send(Box::pin(fut)).unwrap();
    }

    /// Stops background worker thread.
    pub fn shutdown(&self) {
        self.cancel_tok.cancel();
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

// TODO: 停止できるようにする
impl<C> WorkerThread<C>
where
    C: Sync + Send + 'static,
{
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
}

impl<C> Clone for WorkerThread<C> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            cx: Arc::clone(&self.cx),
            cancel_tok: self.cancel_tok.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[test]
    fn worker_spawns_task_immediately() {
        let worker = WorkerThread::new();
        let (tx, rx) = oneshot::channel();

        worker.spawn(async move {
            tx.send("Hello!").unwrap();
        });

        let received = rx.blocking_recv().unwrap();
        assert_eq!("Hello!", received);
    }
}
