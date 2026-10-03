use super::*;
use std::sync::Arc;

#[test]
fn idle_wait_observes_completion_and_timeout_without_holding_worker_lock() {
    let controller = Arc::new(AnalysisController::default());
    controller.wait_idle(Duration::from_millis(10)).unwrap();
    controller.state.lock().unwrap().busy = true;
    assert!(controller.wait_idle(Duration::from_millis(20)).is_err());
    let worker = controller.clone();
    let handle = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        worker.state.lock().unwrap().busy = false;
        worker.idle.notify_all();
    });
    controller.wait_idle(Duration::from_secs(1)).unwrap();
    handle.join().unwrap();
}
