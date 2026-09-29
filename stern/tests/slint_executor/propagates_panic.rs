use stern::worker::{ForegroundExecutor as _, SlintExecutor};

#[test]
#[should_panic = "Boom!"]
fn slint_propagates_panic() {
    i_slint_backend_testing::init_integration_test_with_system_time();

    let ex = SlintExecutor::new();
    ex.spawn({
        async move {
            panic!("Boom!");
        }
    });
    slint::run_event_loop().unwrap();
}
