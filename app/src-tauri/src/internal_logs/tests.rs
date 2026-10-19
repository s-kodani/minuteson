use super::*;

fn context() -> Context {
    Context {
        category: "transcription".into(),
        meeting_id: Some("meeting-a".into()),
        job_id: Some("job-a".into()),
        operation_id: None,
    }
}
fn record(message: &str) -> Entry {
    Entry::new(&context(), "process", "info", "running", message)
}

#[test]
fn internal_logs_query_validates_boundaries_and_window_access() {
    assert!(require_main("main").is_ok());
    for window in ["recorder", "recorder-info", "recording-confirmation", ""] {
        assert!(require_main(window).is_err());
    }
    assert!(Query {
        before: Some(3),
        after: Some(1),
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(Query {
        limit: Some(201),
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(Query {
        filter: Filter {
            from: Some(5),
            to: Some(4),
            ..Default::default()
        },
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(serde_json::from_str::<Query>(r#"{"path":"/etc/passwd"}"#).is_err());
}
#[test]
fn internal_logs_concurrent_writes_reopen_and_page_without_duplicates() {
    let root = tempfile::tempdir().unwrap();
    let logs = LogService::start(Ok(root.path().join("logs")));
    let threads: Vec<_> = (0..4)
        .map(|worker| {
            let logs = logs.clone();
            std::thread::spawn(move || {
                for i in 0..20 {
                    logs.record(record(&format!("worker-{worker}-{i}")));
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let page = logs
        .query(Query {
            limit: Some(20),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.entries.len(), 20);
    assert!(page.has_more);
    assert_eq!(page.status.dropped_entries, 0);
    let older = logs
        .query(Query {
            before: Some(page.older_cursor),
            limit: Some(100),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(older.entries.len(), 60);
    assert!(older.entries.last().unwrap().id < page.entries[0].id);
    logs.record(record("after-page"));
    let delta = logs
        .query(Query {
            after: Some(page.newer_cursor),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(delta.entries.len(), 1);
    assert_eq!(delta.entries[0].message, "after-page");
    logs.flush();
    drop(logs);
    let logs = LogService::start(Ok(root.path().join("logs")));
    let reopened = logs.query(Query::default()).unwrap();
    assert_eq!(reopened.entries.len(), 81);
    assert!(reopened
        .entries
        .windows(2)
        .all(|pair| pair[0].id < pair[1].id));
    logs.record(record("new-session"));
    let new = logs
        .query(Query {
            after: Some(reopened.newer_cursor),
            ..Default::default()
        })
        .unwrap();
    assert_ne!(new.entries[0].session_id, reopened.entries[0].session_id);
}
#[test]
fn internal_logs_filters_and_scan_cursor_advance_through_nonmatches() {
    let root = tempfile::tempdir().unwrap();
    let logs = LogService::start(Ok(root.path().into()));
    logs.record(record("Hello world"));
    let mut raw = Entry::new(
        &context(),
        "engine",
        "raw",
        "output",
        "<script>会議本文</script>\n",
    );
    raw.stream = Some("stderr.log".into());
    logs.record(raw);
    let page = logs
        .query(Query {
            filter: Filter {
                kind: Some("engine".into()),
                level: Some("error".into()),
                related_id: Some("job-a".into()),
                text: Some("会議".into()),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].level, "raw");
    assert_eq!(page.entries[0].message, "<script>会議本文</script>\n");
    let empty = logs
        .query(Query {
            filter: Filter {
                text: Some("missing".into()),
                ..Default::default()
            },
            after: Some(0),
            ..Default::default()
        })
        .unwrap();
    assert!(empty.entries.is_empty());
    assert_eq!(empty.newer_cursor, 2);
    let info = logs
        .query(Query {
            filter: Filter {
                kind: Some("process".into()),
                text: Some("HELLO".into()),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    assert_eq!(info.entries.len(), 1);
}
#[test]
fn internal_logs_write_failure_is_visible_without_failing_producers() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    std::fs::write(&path, "not a directory").unwrap();
    let logs = LogService::start(Ok(path));
    logs.record(record("does not panic"));
    let page = logs.query(Query::default()).unwrap();
    assert!(page.status.storage_error.is_some());
    assert_eq!(page.status.dropped_entries, 1);
}
#[test]
fn internal_logs_phase_deduplication_and_long_unicode_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let logs = LogService::start(Ok(root.path().into()));
    for _ in 0..50 {
        logs.transition(&context(), "running", "info", "starting");
    }
    logs.transition(&context(), "succeeded", "info", "finished");
    let text = "会議🦀".repeat(5000);
    logs.record(record(&text));
    let page = logs.query(Query::default()).unwrap();
    assert_eq!(
        page.entries
            .iter()
            .filter(|e| e.state == "succeeded")
            .count(),
        1
    );
    assert_eq!(
        page.entries
            .iter()
            .filter(|e| e.message == "starting")
            .count(),
        1
    );
    assert_eq!(
        page.entries
            .iter()
            .filter(|e| e.state == "running" && e.message != "starting")
            .map(|e| e.message.as_str())
            .collect::<String>(),
        text
    );
}

#[test]
fn internal_logs_saturated_queue_drops_without_waiting() {
    let (tx, _rx) = mpsc::sync_channel(1);
    let logs = LogService {
        tx,
        dropped: Arc::new(AtomicU64::new(0)),
        status: Arc::new(Mutex::new(Status::default())),
        transitions: Arc::new(Mutex::new(Default::default())),
    };
    logs.record(record("first"));
    let started = Instant::now();
    for _ in 0..100 {
        logs.record(record("overflow"));
    }
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(logs.dropped.load(Ordering::Relaxed), 100);
    assert!(logs.query(Query::default()).is_err());
}

#[test]
fn completion_details_preserve_each_decision_elapsed_and_filter_context() {
    let root = tempfile::tempdir().unwrap();
    let logs = LogService::start(Ok(root.path().into()));
    let context = Context {
        category: "speaker_completion".into(),
        ..context()
    };
    let message = serde_json::json!({"event":"発言照合完了","decision":{"lineage":"u1","reason":"below_threshold","similarity":0.79,"runnerUp":0.7},"margin":0.09}).to_string();
    // Detailed records must not deduplicate distinct units even with identical text.
    logs.detail(
        &context,
        "running",
        "info",
        Duration::from_millis(1234),
        &message,
    );
    logs.detail(
        &context,
        "running",
        "info",
        Duration::from_millis(1500),
        &message,
    );
    logs.flush();
    let page = logs
        .query(Query {
            filter: Filter {
                category: Some("speaker_completion".into()),
                related_id: Some("job-a".into()),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries[0].elapsed_ms, Some(1234));
    assert_eq!(page.entries[1].elapsed_ms, Some(1500));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&page.entries[0].message).unwrap()["decision"]
            ["reason"],
        "below_threshold"
    );
}
