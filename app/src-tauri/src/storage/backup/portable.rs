//! A logical dump carries source IDs/hashes, never executable SQL or a raw DB.
//! Redaction changes the hash basis; restore must remap IDs and rehash fixed data.
use super::*;
use rusqlite::types::{Value as SqlValue, ValueRef};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, BufWriter};

pub(super) const MAX_ROW_BYTES: usize = 32 * 1024 * 1024;
pub(super) const TABLES: &[&str] = &[
    "meetings",
    "file_assets",
    "recording_sessions",
    "audio_imports",
    "audio_tracks",
    "audio_chunks",
    "track_clock_segments",
    "audio_gaps",
    "track_inputs",
    "analysis_revisions",
    "speakers",
    "speaker_segments",
    "utterances",
    "utterance_speakers",
    "analysis_input_snapshots",
    "minutes_revisions",
    "minutes_items",
    "evidence_links",
    "working_drafts",
    "transcript_edit_recovery",
    "edit_operations",
    "processing_jobs",
    "job_checkpoints",
    "export_records",
    "vocabulary_scopes",
    "vocabulary_entries",
    "vocabulary_snapshots",
    "diarization_manual_assignments",
    "analysis_flows",
    "audio_import_commits",
    "audio_import_options",
    "restore_provenance",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Column {
    pub name: String,
    pub data_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    pub row_count: u64,
}

pub(super) use crate::storage::model_metadata::ModelFileMetadata as FileReference;

#[derive(Debug, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Cell {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    table: String,
    cells: Vec<Cell>,
}

pub(super) fn schema(version: usize) -> Result<Connection, String> {
    initialize(
        Connection::open_in_memory().map_err(|e| e.to_string())?,
        version,
    )
}

pub(super) fn initialize(db: Connection, version: usize) -> Result<Connection, String> {
    if version != MIGRATIONS.len() {
        return Err("未対応のバックアップDB形式です".into());
    }
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA cache_size=-8192; PRAGMA temp_store=FILE;")
        .map_err(|e| e.to_string())?;
    // Use the identical trusted bootstrap SQL as Storage::open. Exact schema
    // comparison at publication must not mistake spacing for a foreign schema.
    db.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL);").map_err(|e|e.to_string())?;
    for &(version, sql) in &MIGRATIONS[..version] {
        db.execute_batch(sql).map_err(|e| e.to_string())?;
        db.execute(
            "INSERT INTO schema_migrations VALUES(?1,?2,0)",
            params![version, format!("{:x}", Sha256::digest(sql.as_bytes()))],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(db)
}

pub(super) fn tables(db: &Connection) -> Result<Vec<Table>, String> {
    let actual: BTreeSet<String> = db
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let expected: BTreeSet<String> = TABLES
        .iter()
        .chain(
            [
                "app_settings",
                "schema_migrations",
                "restore_batches",
                "restore_identity_maps",
                "maintenance_operations",
                "maintenance_paths",
                "restore_omitted_assets",
                "model_compatibility_checks",
                "model_catalog_history",
                "recording_input_delivery",
            ]
            .iter(),
        )
        .chain(crate::storage::model_metadata::TABLES.iter())
        .map(|s| (*s).to_owned())
        .collect();
    if actual != expected {
        return Err("未対応の保存テーブルがあります。バックアップ形式を更新してください".into());
    }
    TABLES
        .iter()
        .map(|&name| {
            let columns = db
                .prepare(&format!("PRAGMA table_info(\"{name}\")"))
                .map_err(|e| e.to_string())?
                .query_map([], |r| {
                    Ok(Column {
                        name: r.get(1)?,
                        data_type: r.get(2)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?;
            Ok(Table {
                name: name.into(),
                columns,
                row_count: 0,
            })
        })
        .collect()
}

pub(super) fn basename(path: &str) -> Result<String, String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or("");
    if name.is_empty() || name.contains('\0') || name.len() > 4096 {
        return Err("ファイル名メタデータが不正です".into());
    }
    Ok(name.to_owned())
}

pub(super) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn redact_json(
    value: &mut Value,
    role: &str,
    references: &mut BTreeSet<FileReference>,
    known: &[FileReference],
    depth: usize,
) -> Result<(), String> {
    if depth > 64 {
        return Err("生成条件の階層が深すぎます".into());
    }
    match value {
        Value::Object(object) => {
            if object.len() == 3
                && object.contains_key("path")
                && object.contains_key("size")
                && object.contains_key("sha256")
            {
                let path = object["path"]
                    .as_str()
                    .ok_or("固定ファイルのパスが不正です")?;
                let size = object["size"]
                    .as_u64()
                    .ok_or("固定ファイルのサイズが不正です")?;
                let hash = object["sha256"]
                    .as_str()
                    .filter(|s| valid_hash(s))
                    .ok_or("固定ファイルのハッシュが不正です")?
                    .to_owned();
                let history: Vec<_> = known
                    .iter()
                    .filter(|r| {
                        path == format!("mon-unavailable:{hash}")
                            && r.sha256 == hash
                            && r.size == size
                            && r.role == role
                    })
                    .cloned()
                    .collect();
                if history.is_empty() {
                    references.insert(FileReference {
                        name: basename(path)?,
                        sha256: hash.clone(),
                        size,
                        role: role.to_owned(),
                    });
                } else {
                    references.extend(history);
                }
                object.insert(
                    "path".into(),
                    Value::String(format!("mon-unavailable:{hash}")),
                );
                return Ok(());
            }
            for (key, item) in object {
                if matches!(
                    key.as_str(),
                    "deviceId"
                        | "deviceUID"
                        | "deviceUid"
                        | "deviceIdentifier"
                        | "targetPID"
                        | "device_id"
                        | "device_uid"
                        | "microphoneDeviceUID"
                        | "microphone_device_uid"
                        | "targetApplicationIdentity"
                        | "target_application_identity"
                        | "gpuDeviceId"
                ) {
                    *item = Value::Null;
                } else if matches!(key.as_str(), "targetPIDs" | "target_pids") {
                    *item = json!([]);
                } else if matches!(key.as_str(), "available" | "isAvailable") {
                    *item = Value::Bool(false);
                } else if key == "availability" {
                    *item = Value::String("unavailable".into());
                } else if matches!(
                    key.as_str(),
                    "localPath"
                        | "local_path"
                        | "executablePath"
                        | "modelPath"
                        | "vadModelPath"
                        | "segmentationModelPath"
                        | "embeddingModelPath"
                        | "outputPath"
                        | "storagePath"
                ) {
                    if item.is_string() {
                        *item = Value::String("mon-unavailable:unfingerprinted".into());
                    }
                } else if matches!(
                    key.as_str(),
                    "managedModel" | "whisperManaged" | "sherpaManaged"
                ) {
                    references.extend(crate::storage::model_profiles::jobs::portable_files(item)?);
                } else if key == "runtimeModel" {
                    let (portable, files) =
                        crate::storage::model_profiles::jobs::rebinding::portable(item)?;
                    *item = portable;
                    references.extend(files);
                } else {
                    redact_json(
                        item,
                        if [
                            "model",
                            "executable",
                            "vadModel",
                            "vad_model",
                            "embedding",
                            "segmentation",
                        ]
                        .contains(&key.as_str())
                        {
                            key
                        } else {
                            role
                        },
                        references,
                        known,
                        depth + 1,
                    )?;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_json(item, role, references, known, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn portable_json(
    bytes: &[u8],
    capture: bool,
    references: &mut BTreeSet<FileReference>,
) -> Result<Vec<u8>, String> {
    portable_json_with_metadata(bytes, capture, references, &[])
}

pub(super) fn portable_json_with_metadata(
    bytes: &[u8],
    capture: bool,
    references: &mut BTreeSet<FileReference>,
    known: &[FileReference],
) -> Result<Vec<u8>, String> {
    if bytes.len() > MAX_ROW_BYTES {
        return Err("バックアップJSONが大きすぎます".into());
    }
    let mut value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if capture {
        let diagnostics = crate::input_status::persistence::portable_manifest_fields(bytes)?;
        let original = value.as_object().ok_or("録音manifestが不正です")?;
        let mut result = serde_json::Map::new();
        for key in [
            "schemaVersion",
            "scenario",
            "captureMode",
            "backend",
            "targetApplicationName",
            "targetBundleIdentifier",
            "startedAtUTC",
            "endedAtUTC",
            "requestedDurationSeconds",
            "captureClock",
            "captureEnd",
            "partialRecovery",
            "recordingOwnerId",
            "osVersion",
            "architecture",
            "state",
            "microphone",
            "system",
        ] {
            if let Some(value) = original.get(key) {
                result.insert(key.into(), value.clone());
            }
        }
        result.insert("targetPIDs".into(), json!([]));
        result.insert("targetPID".into(), Value::Null);
        result.insert("errors".into(), json!([]));
        result.insert(
            "backupRemovedErrorCount".into(),
            original
                .get("backupRemovedErrorCount")
                .cloned()
                .unwrap_or_else(|| {
                    json!(original
                        .get("errors")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len))
                }),
        );
        result.extend(diagnostics);
        value = Value::Object(result);
    }
    // whisper.cpp metadata is machine-generated; never rewrite recognized text.
    if let Some(params) = value.get_mut("params").and_then(Value::as_object_mut) {
        for key in ["model", "fname", "file", "input", "output"] {
            if let Some(text) = params.get(key).and_then(Value::as_str) {
                if text.is_empty() {
                    continue;
                }
                let name = if key == "model" {
                    text.strip_prefix("mon-unavailable:")
                        .filter(|sha| valid_hash(sha))
                        .and_then(|sha| {
                            known
                                .iter()
                                .filter(|r| r.role == "model" && r.sha256 == sha)
                                .map(|r| r.name.as_str())
                                .min()
                        })
                        .map(str::to_owned)
                        .unwrap_or(basename(text)?)
                } else {
                    basename(text)?
                };
                params.insert(key.into(), Value::String(name));
            }
        }
    }
    redact_json(&mut value, "artifact", references, known, 0)?;
    serde_json::to_vec(&value).map_err(|e| e.to_string())
}

pub(super) fn portable_completion(
    bytes: &[u8],
    references: &mut BTreeSet<FileReference>,
) -> Result<Vec<u8>, String> {
    // Native completions permit one exact trailing adapter marker. Use that
    // protocol (including duplicate-key rejection), not a permissive substring
    // search. The original byte hash remains in archive provenance.
    let value = crate::generation::llama::parse_output(bytes)?;
    portable_json(
        &serde_json::to_vec(&value).map_err(|e| e.to_string())?,
        false,
        references,
    )
}

pub(super) fn temporary_pcm(db: &Connection, asset: &FileAsset) -> Result<bool, String> {
    if asset.media_type == "audio/wav" && super::exclusions::matches(db, asset, "recreatable_pcm")?
    {
        return Ok(true);
    }
    private_job_artifact(db, asset, "audio/wav", &["input.wav"])
}

pub(super) fn native_log(db: &Connection, asset: &FileAsset) -> Result<bool, String> {
    if asset.media_type == "text/plain" && super::exclusions::matches(db, asset, "native_log")? {
        return Ok(true);
    }
    private_job_artifact(
        db,
        asset,
        "text/plain",
        &["stdout.log", "stderr.log", "logical-stdout.log"],
    )
}

fn private_job_artifact(
    db: &Connection,
    asset: &FileAsset,
    media: &str,
    names: &[&str],
) -> Result<bool, String> {
    let parts: Vec<_> = asset.relative_path.split('/').collect();
    if asset.media_type != media
        || parts.len() != 6
        || parts[0] != "meetings"
        || parts[1] != asset.meeting_id
        || parts[2] != "analysis"
        || !names.contains(&parts[5])
    {
        return Ok(false);
    }
    let (job, chunk): (bool,bool) = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE id=?1 AND meeting_id=?2),EXISTS(SELECT 1 FROM audio_chunks WHERE file_asset_id=?3) OR EXISTS(SELECT 1 FROM audio_imports WHERE original_asset_id=?3 OR converted_asset_id=?3) OR EXISTS(SELECT 1 FROM audio_import_commits WHERE manifest_asset_id=?3)",
        params![parts[3],asset.meeting_id,asset.id], |r| Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|e| e.to_string())?;
    Ok(job && !chunk)
}

pub(super) fn dump(
    snapshot: &Snapshot<'_>,
    path: &Path,
    assets: &BTreeMap<String, (String, bool)>,
    references: &mut BTreeSet<FileReference>,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<Table>, String> {
    let db = read_only(&snapshot.database)?;
    let known = crate::storage::model_metadata::historical(&db)?;
    let trusted = schema(MIGRATIONS.len())?;
    let mut descriptions = tables(&db)?;
    if descriptions != tables(&trusted)? {
        return Err("DBテーブル定義がバックアップ形式と一致しません".into());
    }
    let mut out = BufWriter::new(private_file(path)?);
    for table in &mut descriptions {
        let mut statement = db
            .prepare(&format!("SELECT * FROM \"{}\" ORDER BY rowid", table.name))
            .map_err(|e| e.to_string())?;
        let mut rows = statement.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let mut cells: Vec<Cell> = (0..table.columns.len())
                .map(
                    |index| match row.get_ref(index).map_err(|e| e.to_string())? {
                        ValueRef::Null => Ok(Cell::Null),
                        ValueRef::Integer(n) => Ok(Cell::Integer(n)),
                        ValueRef::Real(n) if n.is_finite() => Ok(Cell::Real(n)),
                        ValueRef::Text(bytes) if bytes.len() <= MAX_ROW_BYTES => Ok(Cell::Text(
                            std::str::from_utf8(bytes)
                                .map_err(|e| e.to_string())?
                                .to_owned(),
                        )),
                        _ => Err("バックアップで扱えないDB値です".into()),
                    },
                )
                .collect::<Result<_, String>>()?;
            let text = |name: &str| -> Result<String, String> {
                let i = table
                    .columns
                    .iter()
                    .position(|c| c.name == name)
                    .ok_or("バックアップ列がありません")?;
                match &cells[i] {
                    Cell::Text(s) => Ok(s.clone()),
                    _ => Err("バックアップ識別子が不正です".into()),
                }
            };
            let file = if table.name == "file_assets" {
                Some(
                    assets
                        .get(&text("id")?)
                        .ok_or("バックアップ対象ファイルがありません")?
                        .clone(),
                )
            } else {
                None
            };
            let input_snapshot = if table.name == "recording_sessions" {
                snapshot
                    .input_snapshots
                    .payload(&text("meeting_id")?, &text("id")?)
                    .map(str::to_owned)
            } else {
                None
            };
            let input_transfer = if table.name == "recording_sessions" {
                snapshot
                    .input_snapshots
                    .transfer(&text("meeting_id")?, &text("id")?)
                    .map(str::to_owned)
            } else {
                None
            };
            for (column, cell) in table.columns.iter().zip(&mut cells) {
                if table.name == "recording_sessions"
                    && column.name == "input_history_snapshot_json"
                {
                    *cell = input_snapshot.clone().map(Cell::Text).unwrap_or(Cell::Null);
                }
                if table.name == "recording_sessions"
                    && column.name == "input_delivery_transfer_json"
                {
                    *cell = input_transfer.clone().map(Cell::Text).unwrap_or(Cell::Null);
                }
                if let Some((key, omitted)) = &file {
                    if column.name == "relative_path" {
                        *cell = Cell::Text(key.clone());
                    }
                    if column.name == "state" && *omitted {
                        *cell = Cell::Text("missing".into());
                    }
                }
                if table.name == "audio_tracks" && column.name == "device_label" {
                    *cell = Cell::Null;
                }
                if matches!(
                    table.name.as_str(),
                    "export_records" | "audio_import_commits" | "analysis_flows"
                ) && column.name == "error_message"
                {
                    *cell = Cell::Null;
                }
                if table.name == "export_records" && column.name == "destination_path" {
                    if let Cell::Text(s) = cell {
                        *s = basename(s)?;
                    }
                }
                let sanitize = matches!(
                    (table.name.as_str(), column.name.as_str()),
                    (
                        "processing_jobs",
                        "parameters_json" | "profile_snapshot_json"
                    ) | (
                        "analysis_revisions",
                        "profile_snapshot_json" | "config_json"
                    ) | ("minutes_revisions", "model_snapshot_json")
                        | ("audio_tracks", "processing_config_json")
                        | ("recording_sessions", "capture_config_json")
                        | ("analysis_flows", "plan_json")
                        | ("job_checkpoints", "checkpoint_json")
                );
                if sanitize {
                    if let Cell::Text(text) = cell {
                        *text = String::from_utf8(portable_json_with_metadata(
                            text.as_bytes(),
                            false,
                            references,
                            &known,
                        )?)
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
            let bytes = serde_json::to_vec(&Row {
                table: table.name.clone(),
                cells,
            })
            .map_err(|e| e.to_string())?;
            if bytes.len() > MAX_ROW_BYTES {
                return Err("バックアップの1レコードが大きすぎます".into());
            }
            out.write_all(&bytes)
                .and_then(|_| out.write_all(b"\n"))
                .map_err(|e| e.to_string())?;
            table.row_count = table
                .row_count
                .checked_add(1)
                .ok_or("バックアップ件数が範囲外です")?;
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    out.get_ref().sync_all().map_err(|e| e.to_string())?;
    Ok(descriptions)
}

// No archive SQL is executed. Only app-owned schema and generated INSERTs are
// used in a private validation DB; its triggers are not the live app's triggers.
pub(super) fn load(
    path: &Path,
    database: &Path,
    descriptions: &[Table],
    version: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Connection, String> {
    private_file(database)?
        .sync_all()
        .map_err(|e| e.to_string())?;
    let db = Connection::open(database).map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA journal_mode=MEMORY; PRAGMA synchronous=FULL;")
        .map_err(|e| e.to_string())?;
    let mut db = initialize(db, version)?;
    let trusted = tables(&db)?;
    if descriptions.len() != trusted.len()
        || descriptions
            .iter()
            .zip(&trusted)
            .any(|(a, b)| a.name != b.name || a.columns != b.columns)
    {
        return Err("バックアップのテーブル・列定義が一致しません".into());
    }
    let triggers: Vec<String> = db
        .prepare("SELECT name FROM sqlite_schema WHERE type='trigger'")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for trigger in triggers {
        db.execute_batch(&format!("DROP TRIGGER \"{trigger}\""))
            .map_err(|e| e.to_string())?;
    }
    let tx = db.transaction().map_err(|e| e.to_string())?;
    tx.execute_batch("PRAGMA defer_foreign_keys=ON")
        .map_err(|e| e.to_string())?;
    let mut input = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
    for (table, trusted) in descriptions.iter().zip(&trusted) {
        let names = trusted
            .columns
            .iter()
            .map(|c| format!("\"{}\"", c.name))
            .collect::<Vec<_>>()
            .join(",");
        let placeholders = vec!["?"; trusted.columns.len()].join(",");
        let mut insert = tx
            .prepare(&format!(
                "INSERT INTO \"{}\"({names}) VALUES({placeholders})",
                trusted.name
            ))
            .map_err(|e| e.to_string())?;
        for _ in 0..table.row_count {
            check_cancelled(cancelled)?;
            let line = bounded_line(&mut input)?;
            let row: Row = serde_json::from_slice(&line).map_err(|e| e.to_string())?;
            if row.table != trusted.name || row.cells.len() != trusted.columns.len() {
                return Err("バックアップの行・列数が不正です".into());
            }
            let values: Vec<SqlValue> = row
                .cells
                .into_iter()
                .zip(&trusted.columns)
                .map(|(cell, col)| match (cell, col.data_type.as_str()) {
                    (Cell::Null, _) => Ok(SqlValue::Null),
                    (Cell::Integer(n), "INTEGER") => Ok(SqlValue::Integer(n)),
                    (Cell::Real(n), "REAL") if n.is_finite() => Ok(SqlValue::Real(n)),
                    (Cell::Text(s), "TEXT") => Ok(SqlValue::Text(s)),
                    _ => Err("バックアップの列型が不正です".to_owned()),
                })
                .collect::<Result<_, _>>()?;
            insert
                .execute(rusqlite::params_from_iter(values))
                .map_err(|e| e.to_string())?;
        }
    }
    if !input.fill_buf().map_err(|e| e.to_string())?.is_empty() {
        return Err("バックアップに余分なDBレコードがあります".into());
    }
    tx.commit()
        .map_err(|e| format!("バックアップの参照整合が不正です: {e}"))?;
    validate_scopes(&db)?;
    super::ledger::validate_provenance(&db, cancelled)?;
    Ok(db)
}

fn bounded_line(input: &mut impl BufRead) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    loop {
        let bytes = input.fill_buf().map_err(|e| e.to_string())?;
        if bytes.is_empty() {
            return Err("バックアップのDBレコードが不足しています".into());
        }
        let end = bytes.iter().position(|&b| b == b'\n');
        let count = end.map_or(bytes.len(), |i| i + 1);
        if result
            .len()
            .checked_add(count)
            .is_none_or(|n| n > MAX_ROW_BYTES + 1)
        {
            return Err("バックアップのDBレコードが大きすぎます".into());
        }
        result.extend_from_slice(&bytes[..count]);
        input.consume(count);
        if end.is_some() {
            result.pop();
            return Ok(result);
        }
    }
}

pub(super) fn validate_scopes(db: &Connection) -> Result<(), String> {
    super::input_snapshots::validate_database(db, &|| false)?;
    for (table, head) in [
        ("analysis_revisions", "current_analysis_id"),
        ("analysis_revisions", "previous_analysis_id"),
        ("minutes_revisions", "current_minutes_id"),
        ("minutes_revisions", "previous_minutes_id"),
    ] {
        let invalid: bool = db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM meetings m JOIN {table} r ON r.id=m.{head} WHERE r.meeting_id!=m.id OR r.publication_state!='committed')"),[],|r| r.get(0)).map_err(|e| e.to_string())?;
        if invalid {
            return Err("バックアップの現在版・直前版が別会議を参照しています".into());
        }
    }
    for sql in [
        "SELECT EXISTS(SELECT 1 FROM minutes_items i JOIN minutes_revisions r ON r.id=i.minutes_revision_id WHERE i.analysis_revision_id!=r.input_analysis_id)",
        "SELECT EXISTS(SELECT 1 FROM evidence_links e JOIN minutes_revisions r ON r.id=e.minutes_revision_id JOIN utterances u ON u.id=e.utterance_id WHERE e.analysis_revision_id!=r.input_analysis_id OR e.analysis_revision_id!=u.analysis_revision_id OR e.start_us<u.start_us OR e.end_us>u.end_us)",
        "SELECT EXISTS(SELECT 1 FROM export_records e JOIN minutes_revisions r ON r.id=e.minutes_revision_id WHERE e.meeting_id!=r.meeting_id)",
        "SELECT EXISTS(SELECT 1 FROM analysis_flows f JOIN processing_jobs j ON j.id=f.transcription_job_id WHERE j.meeting_id!=f.meeting_id OR j.kind!='transcription')",
        "SELECT EXISTS(SELECT 1 FROM analysis_flows f JOIN processing_jobs j ON j.id=f.diarization_job_id WHERE j.meeting_id!=f.meeting_id OR j.kind!='diarization')",
        "SELECT EXISTS(SELECT 1 FROM audio_import_commits p JOIN file_assets f ON f.id=p.manifest_asset_id WHERE f.meeting_id!=p.meeting_id OR f.content_hash!=p.manifest_hash)",
        "SELECT EXISTS(SELECT 1 FROM vocabulary_scopes WHERE (meeting_id IS NULL AND id!='global') OR (meeting_id IS NOT NULL AND id!='meeting:'||meeting_id))",
    ] {
        if db.query_row(sql,[],|r| r.get::<_,bool>(0)).map_err(|e| e.to_string())? { return Err("バックアップ内で別会議・別版の根拠または入力を参照しています".into()); }
    }
    Ok(())
}
