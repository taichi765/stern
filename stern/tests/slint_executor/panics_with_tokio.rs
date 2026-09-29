use std::time::Duration;

use stern::worker::{ForegroundExecutor as _, SlintExecutor};

#[test]
#[should_panic = "there is no reactor running, must be called from the context of a Tokio 1.x runtime"]
fn slint_panics_with_tokio_func() {
    i_slint_backend_testing::init_integration_test_with_system_time();

    let ex = SlintExecutor::new();
    ex.spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    slint::run_event_loop().unwrap();
}
