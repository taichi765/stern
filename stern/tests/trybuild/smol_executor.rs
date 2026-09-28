fn assert_send<T: Send>() {}

fn main() {
    assert_send::<stern::worker::SmolExecutor>();
}
