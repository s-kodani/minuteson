//! Follow owned diagnostic descriptors. Never re-open a replaced engine path.
use super::*;
use std::fs::File;
use std::io::Read;
use std::sync::atomic::AtomicBool;

pub(crate) struct EngineLogs {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for EngineLogs {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
struct Tail {
    file: File,
    stream: String,
    pending: Vec<u8>,
    source: String,
}
impl Tail {
    fn drain(&mut self, service: &LogService, context: &Context, final_read: bool) {
        let mut bytes = [0; MAX_MESSAGE];
        // Bound one poll; normal engine limits remain the source of truth.
        for _ in 0..if final_read { 1024 } else { 64 } {
            match self.file.read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => {
                    self.pending.extend_from_slice(&bytes[..count]);
                    self.emit(service, context, false);
                }
                Err(error) => {
                    service.error(format!("エンジンログを読み取れません: {error}"));
                    break;
                }
            }
        }
        if final_read {
            self.emit(service, context, true);
        }
    }
    fn emit(&mut self, service: &LogService, context: &Context, final_read: bool) {
        let length = if final_read {
            self.pending.len()
        } else {
            match std::str::from_utf8(&self.pending) {
                Ok(_) => self.pending.len(),
                Err(error) if error.error_len().is_none() => error.valid_up_to(),
                Err(_) => self.pending.len(),
            }
        };
        if length == 0 {
            return;
        }
        let mut entry = Entry::new(
            context,
            "engine",
            "raw",
            "output",
            &String::from_utf8_lossy(&self.pending[..length]),
        );
        entry.stream = Some(self.stream.clone());
        entry.source = Some(self.source.clone());
        service.record(entry);
        self.pending.drain(..length);
    }
}
pub(crate) fn watch_engine(directory: &Path, names: &[&str], category: &str) -> Option<EngineLogs> {
    let service = GLOBAL.get()?.clone();
    let context = CONTEXT
        .with(|current| current.borrow().clone())
        .unwrap_or_else(|| Context {
            category: category.into(),
            ..Context::default()
        });
    watch(service, context, directory, names)
}
fn watch(
    service: LogService,
    context: Context,
    directory: &Path,
    names: &[&str],
) -> Option<EngineLogs> {
    let mut tails = Vec::new();
    for name in names {
        // Callers supply fixed diagnostic filenames, never generated results.
        if !matches!(*name, "stdout.log" | "stderr.log" | "embedding.stderr.log") {
            continue;
        }
        match super::store::open_read(&directory.join(name)) {
            Ok(file) => tails.push(Tail {
                file,
                stream: (*name).into(),
                pending: Vec::new(),
                source: directory.to_string_lossy().into_owned(),
            }),
            Err(error) => service.error(format!("エンジンログを収集できません: {error}")),
        }
    }
    if tails.is_empty() {
        return None;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let errors = service.clone();
    match std::thread::Builder::new()
        .name("mon-engine-logs".into())
        .spawn(move || loop {
            let finished = stopped.load(Ordering::Acquire);
            for tail in &mut tails {
                tail.drain(&service, &context, finished);
            }
            if finished {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }) {
        Ok(worker) => Some(EngineLogs {
            stop,
            worker: Some(worker),
        }),
        Err(error) => {
            errors.error(format!("エンジンログを収集できません: {error}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn internal_logs_engine_live_utf8_final_output_and_descriptor_identity() {
        let root = tempfile::tempdir().unwrap();
        let logs = LogService::start(Ok(root.path().join("logs")));
        let path = root.path().join("stderr.log");
        let mut output = File::create(&path).unwrap();
        let mut tail = Tail {
            file: File::open(&path).unwrap(),
            stream: "stderr.log".into(),
            pending: Vec::new(),
            source: root.path().to_string_lossy().into_owned(),
        };
        let context = Context {
            category: "minutes_generation".into(),
            job_id: Some("job-1".into()),
            ..Default::default()
        };
        let text = "行の途中🦀".as_bytes();
        let split = text.len() - 2;
        output.write_all(&text[..split]).unwrap();
        tail.drain(&logs, &context, false);
        output.write_all(&text[split..]).unwrap();
        output.write_all(b"\nlast without newline").unwrap();
        std::fs::rename(&path, root.path().join("original.log")).unwrap();
        std::fs::write(&path, "unrelated replacement").unwrap();
        tail.drain(&logs, &context, true);
        let page = logs.query(Query::default()).unwrap();
        assert_eq!(
            page.entries
                .iter()
                .map(|e| e.message.as_str())
                .collect::<String>(),
            "行の途中🦀\nlast without newline"
        );
        assert!(page
            .entries
            .iter()
            .all(|e| e.job_id.as_deref() == Some("job-1") && e.level == "raw"));
    }
    #[test]
    fn internal_logs_engine_guard_drains_on_failure_and_cancel() {
        let root = tempfile::tempdir().unwrap();
        let logs = LogService::start(Ok(root.path().join("logs")));
        let mut output = File::create(root.path().join("stderr.log")).unwrap();
        let guard = watch(
            logs.clone(),
            Context {
                category: "transcription".into(),
                ..Default::default()
            },
            root.path(),
            &["stderr.log"],
        )
        .unwrap();
        output.write_all("最後のエラー\n".as_bytes()).unwrap();
        drop(guard);
        let page = logs.query(Query::default()).unwrap();
        assert_eq!(
            page.entries
                .iter()
                .map(|e| e.message.as_str())
                .collect::<String>(),
            "最後のエラー\n"
        );
        assert!(watch(
            logs,
            Context::default(),
            root.path(),
            &["result.json", "prompt.txt"]
        )
        .is_none());
    }
}
