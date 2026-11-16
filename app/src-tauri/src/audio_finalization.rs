//! A bounded in-process reconciliation of the same durable audio operation.
//! The supplied finalizer must revalidate its evidence and recognize an already
//! committed result. Never replay capture, decoding or an import request here.
use std::time::{Duration, Instant};

pub(crate) fn complete<T>(
    cancelled: &impl Fn() -> bool,
    finalize: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    complete_with_wait(cancelled, finalize, || {
        let until = Instant::now() + Duration::from_millis(500);
        while !cancelled() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(25));
        }
    })
}

fn complete_with_wait<T>(
    cancelled: &impl Fn() -> bool,
    mut finalize: impl FnMut() -> Result<T, String>,
    wait: impl FnOnce(),
) -> Result<T, String> {
    let mut attempt = || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut finalize)).unwrap_or_else(|_| {
            Err("音声の保存処理が予期せず停止しました。元音声は保持しています".into())
        })
    };
    // Even a lost completion response is reconciled by the same finalizer,
    // whose saved/ready branch verifies the committed WAV without new rows.
    let result = attempt();
    if result.is_ok() || cancelled() {
        return result;
    }
    if let Err(error) = &result {
        crate::internal_logs::report_error("audio_save", error);
    }
    wait();
    if cancelled() {
        return result;
    }
    attempt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn lost_commit_response_reconciles_without_repeating_the_commit() {
        let committed = Cell::new(false);
        let writes = Cell::new(0);
        let result = complete_with_wait(
            &|| false,
            || {
                if committed.get() {
                    return Ok("same-operation");
                }
                committed.set(true);
                writes.set(writes.get() + 1);
                Err("lost response".into())
            },
            || {},
        );
        assert_eq!(result.unwrap(), "same-operation");
        assert_eq!(writes.get(), 1);
    }

    #[test]
    fn persistent_failure_and_cancellation_do_not_loop_or_replay() {
        for cancel in [false, true] {
            let attempts = Cell::new(0);
            let cancelled = Cell::new(false);
            let result: Result<(), _> = complete_with_wait(
                &|| cancelled.get(),
                || {
                    attempts.set(attempts.get() + 1);
                    Err("unavailable".into())
                },
                || cancelled.set(cancel),
            );
            assert_eq!(result.unwrap_err(), "unavailable");
            assert_eq!(attempts.get(), if cancel { 1 } else { 2 });
        }
    }

    #[test]
    fn panic_after_commit_still_reconciles_the_existing_result() {
        let committed = Cell::new(false);
        let result = complete_with_wait(
            &|| false,
            || {
                if committed.replace(true) {
                    return Ok(7);
                }
                panic!("synthetic response loss after commit");
            },
            || {},
        );
        assert_eq!(result.unwrap(), 7);
    }
}
