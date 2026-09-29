use stern::worker::{ForegroundExecutor as _, SlintExecutor};
use tokio::sync::oneshot;

#[test]
fn slint_spawn_local_works_fine() {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let ex = SlintExecutor::new();
    let (tx, rx) = oneshot::channel();

    ex.spawn({
        async move {
            let msg = rx.await.unwrap();
            assert_eq!(msg, "Violets are blue");
            slint::quit_event_loop().unwrap();
        }
    });

    tx.send("Violets are blue").unwrap();
    slint::run_event_loop().unwrap();
}
