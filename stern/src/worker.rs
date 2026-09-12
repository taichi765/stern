use std::{pin::Pin, sync::Arc};

use parking_lot::RwLock;
use tokio::sync::mpsc;

const WORKER_CHANNEL_BUF: usize = 8;

/// A thread to run futures depends on tokio runtime.
#[derive(derive_more::Debug)]
pub struct WorkerThread<C> {
    #[debug(skip)]
    tx: mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    cx: Arc<RwLock<C>>,
}

// TODO: 停止できるようにする
impl<C> WorkerThread<C>
where
    C: Sync + Send + 'static,
{
    pub fn new(cx: C) -> Self {
        let (tx, rx) = mpsc::channel(WORKER_CHANNEL_BUF);
        let _join = std::thread::Builder::new()
            .name("stern-worker".to_string())
            .spawn(move || Self::run(rx));
        Self {
            tx,
            cx: Arc::new(RwLock::new(cx)),
        }
    }

    #[tokio::main]
    async fn run(mut rx: mpsc::Receiver<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>) {
        loop {
            let fut = rx.recv().await.unwrap();
            let _join = tokio::spawn(fut);
        }
    }

    /// Returns the context of worker.
    pub fn context(&mut self) -> Arc<RwLock<C>> {
        Arc::clone(&self.cx)
    }

    pub fn spawn<F>(&self, fut: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tx.blocking_send(Box::pin(fut)).unwrap();
    }

    pub fn spawn_cx<F, Fut>(&self, f: F)
    where
        F: FnOnce(Arc<RwLock<C>>) -> Fut + Send + 'static,
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
