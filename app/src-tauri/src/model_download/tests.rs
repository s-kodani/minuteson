use super::*;
use serde_json::{json, Value};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const BIN: &[u8] = b"not-an-executable";
const MODEL: &[u8] = b"not-a-real-model";
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn recipe_value() -> Value {
    json!({"schemaVersion":1,"manifest":{
        "schemaVersion":1,"engine":"whisper.cpp","engineVersion":"test",
        "modelName":"synthetic","modelVersion":"1","quantization":null,
        "licenseRef":"synthetic fixture only","target":"macos_aarch64",
        "files":[
            {"relativePath":"bin/whisper","role":"executable","expectedSize":BIN.len(),"expectedHash":hash(BIN)},
            {"relativePath":"models/model.gguf","role":"asr","expectedSize":MODEL.len(),"expectedHash":hash(MODEL)}
        ]},"sources":[
        {"relativePath":"bin/whisper","url":"https://example.com/bin"},
        {"relativePath":"models/model.gguf","url":"https://example.com/model"}
    ]})
}
fn recipe() -> Recipe {
    Recipe::parse(&serde_json::to_vec(&recipe_value()).unwrap()).unwrap()
}
fn private_root(temp: &tempfile::TempDir) -> PathBuf {
    temp.path().canonicalize().unwrap().join("downloads")
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// IO-only staging fixture: uses the production durable pipeline and physical
// hashes, but no socket, external download or executable model.
fn synthetic_stage(root: &Path, operation: &str) -> DownloadReceipt {
    let recipe = recipe();
    stage(
        root,
        operation,
        &recipe,
        &recipe.hash().unwrap(),
        &|| false,
        &mut |_, _| {},
        &mut |url, _, _, output, _, notify| {
            let body = if url.ends_with("/bin") { BIN } else { MODEL };
            output.write_all(body).unwrap();
            notify(body.len() as u64);
            Ok(())
        },
    )
    .unwrap()
}
fn read(root: &Path, operation: &str) -> Result<Readback, String> {
    readback(root, operation, &recipe().hash().unwrap(), &|| false)
}

#[test]
fn readback_repeated_after_lost_response_is_physical_read_only_and_never_ready() {
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    let receipt = synthetic_stage(&root, &operation);
    let intent = root.join(&operation).join(INTENT);
    let before = std::fs::read(&intent).unwrap();
    let modified = intent.metadata().unwrap().modified().unwrap();
    for _ in 0..3 {
        let recovered = read(&root, &operation).unwrap();
        assert_eq!(recovered.state, "complete");
        assert_eq!(recovered.review.recipe_hash, receipt.recipe_hash);
        let recovered = recovered.receipt.unwrap();
        assert_eq!(recovered.report.review_hash, receipt.report.review_hash);
        assert!(!recovered.ready && !recovered.installed);
    }
    assert_eq!(before, std::fs::read(&intent).unwrap());
    assert_eq!(modified, intent.metadata().unwrap().modified().unwrap());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn readback_missing_marker_preserves_partial_and_does_not_create_missing_or_unknown_operations() {
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    assert!(read(&root, &operation).is_err());
    assert!(!root.exists());
    let recipe = recipe();
    assert!(stage(
        &root,
        &operation,
        &recipe,
        &recipe.hash().unwrap(),
        &|| false,
        &mut |_, _| {},
        &mut |_, _, _, file, _, _| {
            file.write_all(b"part").unwrap();
            Err("offline".into())
        }
    )
    .is_err());
    let bytes = std::fs::read(root.join(&operation).join("payload/bin/whisper")).unwrap();
    let recovered = read(&root, &operation).unwrap();
    assert_eq!(recovered.state, "incomplete");
    assert!(recovered.receipt.is_none());
    assert_eq!(
        bytes,
        std::fs::read(root.join(&operation).join("payload/bin/whisper")).unwrap()
    );
    let unknown = id();
    std::fs::create_dir(root.join(&unknown)).unwrap();
    std::fs::write(root.join(&unknown).join("keep"), b"foreign").unwrap();
    assert!(read(&root, &unknown).is_err());
    assert_eq!(
        std::fs::read(root.join(unknown).join("keep")).unwrap(),
        b"foreign"
    );
}

#[test]
fn readback_rejects_changed_intent_and_completion_not_empty_or_success() {
    for (name, field, value) in [
        (INTENT, "schemaVersion", json!(2)),
        (INTENT, "operationId", json!(id())),
        (INTENT, "recipeHash", json!("a".repeat(64))),
        (INTENT, "unknown", json!(true)),
        (COMPLETE, "schemaVersion", json!(2)),
        (COMPLETE, "operationId", json!(id())),
        (COMPLETE, "recipeHash", json!("a".repeat(64))),
        (COMPLETE, "reviewHash", json!("a".repeat(64))),
        (COMPLETE, "totalBytes", json!(1)),
        (COMPLETE, "unknown", json!(true)),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = private_root(&temp);
        let operation = id();
        synthetic_stage(&root, &operation);
        let file = root.join(&operation).join(name);
        let mut record: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        record[field] = value;
        let altered = serde_json::to_vec(&record).unwrap();
        std::fs::write(&file, &altered).unwrap();
        assert!(read(&root, &operation).is_err(), "{name}/{field}");
        assert_eq!(std::fs::read(file).unwrap(), altered);
    }
}

#[test]
fn readback_rejects_truncated_oversized_markers_changed_models_and_relabelled_manifest() {
    for name in [
        INTENT,
        COMPLETE,
        "payload/bin/whisper",
        "payload/models/model.gguf",
        "payload/model-package.json",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = private_root(&temp);
        let operation = id();
        let receipt = synthetic_stage(&root, &operation);
        let file = if name.ends_with("model-package.json") {
            receipt.directory.join(model_inspection::MANIFEST)
        } else {
            root.join(&operation).join(name)
        };
        let bytes = if name.ends_with("model-package.json") {
            let mut manifest = recipe().manifest;
            manifest.license_ref = "changed license".into();
            serde_json::to_vec(&manifest).unwrap()
        } else {
            b"broken".to_vec()
        };
        std::fs::write(&file, &bytes).unwrap();
        assert!(read(&root, &operation).is_err(), "{name}");
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
    }
    for name in [INTENT, COMPLETE] {
        let temp = tempfile::tempdir().unwrap();
        let root = private_root(&temp);
        let operation = id();
        synthetic_stage(&root, &operation);
        let file = root.join(&operation).join(name);
        std::fs::write(&file, vec![b' '; MAX_RECIPE + 1025]).unwrap();
        assert!(read(&root, &operation).is_err());
    }
}

#[test]
fn readback_rejects_links_wrong_confirmation_cancel_and_parent_substitution() {
    use std::os::unix::fs::symlink;
    for relative in [INTENT, COMPLETE, "payload", "payload/bin/whisper"] {
        let temp = tempfile::tempdir().unwrap();
        let root = private_root(&temp);
        let operation = id();
        synthetic_stage(&root, &operation);
        let path = root.join(&operation).join(relative);
        let saved = temp.path().join("saved");
        std::fs::rename(&path, &saved).unwrap();
        symlink(&saved, &path).unwrap();
        assert!(read(&root, &operation).is_err(), "{relative}");
        assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
    }
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    synthetic_stage(&root, &operation);
    assert!(readback(&root, &operation, &"a".repeat(64), &|| false).is_err());
    assert!(readback(&root, &operation, &recipe().hash().unwrap(), &|| true).is_err());
    assert!(read(&root, "../outside").is_err());
    let recovered = read(&root, &operation).unwrap();
    std::fs::rename(root.join(&operation), root.join("saved")).unwrap();
    symlink(root.join("saved"), root.join(&operation)).unwrap();
    assert!(recovered.verify().is_err());
}

#[test]
fn downloaded_placement_is_distinct_idempotent_private_and_persistent_without_execution() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    let receipt = synthetic_stage(&root, &operation);
    let meetings = tempfile::tempdir().unwrap();
    let storage = crate::storage::Storage::open(meetings.path().into()).unwrap();
    storage.create_meeting("retain").unwrap();
    let before = storage.analysis_preferences().unwrap().payload;
    let models = temp.path().canonicalize().unwrap().join("models");
    let package = id();
    for _ in 0..2 {
        let placed = place(
            &storage,
            &models,
            &root,
            &operation,
            &receipt.recipe_hash,
            &package,
            &receipt.report.review_hash,
            &|| false,
        )
        .unwrap();
        assert!(placed.placement_recorded && !placed.ready);
    }
    assert_eq!(std::fs::read_dir(&models).unwrap().count(), 1);
    assert_eq!(storage.analysis_preferences().unwrap().payload, before);
    let row = storage.list_model_placements().unwrap().remove(0);
    assert_eq!(row.source_type, "download");
    assert!(row.placement_recorded && !row.ready && row.compatibility.is_none());
    assert_eq!(
        models
            .join(&package)
            .join("bin/whisper")
            .metadata()
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        receipt
            .directory
            .join("bin/whisper")
            .metadata()
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(storage);
    let db = rusqlite::Connection::open(meetings.path().join("meetings.sqlite3")).unwrap();
    let json: String = db
        .query_row(
            "SELECT manifest_json FROM model_packages WHERE id=?1",
            [&package],
            |r| r.get(0),
        )
        .unwrap();
    let record: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(record["schemaVersion"], 2);
    assert_eq!(record["download"]["operationId"], operation);
    assert_eq!(record["download"]["recipeHash"], receipt.recipe_hash);
    assert_eq!(record["download"]["sources"], recipe_value()["sources"]);
    drop(db);
    let reopened = crate::storage::Storage::open(meetings.path().into()).unwrap();
    assert_eq!(
        reopened.list_model_placements().unwrap()[0].source_type,
        "download"
    );
    assert_eq!(reopened.list_meetings().unwrap().len(), 1);
    assert!(reopened.prepare_backup_snapshot(&|| false).is_ok());
}

#[test]
fn incomplete_download_or_stale_review_never_creates_placement_intent() {
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    let receipt = synthetic_stage(&root, &operation);
    let meetings = tempfile::tempdir().unwrap();
    let storage = crate::storage::Storage::open(meetings.path().into()).unwrap();
    let models = temp.path().canonicalize().unwrap().join("models");
    assert!(place(
        &storage,
        &models,
        &root,
        &operation,
        &receipt.recipe_hash,
        &id(),
        &"a".repeat(64),
        &|| false
    )
    .is_err());
    // Simulate a missing marker after process interruption, not a resume.
    std::fs::rename(
        root.join(&operation).join(COMPLETE),
        root.join(&operation).join("saved-marker"),
    )
    .unwrap();
    assert!(place(
        &storage,
        &models,
        &root,
        &operation,
        &receipt.recipe_hash,
        &id(),
        &receipt.report.review_hash,
        &|| false
    )
    .is_err());
    assert!(!models.exists());
    assert!(storage.list_model_placements().unwrap().is_empty());
}

#[test]
fn download_record_rejects_origin_schema_hash_and_database_source_relabel() {
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    let receipt = synthetic_stage(&root, &operation);
    let record = crate::model_placement::PlacementRecord::downloaded(
        &id(),
        &receipt.report,
        recipe().origin(&operation).unwrap(),
    )
    .unwrap();
    for field in [
        "schema", "missing", "hash", "id", "url", "unknown", "manifest",
    ] {
        let mut value = serde_json::to_value(&record).unwrap();
        match field {
            "schema" => value["schemaVersion"] = json!(1),
            "missing" => {
                value.as_object_mut().unwrap().remove("download");
            }
            "hash" => value["download"]["recipeHash"] = json!("a".repeat(64)),
            "id" => value["download"]["operationId"] = json!("../x"),
            "url" => {
                value["download"]["sources"][0]["url"] = json!("https://user:secret@example.com/x")
            }
            "unknown" => value["download"]["ready"] = json!(true),
            _ => value["sourceManifestHash"] = json!("a".repeat(64)),
        }
        let decoded = serde_json::from_value::<crate::model_placement::PlacementRecord>(value);
        assert!(
            decoded.is_err() || decoded.unwrap().validate_portable().is_err(),
            "{field}"
        );
    }
    let meetings = tempfile::tempdir().unwrap();
    let storage = crate::storage::Storage::open(meetings.path().into()).unwrap();
    storage.begin_model_placement(&record).unwrap();
    storage.interrupt_model_placement(&record).unwrap();
    // Storage owns its SQLite locking mode. Inject corruption only after
    // closing that connection, then exercise the cold native readers.
    drop(storage);
    let db = rusqlite::Connection::open(meetings.path().join("meetings.sqlite3")).unwrap();
    db.execute(
        "UPDATE model_packages SET source_type='manual_import' WHERE id=?1",
        [&record.package_id],
    )
    .unwrap();
    drop(db);
    let storage = crate::storage::Storage::open(meetings.path().into()).unwrap();
    assert!(storage.list_model_placements().is_err());
    assert!(storage.prepare_backup_snapshot(&|| false).is_err());
}

#[test]
fn interrupted_downloaded_copy_remains_unavailable_after_restart_and_preserves_partial() {
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    let receipt = synthetic_stage(&root, &operation);
    let meetings = tempfile::tempdir().unwrap();
    let storage = crate::storage::Storage::open(meetings.path().into()).unwrap();
    let models = temp.path().canonicalize().unwrap().join("models");
    let package = id();
    let file = models.join(&package).join("bin/whisper");
    assert!(place(
        &storage,
        &models,
        &root,
        &operation,
        &receipt.recipe_hash,
        &package,
        &receipt.report.review_hash,
        &|| file.metadata().is_ok_and(|m| m.len() > 0)
    )
    .is_err());
    let bytes = std::fs::read(&file).unwrap();
    drop(storage);
    let reopened = crate::storage::Storage::open(meetings.path().into()).unwrap();
    let entry = &reopened.list_model_placements().unwrap()[0];
    assert_eq!(entry.source_type, "download");
    assert!(!entry.placement_recorded && !entry.ready);
    assert!(place(
        &reopened,
        &models,
        &root,
        &operation,
        &receipt.recipe_hash,
        &package,
        &receipt.report.review_hash,
        &|| false
    )
    .is_err());
    assert_eq!(bytes, std::fs::read(file).unwrap());
}

#[test]
fn downloaded_copy_rechecks_intent_evidence_before_committing() {
    use std::cell::Cell;
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    let receipt = synthetic_stage(&root, &operation);
    let meetings = tempfile::tempdir().unwrap();
    let storage = crate::storage::Storage::open(meetings.path().into()).unwrap();
    let models = temp.path().canonicalize().unwrap().join("models");
    let package = id();
    let changed = Cell::new(false);
    let result = place(
        &storage,
        &models,
        &root,
        &operation,
        &receipt.recipe_hash,
        &package,
        &receipt.report.review_hash,
        &|| {
            if !changed.get()
                && models
                    .join(&package)
                    .join("bin/whisper")
                    .metadata()
                    .is_ok_and(|m| m.len() > 0)
            {
                std::fs::write(root.join(&operation).join(INTENT), b"changed during copy").unwrap();
                changed.set(true);
            }
            false
        },
    );
    assert!(changed.get() && result.is_err());
    let row = &storage.list_model_placements().unwrap()[0];
    assert_eq!(row.source_type, "download");
    assert!(!row.placement_recorded && !row.ready);
    assert!(models.join(&package).exists());
}

// Only loopback, tiny synthetic bodies, and this owned fixture thread. Drop
// stops/joins even after assertion failure. No external network or user model.
struct Server {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(handler: impl Fn(&str, &mut TcpStream, &AtomicBool) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let stop_thread = stop.clone();
        let requests_thread = requests.clone();
        let thread = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !stop_thread.load(Ordering::SeqCst) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_millis(500)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_millis(500)))
                            .unwrap();
                        let mut request = Vec::new();
                        let mut buffer = [0_u8; 256];
                        while request.len() < 4096 && !request.windows(4).any(|b| b == b"\r\n\r\n")
                        {
                            match stream.read(&mut buffer) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => request.extend_from_slice(&buffer[..n]),
                            }
                        }
                        requests_thread.fetch_add(1, Ordering::SeqCst);
                        handler(
                            &String::from_utf8_lossy(&request),
                            &mut stream,
                            &stop_thread,
                        );
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            stop,
            requests,
            thread: Some(thread),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }
    fn package() -> Self {
        Self::new(|request, stream, _| {
            let body = if request.starts_with("GET /bin ") {
                BIN
            } else {
                MODEL
            };
            response(stream, "200 OK", "", body);
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn response(stream: &mut TcpStream, status: &str, extra: &str, body: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}
fn stage_fixture(
    root: &Path,
    operation: &str,
    recipe: &Recipe,
    server: &Server,
    cancelled: &impl Fn() -> bool,
    progress: &mut impl FnMut(u64, u64),
) -> Result<DownloadReceipt, String> {
    stage(
        root,
        operation,
        recipe,
        &recipe.hash().unwrap(),
        cancelled,
        progress,
        &mut |url, size, hash, output, cancel, notify| {
            let path = tauri::Url::parse(url).unwrap().path().to_string();
            transport::fixture_fetch(
                &server.url(&path),
                size,
                hash,
                output,
                cancel,
                notify,
                Duration::from_secs(5),
            )
        },
    )
}
fn direct_fetch(server: &Server, size: u64, hash: &str) -> (Result<(), String>, Vec<u8>) {
    let mut output = tempfile::tempfile().unwrap();
    let result = transport::fixture_fetch(
        &server.url("/bin"),
        size,
        hash,
        &mut output,
        &|| false,
        &mut |_| {},
        Duration::from_secs(3),
    );
    use std::io::{Seek, SeekFrom};
    output.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    output.read_to_end(&mut bytes).unwrap();
    (result, bytes)
}

#[test]
fn recipe_review_binds_sources_sizes_hashes_license_and_target_without_readiness() {
    let recipe = recipe();
    let review = recipe.review().unwrap();
    assert_eq!(review.total_bytes, (BIN.len() + MODEL.len()) as u64);
    assert!(!review.installed && !review.ready);
    assert_eq!(review.publisher_authenticity, "not_verified");
    assert_eq!(
        review.recipe_hash,
        Recipe::parse(&serde_json::to_vec_pretty(&recipe).unwrap())
            .unwrap()
            .hash()
            .unwrap()
    );
    for field in ["url", "expectedHash", "licenseRef", "modelVersion"] {
        let mut altered = recipe_value();
        match field {
            "url" => altered["sources"][0][field] = json!("https://other.example.com/bin"),
            "expectedHash" => altered["manifest"]["files"][0][field] = json!("a".repeat(64)),
            _ => altered["manifest"][field] = json!("changed"),
        }
        assert_ne!(
            review.recipe_hash,
            Recipe::parse(&serde_json::to_vec(&altered).unwrap())
                .unwrap()
                .hash()
                .unwrap()
        );
    }
}

#[test]
fn recipe_rejects_unknown_schema_fields_paths_targets_and_missing_or_reordered_sources() {
    let mut cases = Vec::new();
    let mut add = |change: fn(&mut Value)| {
        let mut value = recipe_value();
        change(&mut value);
        cases.push(value);
    };
    add(|v| v["schemaVersion"] = json!(2));
    add(|v| v["extra"] = json!(true));
    add(|v| v["sources"][0]["headers"] = json!({"Authorization":"secret"}));
    add(|v| v["manifest"]["target"] = json!("windows_x86_64"));
    add(|v| v["manifest"]["files"][0]["relativePath"] = json!("../escape"));
    add(|v| v["sources"][0]["relativePath"] = json!("bin/other"));
    add(|v| v["sources"].as_array_mut().unwrap().reverse());
    add(|v| {
        v["sources"].as_array_mut().unwrap().pop();
    });
    add(|v| v["manifest"]["files"][0]["expectedSize"] = Value::Null);
    add(|v| v["manifest"]["files"][0]["expectedHash"] = Value::Null);
    add(|v| v["manifest"]["files"][0]["expectedHash"] = json!("A".repeat(64)));
    add(|v| v["manifest"]["files"][0]["expectedSize"] = json!(MAX_TOTAL));
    for value in cases {
        assert!(Recipe::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    assert!(Recipe::parse(&[]).is_err());
    assert!(Recipe::parse(&vec![b' '; MAX_RECIPE + 1]).is_err());
}

#[test]
fn recipe_rejects_plaintext_credentials_tokens_fragments_and_normalized_unsafe_urls() {
    for url in [
        "http://example.com/file",
        "file:///etc/passwd",
        "https://user:secret@example.com/file",
        "https://@example.com/file",
        "https://example.com/file?token=secret",
        "https://example.com/file#x",
        " https://example.com/file",
        "https://example.com\\file",
        "https://example.com/\nfile",
        "https:///",
        "--config",
    ] {
        let mut value = recipe_value();
        value["sources"][0]["url"] = json!(url);
        assert!(
            Recipe::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
            "accepted {url:?}"
        );
    }
}

#[test]
fn system_transport_ignores_config_globs_credentials_and_non_https_protocols() {
    let args = transport::transport_arguments("https://example.com/file");
    assert_eq!(args[0], "--disable");
    for required in [
        "--globoff",
        "--no-buffer",
        "--fail",
        "--location",
        "--proto",
        "--proto-redir",
    ] {
        assert!(args.iter().any(|v| v == required));
    }
    for index in 0..args.len() - 1 {
        if matches!(args[index].as_str(), "--proto" | "--proto-redir") {
            assert_eq!(args[index + 1], "=https");
        }
    }
    for forbidden in [
        "--insecure",
        "--location-trusted",
        "--netrc",
        "--retry",
        "--output",
        "--remote-name",
        "--config",
    ] {
        assert!(!args.iter().any(|v| v == forbidden));
    }
}

#[test]
fn streamed_download_preserves_bound_intent_rechecks_all_files_and_never_installs_or_runs() {
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let recipe = recipe();
    let server = Server::package();
    let operation = id();
    let mut progress = Vec::new();
    let receipt = stage_fixture(
        &root,
        &operation,
        &recipe,
        &server,
        &|| false,
        &mut |n, total| progress.push((n, total)),
    )
    .unwrap();
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    assert_eq!(receipt.recipe_hash, recipe.hash().unwrap());
    assert!(
        !receipt.installed && !receipt.ready && !receipt.report.installed && !receipt.report.ready
    );
    assert!(receipt
        .report
        .files
        .iter()
        .all(|f| f.expected_size_checked && f.expected_hash_checked));
    assert_eq!(
        std::fs::read(receipt.directory.join("bin/whisper")).unwrap(),
        BIN
    );
    assert_eq!(
        std::fs::read(receipt.directory.join("models/model.gguf")).unwrap(),
        MODEL
    );
    let intent: Intent =
        serde_json::from_slice(&std::fs::read(root.join(&operation).join(INTENT)).unwrap())
            .unwrap();
    let completion: Completion =
        serde_json::from_slice(&std::fs::read(root.join(&operation).join(COMPLETE)).unwrap())
            .unwrap();
    assert_eq!(intent.operation_id, operation);
    assert_eq!(intent.recipe_hash, completion.recipe_hash);
    assert_eq!(completion.review_hash, receipt.report.review_hash);
    assert_eq!(progress.first().unwrap().0, 0);
    assert!(progress.windows(2).all(|p| p[0].0 <= p[1].0));
    assert!(progress.iter().all(|(n, total)| *n <= *total));
    use std::os::unix::fs::PermissionsExt;
    // Even the executable role is just inert private staged bytes.
    assert_eq!(
        std::fs::metadata(receipt.directory.join("bin/whisper"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(stage_fixture(
        &root,
        &operation,
        &recipe,
        &server,
        &|| false,
        &mut |_, _| {}
    )
    .is_err());
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    assert_eq!(
        std::fs::read(receipt.directory.join("bin/whisper")).unwrap(),
        BIN
    );
}

#[test]
fn stale_review_invalid_id_and_early_cancel_create_no_operation_or_connection() {
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let recipe = recipe();
    for (operation, expected, cancel) in [
        (id(), "0".repeat(64), false),
        ("../escape".into(), recipe.hash().unwrap(), false),
        (id(), recipe.hash().unwrap(), true),
    ] {
        let result = stage(
            &root,
            &operation,
            &recipe,
            &expected,
            &|| cancel,
            &mut |_, _| {},
            &mut |_, _, _, _, _, _| panic!("must not connect"),
        );
        assert!(result.is_err());
        assert!(!root.exists());
    }
}

#[test]
fn canceled_stream_reaps_owned_child_and_keeps_partial_bytes_without_completion_or_retry() {
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let recipe = recipe();
    let cancel = AtomicBool::new(false);
    let server = Server::new(|_, stream, stop| {
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\nnot-",
            BIN.len()
        );
        while !stop.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(5));
        }
    });
    let operation = id();
    let started = Instant::now();
    let error = stage_fixture(
        &root,
        &operation,
        &recipe,
        &server,
        &|| cancel.load(Ordering::SeqCst),
        &mut |n, _| {
            if n > 0 {
                cancel.store(true, Ordering::SeqCst);
            }
        },
    )
    .unwrap_err();
    assert!(error.contains("中断"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    assert!(root.join(&operation).join(INTENT).is_file());
    assert!(!root.join(&operation).join(COMPLETE).exists());
    assert_eq!(
        std::fs::read(root.join(&operation).join("payload/bin/whisper")).unwrap(),
        b"not-"
    );
}

#[test]
fn truncated_mismatched_excess_and_chunked_excess_bodies_never_exceed_expected_disk_bytes() {
    for kind in ["truncated", "hash", "excess", "chunked"] {
        let server = Server::new(move |_, stream, _| match kind {
            "truncated" => {
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\nConnection: close\r\n\r\nshort",
                );
            }
            "hash" => response(stream, "200 OK", "", &vec![b'x'; BIN.len()]),
            "excess" => response(stream, "200 OK", "", &vec![b'x'; 65536]),
            "chunked" => {
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n20\r\nxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\r\n0\r\n\r\n");
            }
            _ => unreachable!(),
        });
        let (result, bytes) = direct_fetch(&server, BIN.len() as u64, &hash(BIN));
        assert!(result.is_err(), "{kind}");
        assert!(bytes.len() <= BIN.len(), "{kind}");
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn http_errors_redirect_without_location_and_partial_206_are_not_success_even_with_matching_body() {
    for status in [
        "302 Found",
        "404 Not Found",
        "429 Too Many Requests",
        "500 Server Error",
        "206 Partial Content",
    ] {
        let server = Server::new(move |_, stream, _| {
            response(stream, status, "X-Secret: never-log-token\r\n", BIN)
        });
        let (result, _) = direct_fetch(&server, BIN.len() as u64, &hash(BIN));
        let error = result.unwrap_err();
        assert!(!error.contains("never-log-token"));
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn redirect_to_allowed_fixture_protocol_is_bounded_and_hash_verified() {
    let server = Server::new(|request, stream, _| {
        if request.starts_with("GET /bin ") {
            response(
                stream,
                "302 Found",
                "Location: /final\r\n",
                b"redirect body ignored",
            );
        } else {
            response(stream, "200 OK", "", BIN);
        }
    });
    let (result, bytes) = direct_fetch(&server, BIN.len() as u64, &hash(BIN));
    result.unwrap();
    assert_eq!(bytes, BIN);
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    let cycle =
        Server::new(|_, stream, _| response(stream, "302 Found", "Location: /bin\r\n", b""));
    assert!(direct_fetch(&cycle, BIN.len() as u64, &hash(BIN))
        .0
        .is_err());
    assert_eq!(cycle.requests.load(Ordering::SeqCst), 6);
}

#[test]
fn stalled_server_deadline_kills_and_reaps_only_owned_transport() {
    let server = Server::new(|_, _, stop| {
        while !stop.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(5));
        }
    });
    let mut output = tempfile::tempfile().unwrap();
    let started = Instant::now();
    let result = transport::fixture_fetch(
        &server.url("/bin"),
        BIN.len() as u64,
        &hash(BIN),
        &mut output,
        &|| false,
        &mut |_| {},
        Duration::from_millis(150),
    );
    assert!(result.unwrap_err().contains("上限"));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn changed_destination_and_corrupt_fetched_bytes_leave_no_completion_and_preserve_foreign_objects()
{
    for substituted in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = private_root(&temp);
        let recipe = recipe();
        let operation = id();
        let foreign = temp.path().join("foreign");
        std::fs::create_dir(&foreign).unwrap();
        std::fs::write(foreign.join("sentinel"), b"keep").unwrap();
        let result = stage(
            &root,
            &operation,
            &recipe,
            &recipe.hash().unwrap(),
            &|| false,
            &mut |_, _| {},
            &mut |_, size, _, output, _, notify| {
                output.write_all(&vec![b'x'; size as usize]).unwrap();
                if substituted {
                    let payload = root.join(&operation).join("payload");
                    std::fs::rename(&payload, root.join(&operation).join("moved-payload")).unwrap();
                    std::os::unix::fs::symlink(&foreign, &payload).unwrap();
                }
                notify(size);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(!root.join(&operation).join(COMPLETE).exists());
        assert_eq!(std::fs::read(foreign.join("sentinel")).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(&foreign).unwrap().count(), 1);
        assert!(root.join(&operation).join(INTENT).exists());
    }
}

#[test]
fn occupied_operation_and_linked_staging_root_are_not_adopted_or_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let directory = Directory::root(&root).unwrap();
    let operation = id();
    let occupied = directory.child(&operation, true).unwrap();
    durable_file(&occupied, "sentinel", b"keep").unwrap();
    let recipe = recipe();
    let mut never_connect = |_: &str,
                             _: u64,
                             _: &str,
                             _: &mut std::fs::File,
                             _: &dyn Fn() -> bool,
                             _: &mut dyn FnMut(u64)|
     -> Result<(), String> { panic!("must not connect") };
    assert!(stage(
        &root,
        &operation,
        &recipe,
        &recipe.hash().unwrap(),
        &|| false,
        &mut |_, _| {},
        &mut never_connect
    )
    .is_err());
    assert_eq!(
        std::fs::read(occupied.path().join("sentinel")).unwrap(),
        b"keep"
    );
    let linked = temp.path().canonicalize().unwrap().join("linked");
    std::os::unix::fs::symlink(&root, &linked).unwrap();
    assert!(stage(
        &linked,
        &id(),
        &recipe,
        &recipe.hash().unwrap(),
        &|| false,
        &mut |_, _| {},
        &mut never_connect
    )
    .is_err());
}

#[test]
fn altered_manifest_cannot_relabel_verified_download_as_another_model_or_license() {
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let recipe = recipe();
    let operation = id();
    let server = Server::package();
    let mut changed = false;
    let result = stage_fixture(
        &root,
        &operation,
        &recipe,
        &server,
        &|| false,
        &mut |n, total| {
            if n == total && !changed {
                changed = true;
                let mut manifest = serde_json::to_value(&recipe.manifest).unwrap();
                manifest["licenseRef"] = json!("different license");
                std::fs::write(
                    root.join(&operation)
                        .join("payload")
                        .join(model_inspection::MANIFEST),
                    serde_json::to_vec(&manifest).unwrap(),
                )
                .unwrap();
            }
        },
    );
    assert!(result.unwrap_err().contains("manifest"));
    assert!(!root.join(&operation).join(COMPLETE).exists());
}

#[test]
fn unwinding_progress_callback_reaps_owned_transport_and_closes_its_connection() {
    let closed = Arc::new(AtomicBool::new(false));
    let observed = closed.clone();
    let observation = Arc::new(std::sync::Mutex::new(String::from("not observed")));
    let details = observation.clone();
    let server = Server::new(move |_, stream, _| {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\nnot-",
            BIN.len()
        );
        let mut byte = [0_u8; 1];
        let deadline = Instant::now() + Duration::from_secs(2);
        let ended = loop {
            let result = stream.read(&mut byte);
            *details.lock().unwrap() = format!("{result:?}");
            match result {
                Ok(0) => break true,
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => break true,
                Ok(_) => {} // Read through any remaining request bytes to EOF.
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::Interrupted
                            | std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break false,
            }
            if Instant::now() >= deadline {
                break false;
            }
        };
        if ended {
            observed.store(true, Ordering::SeqCst);
        }
    });
    let mut output = tempfile::tempfile().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = transport::fixture_fetch(
            &server.url("/bin"),
            BIN.len() as u64,
            &hash(BIN),
            &mut output,
            &|| false,
            &mut |_| panic!("synthetic progress panic"),
            Duration::from_secs(3),
        );
    }));
    assert!(result.is_err());
    drop(server);
    assert!(
        closed.load(Ordering::SeqCst),
        "{}",
        observation.lock().unwrap()
    );
}
