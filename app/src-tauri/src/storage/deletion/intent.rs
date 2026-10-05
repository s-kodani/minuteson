//! Durable confirmation observations, not an ownership/unlink capability.
//! No user-facing caller exists until deletion admission and recovery follow.
use super::{
    database::Scope,
    files::{self, Census, Entry, Manifest, Registered},
    fs::Proof,
};
use crate::storage::{now_ms, Storage};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_INTENT: usize = 65_536;
const MAX_OBSERVATION: usize = 1024;
const INITIAL_PROGRESS: &str = "{\"filePhase\":\"notStarted\"}";

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s)
}
fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("削除の確認記録を取り消しました".into())
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Intent {
    version: u32,
    scope: Scope,
    // Header only: full entries live in maintenance_paths, not the 64KiB intent.
    file_header: Manifest,
    file_manifest_hash: String,
    path_count: usize,
    paths_hash: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Observation {
    proof: Option<Proof>,
    registered: Option<Registered>,
    stage_hint: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct PathRow {
    path: String,
    kind: String,
    size: Option<i64>,
    content_hash: Option<String>,
    observation: String,
    observation_hash: String,
}
fn encode<T: Serialize>(value: &T, limit: usize) -> Result<String, String> {
    let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if json.len() > limit {
        Err("削除確認記録の容量上限を超えています".into())
    } else {
        Ok(json)
    }
}
fn paths_hash(rows: &[PathRow]) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(b"mon-delete-observed-paths-v1\0");
    for row in rows {
        let bytes = serde_json::to_vec(row).map_err(|e| e.to_string())?;
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}
impl PathRow {
    fn from_entry(entry: &Entry) -> Result<Self, String> {
        let (size, content_hash) = if entry.directory {
            (None, None)
        } else if let Some(proof) = &entry.proof {
            (
                Some(i64::try_from(proof.size).map_err(|_| "ファイル容量が範囲外です")?),
                entry.sha256.clone(),
            )
        } else {
            let asset = entry
                .registered
                .as_ref()
                .ok_or("欠落対象の登録記録がありません")?;
            (Some(asset.size), Some(asset.sha256.clone()))
        };
        let observation = encode(
            &Observation {
                proof: entry.proof.clone(),
                registered: entry.registered.clone(),
                stage_hint: entry.stage_hint.clone(),
            },
            MAX_OBSERVATION,
        )?;
        Ok(Self {
            path: entry.path.clone(),
            kind: if entry.directory { "directory" } else { "file" }.into(),
            size,
            content_hash,
            observation_hash: hash(observation.as_bytes()),
            observation,
        })
    }
    fn entry(&self, meeting: &str) -> Result<Entry, String> {
        if (self.path != format!("meetings/{meeting}") && !files::valid_path(&self.path, meeting))
            || self.observation.len() > MAX_OBSERVATION
            || hash(self.observation.as_bytes()) != self.observation_hash
        {
            return Err("削除対象のパス・実体記録が不正です".into());
        }
        let observed: Observation =
            serde_json::from_str(&self.observation).map_err(|e| e.to_string())?;
        if encode(&observed, MAX_OBSERVATION)? != self.observation {
            return Err("削除対象の実体記録が標準形式と一致しません".into());
        }
        let directory = match self.kind.as_str() {
            "directory" => true,
            "file" => false,
            _ => return Err("削除対象の種類が不正です".into()),
        };
        if let Some(asset) = &observed.registered {
            if asset.id.is_empty()
                || asset.id.len() > 256
                || asset.size < 0
                || !valid_hash(&asset.sha256)
                || !matches!(
                    asset.state.as_str(),
                    "ready" | "missing" | "corrupt" | "delete_pending"
                )
            {
                return Err("削除対象の登録資産記録が不正です".into());
            }
        }
        if observed.stage_hint.as_deref().is_some_and(|s| {
            !matches!(
                s,
                "recording" | "import" | "transcription" | "diarization" | "minutes_generation"
            )
        }) {
            return Err("削除対象の由来候補が不正です".into());
        }
        if let Some(proof) = &observed.proof {
            proof.validate(directory)?;
        }
        if directory {
            if observed.proof.is_none()
                || observed.registered.is_some()
                || observed.stage_hint.is_some()
                || self.size.is_some()
                || self.content_hash.is_some()
            {
                return Err("削除対象フォルダーの記録が不正です".into());
            }
        } else {
            if self.path == format!("meetings/{meeting}")
                || self.size.is_none_or(|s| s < 0)
                || !self.content_hash.as_deref().is_some_and(valid_hash)
            {
                return Err("削除対象ファイルの記録が不正です".into());
            }
            if let Some(proof) = &observed.proof {
                if self.size != Some(proof.size as i64) {
                    return Err("削除対象容量が実体記録と一致しません".into());
                }
            } else if observed.stage_hint.is_some()
                || observed.registered.as_ref().is_none_or(|a| {
                    self.size != Some(a.size)
                        || self.content_hash.as_deref() != Some(a.sha256.as_str())
                })
            {
                return Err("欠落対象を実在ファイルとして採用しません".into());
            }
        }
        Ok(Entry {
            path: self.path.clone(),
            directory,
            sha256: if observed.proof.is_some() {
                self.content_hash.clone()
            } else {
                None
            },
            proof: observed.proof,
            registered: observed.registered,
            stage_hint: observed.stage_hint,
        })
    }
}
impl Intent {
    fn from_census(census: &Census, rows: &[PathRow]) -> Result<Self, String> {
        let mut header = census.manifest().clone();
        header.entries.clear();
        Ok(Self {
            version: 1,
            scope: census.scope().clone(),
            file_header: header,
            file_manifest_hash: census.hash()?,
            path_count: rows.len(),
            paths_hash: paths_hash(rows)?,
        })
    }
    fn validate(&self, db: &Connection) -> Result<(), String> {
        self.scope.validate_format(db)?;
        self.file_header.root.validate(true)?;
        self.file_header.database.validate(false)?;
        if self.version != 1
            || self.file_header.version != 1
            || !self.file_header.entries.is_empty()
            || self.file_header.meeting_id != self.scope.meeting_id
            || self.file_header.database_scope_hash != self.scope.hash()?
            || self.path_count > files::MAX_ENTRIES
            || !valid_hash(&self.paths_hash)
            || !valid_hash(&self.file_manifest_hash)
        {
            return Err("削除の固定確認記録が不正です".into());
        }
        Ok(())
    }
    fn manifest(&self, rows: &[PathRow]) -> Result<Manifest, String> {
        if self.path_count != rows.len() || paths_hash(rows)? != self.paths_hash {
            return Err("削除対象のパス一覧が変更または欠落しています".into());
        }
        let mut manifest = self.file_header.clone();
        let mut previous: Option<&str> = None;
        for row in rows {
            if previous.is_some_and(|p| p >= row.path.as_str()) {
                return Err("削除対象の順序・重複が不正です".into());
            }
            previous = Some(&row.path);
            manifest.entries.push(row.entry(&self.scope.meeting_id)?);
        }
        if hash(encode(&manifest, 64 * 1024 * 1024)?.as_bytes()) != self.file_manifest_hash {
            return Err("削除対象の全体manifestが確認記録と一致しません".into());
        }
        Ok(manifest)
    }
}

// A saved observation can be inspected even when files changed. verify_live is
// mandatory before any future transition; loading is not deletion admission.
#[derive(Debug)]
pub(super) struct Prepared {
    operation: String,
    intent: Intent,
    manifest: Manifest,
    consent_at: i64,
}
impl Prepared {
    pub(super) fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub(super) fn scope(&self) -> &Scope {
        &self.intent.scope
    }
    pub(super) fn operation(&self) -> &str {
        &self.operation
    }
    pub(super) fn consent_at(&self) -> i64 {
        self.consent_at
    }
    pub(super) fn verify_live(
        &self,
        storage: &Storage,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        cancel(check)?;
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let saved = read_locked(&db, &self.operation, check)?;
        if saved.intent != self.intent
            || saved.manifest != self.manifest
            || saved.consent_at != self.consent_at
        {
            return Err("削除の確認記録が変更されています".into());
        }
        self.verify_locked(storage, &db, check)
    }
    fn verify_locked(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if db.query_row("SELECT EXISTS(SELECT 1 FROM maintenance_operations WHERE id!=?1 AND state IN ('prepared','installing','cleanup','attention'))", params![self.operation], |r| r.get::<_,bool>(0)).map_err(|e|e.to_string())? {
            return Err("別の未完了保存領域操作があります".into());
        }
        let current = Census::capture_locked(
            storage,
            db,
            &self.intent.scope.meeting_id,
            &self.operation,
            check,
        )?;
        if current.scope() != &self.intent.scope || current.manifest() != &self.manifest {
            return Err("確認後にDB・ファイル・実体が変更されています。再確認してください".into());
        }
        Ok(())
    }
}
fn read_locked(
    db: &Connection,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<Prepared, String> {
    cancel(check)?;
    if !uuid(operation) {
        return Err("削除操作の識別子が不正です".into());
    }
    let (meeting, json, expected_hash, consent, state, progress, created, updated): (Option<String>,String,String,Option<i64>,String,String,i64,i64) = db.query_row(
        "SELECT meeting_id,target_manifest_json,target_hash,consent_at,state,progress_json,created_at,updated_at FROM maintenance_operations WHERE id=?1 AND kind='delete_meeting'",
        params![operation], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).map_err(|e| e.to_string())?;
    if json.len() > MAX_INTENT
        || hash(json.as_bytes()) != expected_hash
        || consent.is_none_or(|s| s < 0)
        || state != "prepared"
        || progress != INITIAL_PROGRESS
        || created < 0
        || updated < created
        || consent != Some(created)
    {
        return Err("未開始の削除確認記録を検証できません".into());
    }
    let intent: Intent = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    intent.validate(db)?;
    if encode(&intent, MAX_INTENT)? != json
        || meeting.as_deref() != Some(intent.scope.meeting_id.as_str())
    {
        return Err("削除の対象会議・固定確認記録が一致しません".into());
    }
    let mut q = db.prepare("SELECT relative_path,kind,size_bytes,content_hash,identity_json,identity_hash FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path LIMIT 200001").map_err(|e| e.to_string())?;
    let mut query = q.query(params![operation]).map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    while let Some(row) = query.next().map_err(|e| e.to_string())? {
        cancel(check)?;
        if rows.len() >= files::MAX_ENTRIES {
            return Err("削除対象件数が安全上限を超えています".into());
        }
        rows.push(PathRow {
            path: row.get(0).map_err(|e| e.to_string())?,
            kind: row.get(1).map_err(|e| e.to_string())?,
            size: row.get(2).map_err(|e| e.to_string())?,
            content_hash: row.get(3).map_err(|e| e.to_string())?,
            observation: row.get(4).map_err(|e| e.to_string())?,
            observation_hash: row.get(5).map_err(|e| e.to_string())?,
        });
    }
    let manifest = intent.manifest(&rows)?;
    cancel(check)?;
    Ok(Prepared {
        operation: operation.into(),
        intent,
        manifest,
        consent_at: consent.unwrap(),
    })
}
pub(super) fn read(
    storage: &Storage,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<Prepared, String> {
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    // Never read a new path while cached SQLite still refers to the old DB.
    storage.managed_anchor.identity()?;
    if storage.restart_required() {
        return Err("旧保存先は停止済みです".into());
    }
    read_locked(&db, operation, check)
}

// Only a freshly verified in-memory Census may fix confirmation observations.
// The eventual controller must establish explicit user consent and exclusive
// admission before calling this. This helper itself exposes no IPC or unlink.
pub(super) fn prepare(
    storage: &Storage,
    operation: &str,
    confirmed: &Census,
    check: &impl Fn() -> bool,
) -> Result<Prepared, String> {
    prepare_inner(storage, operation, confirmed, check, &|_| {})
}
fn prepare_inner(
    storage: &Storage,
    operation: &str,
    confirmed: &Census,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<Prepared, String> {
    cancel(check)?;
    if !uuid(operation) {
        return Err("削除操作の識別子が不正です".into());
    }
    let rows = confirmed
        .manifest()
        .entries
        .iter()
        .map(PathRow::from_entry)
        .collect::<Result<Vec<_>, _>>()?;
    let intent = Intent::from_census(confirmed, &rows)?;
    let json = encode(&intent, MAX_INTENT)?;
    let mut db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    if !db.is_autocommit() {
        return Err("削除確認のトランザクションが既に進行中です".into());
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    intent.validate(&tx)?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM maintenance_operations WHERE id=?1",
            params![operation],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if existing.is_some() {
        let saved = read_locked(&tx, operation, check)?;
        if saved.intent != intent || saved.manifest != *confirmed.manifest() {
            return Err("同じ操作IDに別の削除対象を採用しません".into());
        }
        saved.verify_locked(storage, &tx, check)?;
        cancel(check)?;
        tx.commit().map_err(|e| e.to_string())?;
        return Ok(saved);
    }
    let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM maintenance_operations WHERE state IN ('prepared','installing','cleanup','attention'))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if pending {
        return Err("未完了の保存領域操作を確認・解決してから削除を確認してください".into());
    }
    let prepared = Prepared {
        operation: operation.into(),
        intent,
        manifest: confirmed.manifest().clone(),
        consent_at: now_ms()?,
    };
    prepared.verify_locked(storage, &tx, check)?;
    cancel(check)?;
    tx.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_meeting','prepared',?3,?4,?5,?6,?5,?5)",params![operation,prepared.intent.scope.meeting_id,json,hash(json.as_bytes()),prepared.consent_at,INITIAL_PROGRESS]).map_err(|e|e.to_string())?;
    event("intentStored");
    for row in rows {
        cancel(check)?;
        tx.execute("INSERT INTO maintenance_paths(operation_id,relative_path,kind,size_bytes,content_hash,identity_json,identity_hash) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![operation,row.path,row.kind,row.size,row.content_hash,row.observation,row.observation_hash]).map_err(|e|e.to_string())?;
        event("pathStored");
    }
    // Including the new intent in the DB hash would invalidate its own scope.
    // Only this exact operation is omitted; all earlier operation rows remain.
    prepared.verify_locked(storage, &tx, check)?;
    let saved = read_locked(&tx, operation, check)?;
    if saved.intent != prepared.intent || saved.manifest != prepared.manifest {
        return Err("削除確認の保存結果が一致しません".into());
    }
    cancel(check)?;
    event("beforeCommit");
    cancel(check)?;
    tx.commit().map_err(|e| e.to_string())?;
    event("committed");
    Ok(saved)
}
#[cfg(test)]
pub(super) fn prepare_observed(
    storage: &Storage,
    operation: &str,
    confirmed: &Census,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<Prepared, String> {
    prepare_inner(storage, operation, confirmed, check, event)
}
