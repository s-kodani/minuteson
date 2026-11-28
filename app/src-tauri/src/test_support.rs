//! Shared inputs for native unit tests.

/// Reuse one function item across cancellation-free test calls so generic
/// storage and processing paths do not specialize for every `|| false` closure.
pub(crate) fn never_cancelled() -> bool {
    false
}

/// One second of canonical audio for protocol-only CLI tests.
pub(crate) fn write_wave(path: &std::path::Path) {
    let mut bytes = crate::audio::wave::header(16_000).unwrap();
    bytes.resize(bytes.len() + 32_000, 0);
    std::fs::write(path, bytes).unwrap();
}
/// Absolute fixture identity without assuming a Unix root on Windows.
pub(crate) fn absolute(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{path}")
    } else {
        path.into()
    }
}

#[cfg(target_os = "macos")]
pub(crate) mod relocation_identity {
    use std::cell::Cell;

    thread_local! {
        static CHANGE: Cell<Option<(&'static str, bool)>> = const { Cell::new(None) };
    }

    /// 起動時に読んだ履歴の証拠をテスト内だけで差し替える。保存済み記録は保持する。
    pub(crate) fn with_change<T>(
        scope: &'static str,
        device_only: bool,
        test: impl FnOnce() -> T,
    ) -> T {
        struct Restore(Option<(&'static str, bool)>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CHANGE.set(self.0);
            }
        }
        let _restore = Restore(CHANGE.replace(Some((scope, device_only))));
        test()
    }

    pub(crate) fn perturb(scope: &str, device: &mut u64, inode: &mut u64) {
        if let Some((selected, device_only)) = CHANGE.get() {
            if selected == scope {
                if device_only {
                    *device ^= 1;
                } else {
                    *inode ^= 1;
                }
            }
        }
    }
}
