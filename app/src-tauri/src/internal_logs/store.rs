use super::*;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

const SCAN_BYTES: u64 = 8 * 1024 * 1024;
struct Segment {
    path: PathBuf,
    bytes: u64,
    created: i64,
    first: u64,
    last: u64,
}
pub(super) struct Store {
    root: PathBuf,
    segments: Vec<Segment>,
    current: Option<File>,
    sequence: u64,
    session: String,
    status: Status,
    maximum: u64,
    retention: i64,
}
fn root_marker(root: &Path) -> bool {
    root.join("history-pruned")
        .symlink_metadata()
        .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
}
fn owned_name(name: &str) -> Option<i64> {
    let rest = name.strip_prefix("mon-log-")?.strip_suffix(".jsonl")?;
    let (time, id) = rest.split_once('-')?;
    (time.len() == 13 && uuid::Uuid::parse_str(id).ok()?.to_string() == id)
        .then(|| time.parse().ok())
        .flatten()
}
pub(super) fn open_read(path: &Path) -> Result<File, String> {
    let metadata = path.symlink_metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("ログが通常ファイルではありません".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("ログのリンクは読み取れません".into());
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let after = file.metadata().map_err(|e| e.to_string())?;
        if after.dev() != metadata.dev() || after.ino() != metadata.ino() {
            return Err("ログファイルが変更されました".into());
        }
    }
    Ok(file)
}
fn entries(path: &Path) -> Result<(Vec<Entry>, bool), String> {
    let file = open_read(path)?;
    if file.metadata().map_err(|e| e.to_string())?.len() > SEGMENT_BYTES {
        return Err("ログファイルが上限を超えています".into());
    }
    let mut bytes = Vec::new();
    file.take(SEGMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    let mut invalid = false;
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if !line.ends_with(b"\n") {
            invalid = true;
            continue;
        }
        match serde_json::from_slice::<Entry>(line) {
            Ok(entry) => result.push(entry),
            Err(_) => invalid = true,
        }
    }
    Ok((result, invalid))
}
impl Store {
    fn open(root: PathBuf) -> Result<Self, String> {
        if root.exists()
            && root
                .symlink_metadata()
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
        {
            return Err("ログ保存先にリンクは使用できません".into());
        }
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let history_truncated = root_marker(&root);
        let mut store = Self {
            root,
            segments: Vec::new(),
            current: None,
            sequence: 0,
            session: uuid::Uuid::new_v4().to_string(),
            status: Status {
                history_truncated,
                ..Status::default()
            },
            maximum: MAX_BYTES,
            retention: RETENTION_MS,
        };
        for file in std::fs::read_dir(&store.root).map_err(|e| e.to_string())? {
            let file = file.map_err(|e| e.to_string())?;
            let Some(created) = owned_name(&file.file_name().to_string_lossy()) else {
                continue;
            };
            match entries(&file.path()) {
                Ok((records, invalid)) => {
                    if invalid {
                        store.status.storage_error =
                            Some("一部のログが不完全です。読み取れる記録だけ表示します".into());
                    }
                    let first = records.iter().map(|e| e.id).min().unwrap_or(0);
                    let last = records.iter().map(|e| e.id).max().unwrap_or(0);
                    store.sequence = store.sequence.max(last);
                    store.segments.push(Segment {
                        path: file.path(),
                        bytes: file.metadata().map_err(|e| e.to_string())?.len(),
                        created,
                        first,
                        last,
                    });
                }
                Err(error) => {
                    store.status.storage_error = Some(error);
                }
            }
        }
        store.segments.sort_by_key(|s| (s.first, s.created));
        store.prune(chrono::Utc::now().timestamp_millis(), 0)?;
        Ok(store)
    }
    fn prune(&mut self, now: i64, incoming: u64) -> Result<(), String> {
        let mut total = self.segments.iter().map(|s| s.bytes).sum::<u64>();
        while self
            .segments
            .first()
            .is_some_and(|s| s.created <= now - self.retention || total + incoming > self.maximum)
        {
            if self.segments.len() == 1 {
                self.current = None;
            }
            let segment = &self.segments[0];
            // Only remove files this service recognizes and can safely open.
            open_read(&segment.path)?;
            std::fs::remove_file(&segment.path).map_err(|e| e.to_string())?;
            total -= segment.bytes;
            self.segments.remove(0);
            self.status.history_truncated = true;
            let mut marker = OpenOptions::new();
            marker.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                marker.mode(0o600);
            }
            let _ = marker.open(self.root.join("history-pruned"));
        }
        Ok(())
    }
    fn append(&mut self, mut entry: Entry) -> Result<(), String> {
        self.sequence = self.sequence.checked_add(1).ok_or("ログIDの上限です")?;
        entry.id = self.sequence;
        entry.session_id = self.session.clone();
        let mut bytes = serde_json::to_vec(&entry).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        let now = chrono::Utc::now().timestamp_millis();
        if bytes.len() as u64 > SEGMENT_BYTES {
            return Err("ログの記録が大きすぎます".into());
        }
        self.prune(now, bytes.len() as u64)?;
        if self.current.is_none()
            || self.segments.last().is_none_or(|s| {
                s.bytes + bytes.len() as u64 > SEGMENT_BYTES || now - s.created > 60 * 60 * 1000
            })
        {
            self.current = None;
            let path = self
                .root
                .join(format!("mon-log-{now:013}-{}.jsonl", uuid::Uuid::new_v4()));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = options.open(&path).map_err(|e| e.to_string())?;
            self.segments.push(Segment {
                path,
                bytes: 0,
                created: now,
                first: entry.id,
                last: entry.id,
            });
            self.current = Some(file);
        }
        let file = self.current.as_mut().ok_or("ログを開けません")?;
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.flush()) {
            // A partial trailing record is never appended to on retry.
            self.current = None;
            return Err(error.to_string());
        }
        if let Some(segment) = self.segments.last_mut() {
            segment.bytes += bytes.len() as u64;
            segment.last = entry.id;
        }
        Ok(())
    }
    fn query(&mut self, query: &Query) -> Result<Page, String> {
        self.prune(chrono::Utc::now().timestamp_millis(), 0)?;
        let forward = query.after.is_some();
        let limit = query.limit.unwrap_or(200);
        let mut found = Vec::new();
        let mut scanned = 0;
        let mut cursor = query.after.unwrap_or(self.sequence.saturating_add(1));
        let mut has_more = false;
        let segments: Vec<_> = if forward {
            self.segments.iter().collect()
        } else {
            self.segments.iter().rev().collect()
        };
        'scan: for segment in segments {
            if forward && segment.last <= query.after.unwrap_or(0)
                || !forward && query.before.is_some_and(|v| segment.first >= v)
            {
                continue;
            }
            if scanned + segment.bytes > SCAN_BYTES {
                has_more = true;
                break;
            }
            scanned += segment.bytes;
            let (mut records, invalid) = entries(&segment.path)?;
            if invalid {
                self.status.storage_error =
                    Some("一部のログが不完全です。読み取れる記録だけ表示します".into());
            }
            if !forward {
                records.reverse();
            }
            for entry in records {
                if query.after.is_some_and(|v| entry.id <= v)
                    || query.before.is_some_and(|v| entry.id >= v)
                {
                    continue;
                }
                if found.len() >= limit {
                    has_more = true;
                    break 'scan;
                }
                cursor = entry.id;
                if entry.timestamp > chrono::Utc::now().timestamp_millis() - self.retention
                    && query.filter.matches(&entry)
                {
                    found.push(entry);
                }
            }
        }
        if !forward {
            found.reverse();
        }
        let minimum = self
            .segments
            .first()
            .map(|s| s.first)
            .unwrap_or(self.sequence.saturating_add(1));
        Ok(Page {
            entries: found,
            older_cursor: if forward { minimum } else { cursor },
            newer_cursor: if forward && has_more {
                cursor
            } else {
                self.sequence
            },
            has_more,
            expired: query.after.is_some_and(|v| v.saturating_add(1) < minimum),
            status: self.status.clone(),
        })
    }
}
pub(super) fn run(
    root: Result<PathBuf, String>,
    rx: mpsc::Receiver<Command>,
    dropped: Arc<AtomicU64>,
    shared: Arc<Mutex<Status>>,
) {
    let mut store = root.and_then(Store::open);
    let mut last_prune = Instant::now();
    loop {
        let command = match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match command {
            Some(Command::Record(entry)) => {
                if let Ok(store) = &mut store {
                    if let Err(error) = store.append(entry) {
                        store.status.storage_error = Some(format!("ログを保存できません: {error}"));
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            Some(Command::Query(query, reply)) => {
                let result = match &mut store {
                    Ok(store) => store.query(&query).map(|mut page| {
                        page.status.dropped_entries = dropped.load(Ordering::Relaxed);
                        if let Ok(status) = shared.lock() {
                            if status.storage_error.is_some() {
                                page.status.storage_error = status.storage_error.clone();
                            }
                        }
                        page
                    }),
                    Err(error) => Ok(Page {
                        entries: Vec::new(),
                        older_cursor: 0,
                        newer_cursor: 0,
                        has_more: false,
                        expired: false,
                        status: Status {
                            storage_error: Some(format!("ログを保存できません: {error}")),
                            dropped_entries: dropped.load(Ordering::Relaxed),
                            history_truncated: false,
                        },
                    }),
                };
                let _ = reply.send(result);
            }
            Some(Command::Flush(reply)) => {
                if let Ok(store) = &mut store {
                    if let Some(file) = &mut store.current {
                        let _ = file.flush();
                    }
                }
                let _ = reply.send(());
            }
            None => {}
        }
        if last_prune.elapsed() > Duration::from_secs(60) {
            if let Ok(store) = &mut store {
                if let Err(error) = store.prune(chrono::Utc::now().timestamp_millis(), 0) {
                    store.status.storage_error = Some(error);
                }
            }
            last_prune = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(text: &str) -> Entry {
        Entry::new(
            &Context {
                category: "model".into(),
                ..Default::default()
            },
            "process",
            "info",
            "running",
            text,
        )
    }
    #[test]
    fn internal_logs_retention_capacity_and_expired_cursor() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        store.append(entry("old")).unwrap();
        store.current = None;
        store.segments[0].created = chrono::Utc::now().timestamp_millis() - RETENTION_MS - 1;
        store.append(entry("new")).unwrap();
        let page = store
            .query(&Query {
                after: Some(0),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        assert!(page.expired);
        assert!(page.status.history_truncated);
        assert!(
            Store::open(root.path().into())
                .unwrap()
                .status
                .history_truncated
        );
        let size = store.segments[0].bytes;
        store.maximum = size + 10;
        store.append(entry("replacement")).unwrap();
        assert!(store.segments.iter().map(|s| s.bytes).sum::<u64>() <= store.maximum);
        assert_eq!(
            store.query(&Query::default()).unwrap().entries[0].message,
            "replacement"
        );
    }
    #[test]
    fn internal_logs_partial_trailing_record_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        store.append(entry("complete")).unwrap();
        let path = store.segments[0].path.clone();
        drop(store);
        OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(b"{\"id\":2")
            .unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        store.append(entry("after restart")).unwrap();
        let page = store.query(&Query::default()).unwrap();
        assert_eq!(page.entries.len(), 2);
        assert!(page.status.storage_error.is_some());
        assert_eq!(page.entries[1].id, 2);
    }
    #[cfg(unix)]
    #[test]
    fn internal_logs_never_reads_symlinks_or_removes_unknown_files() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let external = other.path().join("private");
        std::fs::write(&external, b"secret").unwrap();
        symlink(
            &external,
            root.path().join(format!(
                "mon-log-1000000000000-{}.jsonl",
                uuid::Uuid::new_v4()
            )),
        )
        .unwrap();
        std::fs::write(root.path().join("unowned.txt"), b"keep").unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        store.append(entry("safe")).unwrap();
        assert_eq!(store.query(&Query::default()).unwrap().entries.len(), 1);
        assert_eq!(std::fs::read(&external).unwrap(), b"secret");
        assert!(root.path().join("unowned.txt").exists());
    }
    #[test]
    fn internal_logs_time_filter_excludes_expired_records_in_recent_segment() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        let mut old = entry("too old");
        old.timestamp -= RETENTION_MS + 1;
        store.append(old).unwrap();
        store.append(entry("current")).unwrap();
        assert_eq!(store.query(&Query::default()).unwrap().entries.len(), 1);
    }
    #[test]
    fn internal_logs_bounded_search_continues_without_skipping_history() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        let text = "x".repeat(16 * 1024);
        for _ in 0..600 {
            store.append(entry(&text)).unwrap();
        }
        let filter = Filter {
            text: Some("absent".into()),
            ..Default::default()
        };
        let newest = store
            .query(&Query {
                filter: filter.clone(),
                ..Default::default()
            })
            .unwrap();
        assert!(newest.entries.is_empty());
        assert!(newest.has_more);
        assert!(newest.older_cursor > 1 && newest.older_cursor < 600);
        let old = store
            .query(&Query {
                filter: filter.clone(),
                before: Some(newest.older_cursor),
                ..Default::default()
            })
            .unwrap();
        assert!(!old.has_more);
        assert_eq!(old.older_cursor, 1);
        let forward = store
            .query(&Query {
                filter: filter.clone(),
                after: Some(0),
                ..Default::default()
            })
            .unwrap();
        assert!(forward.has_more);
        assert!(forward.newer_cursor > 0 && forward.newer_cursor < 600);
        let end = store
            .query(&Query {
                filter,
                after: Some(forward.newer_cursor),
                ..Default::default()
            })
            .unwrap();
        assert!(!end.has_more);
        assert_eq!(end.newer_cursor, 600);
    }
}
