use stern::WorkerThread;
use tokio::sync::oneshot;

#[test]
fn slint_spawn_local_works_fine() {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let worker = WorkerThread::new(());
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
