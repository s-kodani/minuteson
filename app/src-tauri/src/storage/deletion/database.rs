use crate::storage::apply_migrations;
#[cfg(test)]
use crate::storage::now_ms;
use rusqlite::{config::DbConfig, params, types::ValueRef, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const MAX_ROWS: u64 = 200_000;
const MAX_CELL: usize = 32 * 1024 * 1024;
const MAX_MANIFEST: usize = 65_536;

// Every selector is repository-owned SQL. No request may choose tables or SQL.
// Order is dependency order, except the three self-referential trees below.
const TARGETS: &[(&str, &str)] = &[
    ("analysis_flows", "meeting_id=?1"),
    (
        "edit_operations",
        "draft_id IN (SELECT id FROM working_drafts WHERE meeting_id=?1)",
    ),
    ("working_drafts", "meeting_id=?1"),
    ("export_records", "meeting_id=?1"),
    (
        "evidence_links",
        "minutes_revision_id IN (SELECT id FROM minutes_revisions WHERE meeting_id=?1)",
    ),
    (
        "minutes_items",
        "minutes_revision_id IN (SELECT id FROM minutes_revisions WHERE meeting_id=?1)",
    ),
    ("vocabulary_snapshots", "meeting_id=?1"),
    (
        "job_checkpoints",
        "job_id IN (SELECT id FROM processing_jobs WHERE meeting_id=?1)",
    ),
    ("processing_jobs", "meeting_id=?1"),
    ("minutes_revisions", "meeting_id=?1"),
    (
        "diarization_manual_assignments",
        "analysis_id IN (SELECT id FROM analysis_revisions WHERE meeting_id=?1)",
    ),
    ("analysis_input_snapshots", "meeting_id=?1"),
    (
        "utterance_speakers",
        "analysis_revision_id IN (SELECT id FROM analysis_revisions WHERE meeting_id=?1)",
    ),
    ("utterances", "meeting_id=?1"),
    ("speaker_segments", "meeting_id=?1"),
    (
        "speakers",
        "analysis_revision_id IN (SELECT id FROM analysis_revisions WHERE meeting_id=?1)",
    ),
    ("analysis_revisions", "meeting_id=?1"),
    (
        "track_clock_segments",
        "track_id IN (SELECT id FROM audio_tracks WHERE meeting_id=?1)",
    ),
    (
        "audio_gaps",
        "track_id IN (SELECT id FROM audio_tracks WHERE meeting_id=?1)",
    ),
    ("track_inputs", "meeting_id=?1"),
    ("audio_chunks", "meeting_id=?1"),
    ("audio_tracks", "meeting_id=?1"),
    (
        "audio_import_options",
        "import_id IN (SELECT id FROM audio_imports WHERE meeting_id=?1)",
    ),
    ("audio_import_commits", "meeting_id=?1"),
    ("audio_imports", "meeting_id=?1"),
    ("recording_sessions", "meeting_id=?1"),
    ("restore_omitted_assets", "meeting_id=?1"),
    ("file_assets", "meeting_id=?1"),
    (
        "vocabulary_entries",
        "scope_id IN (SELECT id FROM vocabulary_scopes WHERE meeting_id=?1)",
    ),
    ("vocabulary_scopes", "meeting_id=?1"),
    ("restore_provenance", "meeting_id=?1"),
    ("meetings", "id=?1"),
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Table {
    pub name: String,
    pub rows: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Scope {
    pub version: u32,
    pub meeting_id: String,
    pub schema_hash: String,
    pub tables: Vec<Table>,
    pub maintenance_hash: String,
}

// Historical audio metadata only. Non-audio artifacts may still be added by
// text-based minutes generation after source audio is deliberately removed.
pub(super) fn audio_tables(
    db: &Connection,
    meeting: &str,
    check: &impl Fn() -> bool,
) -> Result<Vec<Table>, String> {
    if !uuid(meeting) {
        return Err("音声履歴の会議IDが不正です".into());
    }
    trusted_schema(db)?;
    [
        ("audio_chunks", "meeting_id=?1"),
        ("audio_tracks", "meeting_id=?1"),
        ("file_assets", "meeting_id=?1 AND media_type LIKE 'audio/%'"),
    ]
    .iter()
    .map(|(name, predicate)| table(db, name, predicate, meeting, "", check))
    .collect()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s)
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("削除対象の検査を取り消しました".into())
    } else {
        Ok(())
    }
}
fn schema(db: &Connection) -> Result<Vec<(String, String, String, String)>, String> {
    db.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY type,name")
        .map_err(|e| e.to_string())?.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(|e| e.to_string())?.collect::<Result<_,_>>().map_err(|e| e.to_string())
}
fn expected_schema() -> Result<&'static Vec<(String, String, String, String)>, String> {
    // Cache only the schema built from this binary's fixed migrations, never a
    // live connection's schema, migration history, trigger/FK state or rows.
    // Each actual DB still undergoes every inspection below on every call.
    static EXPECTED: std::sync::OnceLock<Result<Vec<(String, String, String, String)>, String>> =
        std::sync::OnceLock::new();
    EXPECTED
        .get_or_init(|| {
            let mut expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
            apply_migrations(&mut expected).map_err(|e| e.to_string())?;
            schema(&expected)
        })
        .as_ref()
        .map_err(Clone::clone)
}
// Older portable/private readers may inspect v20/v21 before their normal
// migration. Check the actual fixed schema too: truncating migration history
// cannot disguise a current DB with its deletion journal removed.
pub(super) fn legacy_without_journal(db: &Connection) -> Result<bool, String> {
    let version = crate::storage::verify_migration_history(db).map_err(|e| e.to_string())?;
    if !(20..22).contains(&version) {
        return Ok(false);
    }
    #[cfg(unix)]
    super::startup::verify_temp_schema(db)?;
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL);").map_err(|e|e.to_string())?;
    for (_, sql) in &crate::storage::MIGRATIONS[..version] {
        expected.execute_batch(sql).map_err(|e| e.to_string())?;
    }
    if schema(db)? != schema(&expected)?
        || !db
            .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, bool>(0))
            .map_err(|e| e.to_string())?
        || !db
            .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
            .map_err(|e| e.to_string())?
    {
        return Err("音声利用可否を確認する旧DBの構造・保護が既知の形式と一致しません".into());
    }
    if db
        .prepare("PRAGMA foreign_key_check")
        .map_err(|e| e.to_string())?
        .query([])
        .map_err(|e| e.to_string())?
        .next()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("旧DBの参照整合性を確認できません".into());
    }
    Ok(true)
}
pub(super) fn trusted_schema(db: &Connection) -> Result<String, String> {
    #[cfg(unix)]
    super::startup::verify_temp_schema(db)?;
    #[cfg(not(unix))]
    if db
        .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_schema)", [], |r| {
            r.get::<_, bool>(0)
        })
        .map_err(|e| e.to_string())?
    {
        return Err("削除対象DBに未対応の一時オブジェクトがあります".into());
    }
    if crate::storage::verify_migration_history(db).map_err(|e| e.to_string())?
        != crate::storage::MIGRATIONS.len()
    {
        return Err("削除対象のDB移行が未完了です".into());
    }
    let actual = schema(db)?;
    if &actual != expected_schema()? {
        return Err("削除対象DBの構造・保護制約が既知の形式と一致しません".into());
    }
    if !db
        .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, bool>(0))
        .map_err(|e| e.to_string())?
        || !db
            .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
            .map_err(|e| e.to_string())?
    {
        return Err("削除対象の参照保護が有効ではありません".into());
    }
    if db
        .prepare("PRAGMA foreign_key_check")
        .map_err(|e| e.to_string())?
        .query([])
        .map_err(|e| e.to_string())?
        .next()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("削除前のDB参照整合性を確認できません".into());
    }
    Ok(digest(
        &serde_json::to_vec(&actual).map_err(|e| e.to_string())?,
    ))
}

pub(super) fn stopped(db: &Connection) -> Result<(), String> {
    let active: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE state IN ('running','pausing')) OR EXISTS(SELECT 1 FROM recording_sessions WHERE state IN ('preparing','recording','stopping')) OR EXISTS(SELECT 1 FROM audio_imports WHERE state IN ('copying','converting')) OR EXISTS(SELECT 1 FROM export_records WHERE state='writing') OR EXISTS(SELECT 1 FROM audio_tracks WHERE state='writing') OR EXISTS(SELECT 1 FROM analysis_revisions WHERE publication_state='building') OR EXISTS(SELECT 1 FROM minutes_revisions WHERE publication_state='building') OR EXISTS(SELECT 1 FROM model_packages WHERE source_type!='historical_snapshot' AND state='preparing')", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    if active {
        Err("録音・解析・取り込み・準備・書き出しの停止完了後に削除を確認してください".into())
    } else {
        Ok(())
    }
}

// A crash-stale partial model package is a legitimate deletion target. The
// caller MUST own the real DataMaintenanceLease (which excludes live model
// workers); all other active SQL work and unfinished CPU checks still block.
#[cfg(unix)]
pub(super) fn stopped_for_model_consent(db: &Connection) -> Result<(), String> {
    let active: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE state IN ('running','pausing')) OR EXISTS(SELECT 1 FROM recording_sessions WHERE state IN ('preparing','recording','stopping')) OR EXISTS(SELECT 1 FROM audio_imports WHERE state IN ('copying','converting')) OR EXISTS(SELECT 1 FROM export_records WHERE state='writing') OR EXISTS(SELECT 1 FROM audio_tracks WHERE state='writing') OR EXISTS(SELECT 1 FROM analysis_revisions WHERE publication_state='building') OR EXISTS(SELECT 1 FROM minutes_revisions WHERE publication_state='building') OR EXISTS(SELECT 1 FROM model_compatibility_checks WHERE state='preparing')", [], |r|r.get(0)).map_err(|e|e.to_string())?;
    if active {
        Err(
            "録音・解析・取り込み・互換性確認・書き出しの停止完了後にモデル削除を確認してください"
                .into(),
        )
    } else {
        Ok(())
    }
}

pub(super) fn secure_delete(db: &Connection) -> Result<(), String> {
    db.execute_batch("PRAGMA secure_delete=ON")
        .map_err(|e| e.to_string())?;
    if db
        .query_row("PRAGMA secure_delete", [], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        != 1
    {
        return Err("DBの削除領域を消去する設定を有効にできません".into());
    }
    Ok(())
}

// Hash all typed cells, including inactive drafts, fixed input text and heads.
// Only hashes/counts enter the durable manifest; never copy those texts there.
fn table(
    db: &Connection,
    name: &str,
    predicate: &str,
    meeting: &str,
    omitted_operation: &str,
    check: &impl Fn() -> bool,
) -> Result<Table, String> {
    let mut q = db
        .prepare(&format!(
            "SELECT * FROM {} WHERE {predicate} ORDER BY rowid",
            quote(name)
        ))
        .map_err(|e| e.to_string())?;
    let columns = q.column_count();
    let mut hash = Sha256::new();
    hash.update(name.as_bytes());
    hash.update(serde_json::to_vec(&q.column_names()).map_err(|e| e.to_string())?);
    let mut rows = if name == "maintenance_operations" {
        q.query(params![meeting, omitted_operation])
    } else {
        q.query(params![meeting])
    }
    .map_err(|e| e.to_string())?;
    let mut count = 0;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        cancel(check)?;
        count += 1;
        if count > MAX_ROWS {
            return Err("削除対象のDB件数が安全上限を超えています".into());
        }
        hash.update([0xff]);
        let mut bytes = 0;
        for i in 0..columns {
            hash_cell(
                &mut hash,
                row.get_ref(i).map_err(|e| e.to_string())?,
                &mut bytes,
            )?;
        }
    }
    Ok(Table {
        name: name.into(),
        rows: count,
        sha256: format!("{:x}", hash.finalize()),
    })
}

fn hash_cell(hash: &mut Sha256, value: ValueRef<'_>, bytes: &mut usize) -> Result<(), String> {
    match value {
        ValueRef::Null => hash.update([0]),
        ValueRef::Integer(n) => {
            hash.update([1]);
            hash.update(n.to_le_bytes());
        }
        ValueRef::Real(n) if n.is_finite() => {
            hash.update([2]);
            hash.update(n.to_bits().to_le_bytes());
        }
        ValueRef::Text(s) if s.len() <= MAX_CELL && std::str::from_utf8(s).is_ok() => {
            *bytes += s.len();
            if *bytes > MAX_CELL {
                return Err("削除対象のDB行が容量上限を超えています".into());
            }
            hash.update([3]);
            hash.update((s.len() as u64).to_le_bytes());
            hash.update(s);
        }
        _ => return Err("削除対象に未対応のDB値があります".into()),
    }
    Ok(())
}

// Reconstruct the confirmation's complete maintenance fingerprint after the
// meeting FK was detached by ON DELETE SET NULL. This is a hash-only view: no
// row is rewritten, and every other cell remains byte-for-byte bound to Scope.
#[cfg(unix)]
pub(super) fn detached_maintenance_hash(
    db: &Connection,
    meeting: &str,
    ids: &[String],
    check: &impl Fn() -> bool,
) -> Result<String, String> {
    trusted_schema(db)?;
    if !uuid(meeting) || ids.len() > 1024 {
        return Err("保持する保守履歴の範囲が不正です".into());
    }
    let mut q = db
        .prepare("SELECT * FROM maintenance_operations WHERE id=?1")
        .map_err(|e| e.to_string())?;
    let columns = q.column_count();
    let fk = q.column_index("meeting_id").map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    hash.update(b"maintenance_operations");
    hash.update(serde_json::to_vec(&q.column_names()).map_err(|e| e.to_string())?);
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        cancel(check)?;
        if !uuid(id) || !seen.insert(id) {
            return Err("保持する保守履歴のIDが不正です".into());
        }
        let mut rows = q.query([id]).map_err(|e| e.to_string())?;
        let row = rows
            .next()
            .map_err(|e| e.to_string())?
            .ok_or("保持する保守履歴が欠落しています")?;
        if !matches!(row.get_ref(fk).map_err(|e| e.to_string())?, ValueRef::Null) {
            return Err("完全削除後の会議参照が再接続されています".into());
        }
        hash.update([0xff]);
        let mut bytes = 0;
        for i in 0..columns {
            let value = if i == fk {
                ValueRef::Text(meeting.as_bytes())
            } else {
                row.get_ref(i).map_err(|e| e.to_string())?
            };
            hash_cell(&mut hash, value, &mut bytes)?;
        }
    }
    cancel(check)?;
    Ok(format!("{:x}", hash.finalize()))
}

// Check every declared incoming FK, not just today's common rows. A valid
// cross-meeting retry or future reference cannot be silently swept along.
fn closed_scope(db: &Connection, meeting: &str, check: &impl Fn() -> bool) -> Result<(), String> {
    let names: Vec<String> = db
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for child in names {
        cancel(check)?;
        let mut q = db
            .prepare(&format!("PRAGMA foreign_key_list({})", quote(&child)))
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut keys = BTreeMap::<i64, (String, String, Vec<(String, String)>)>::new();
        for row in rows {
            let (id, parent, from, to, action) = row.map_err(|e| e.to_string())?;
            keys.entry(id)
                .or_insert_with(|| (parent, action, vec![]))
                .2
                .push((from, to));
        }
        for (_, (parent, action, columns)) in keys {
            let Some((_, parent_scope)) = TARGETS.iter().find(|(n, _)| *n == parent) else {
                continue;
            };
            if child == "maintenance_operations" && parent == "meetings" && action == "SET NULL" {
                continue;
            }
            let own = TARGETS
                .iter()
                .find(|(n, _)| *n == child)
                .map(|(_, scope)| {
                    format!(
                        "c.rowid NOT IN (SELECT rowid FROM {} WHERE {scope})",
                        quote(&child)
                    )
                })
                .unwrap_or_else(|| "1".into());
            let join = columns
                .iter()
                .map(|(from, to)| format!("c.{}=p.{}", quote(from), quote(to)))
                .collect::<Vec<_>>()
                .join(" AND ");
            let sql = format!("SELECT EXISTS(SELECT 1 FROM {} c JOIN {} p ON {join} WHERE p.rowid IN (SELECT rowid FROM {} WHERE {parent_scope}) AND {own})", quote(&child), quote(&parent), quote(&parent));
            if db
                .query_row(&sql, params![meeting], |r| r.get::<_, bool>(0))
                .map_err(|e| e.to_string())?
            {
                return Err(format!(
                    "削除対象の{parent}を対象外の{child}が参照しています。範囲を広げて削除しません"
                ));
            }
        }
    }
    Ok(())
}

impl Scope {
    pub(super) fn capture(
        db: &Connection,
        meeting: &str,
        omitted_operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::capture_inner(db, meeting, omitted_operation, check, true)
    }
    // Read-only current observations under a typed, completed unavailable-audio
    // source. The caller owns the live maintenance lease; crash-stale work is
    // retained, not silently recovered while startup has deferred writes.
    pub(super) fn capture_audio_reconfirmation(
        db: &Connection,
        meeting: &str,
        omitted_operation: &str,
        source: &super::audio::intent::reconfirmation::Source,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        source.verify_saved(db, check)?;
        if source.meeting() != meeting {
            return Err("音声再同意の会議が一致しません".into());
        }
        Self::capture_inner(db, meeting, omitted_operation, check, false)
    }
    fn capture_inner(
        db: &Connection,
        meeting: &str,
        omitted_operation: &str,
        check: &impl Fn() -> bool,
        require_stopped: bool,
    ) -> Result<Self, String> {
        cancel(check)?;
        if !uuid(meeting) || (!omitted_operation.is_empty() && !uuid(omitted_operation)) {
            return Err("削除対象の識別子が不正です".into());
        }
        let schema_hash = trusted_schema(db)?;
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",
                params![meeting],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("削除する会議が見つかりません".into());
        }
        if require_stopped {
            stopped(db)?;
        }
        closed_scope(db, meeting, check)?;
        let tables = TARGETS
            .iter()
            .map(|(name, predicate)| table(db, name, predicate, meeting, "", check))
            .collect::<Result<Vec<_>, _>>()?;
        let maintenance = table(
            db,
            "maintenance_operations",
            "meeting_id=?1 AND id!=?2",
            meeting,
            omitted_operation,
            check,
        )?;
        let scope = Self {
            version: 1,
            meeting_id: meeting.into(),
            schema_hash,
            tables,
            maintenance_hash: maintenance.sha256,
        };
        if scope.encode()?.len() > MAX_MANIFEST {
            return Err("削除対象のmanifestが容量上限を超えています".into());
        }
        Ok(scope)
    }
    pub(super) fn encode(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
    pub(super) fn hash(&self) -> Result<String, String> {
        Ok(digest(self.encode()?.as_bytes()))
    }
    pub(super) fn validate_format(&self, db: &Connection) -> Result<(), String> {
        let valid_hash = |s: &str| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if trusted_schema(db)? != self.schema_hash
            || self.version != 1
            || !uuid(&self.meeting_id)
            || !valid_hash(&self.maintenance_hash)
            || self.tables.len() != TARGETS.len()
            || self.tables.iter().zip(TARGETS).any(|(t, (name, _))| {
                t.name != *name || t.rows > MAX_ROWS || !valid_hash(&t.sha256)
            })
            || self.tables.last().is_none_or(|t| t.rows != 1)
        {
            return Err("削除の確認対象が既知の形式と一致しません".into());
        }
        Ok(())
    }
}

struct Triggers<'a>(&'a Connection);
impl Drop for Triggers<'_> {
    fn drop(&mut self) {
        if self
            .0
            .set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .is_err()
        {
            // Never expose a connection with accidentally disabled protection.
            let _ = self
                .0
                .authorizer(Some(|_: rusqlite::hooks::AuthContext<'_>| {
                    rusqlite::hooks::Authorization::Deny
                }));
        }
    }
}

// Internal row phase only. Its caller must hold Storage's connection mutex and
// maintenance admission, and must have durably completed the fixed file phase.
// No production entry point calls it yet; the consent/file orchestrator follows.
#[cfg(test)]
pub(super) fn erase_confirmed_rows(
    db: &Connection,
    operation: &str,
    expected: &Scope,
    check: &impl Fn() -> bool,
) -> Result<u64, String> {
    cancel(check)?;
    if !db.is_autocommit() || !uuid(operation) {
        return Err("削除確定の操作・トランザクションが不正です".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let (meeting, manifest, hash, consent): (Option<String>,String,String,Option<i64>) = tx.query_row("SELECT meeting_id,target_manifest_json,target_hash,consent_at FROM maintenance_operations WHERE id=?1 AND kind='delete_meeting' AND state='cleanup'", params![operation], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e| e.to_string())?;
    if meeting.as_deref() != Some(expected.meeting_id.as_str())
        || consent.is_none_or(|n| n < 0)
        || manifest.len() > MAX_MANIFEST
        || digest(manifest.as_bytes()) != hash
        || serde_json::from_str::<Scope>(&manifest).map_err(|e| e.to_string())? != *expected
        || Scope::capture(&tx, &expected.meeting_id, operation, check)? != *expected
    {
        return Err(
            "確認後に削除対象・依存関係・保守記録が変更されています。再確認してください".into(),
        );
    }
    let erased = erase_rows(&tx, expected, check)?;
    tx.execute("UPDATE maintenance_operations SET progress_json='{\"databaseDeleted\":true}',error_code=NULL,updated_at=?2 WHERE id=?1", params![operation,now_ms()?]).map_err(|e| e.to_string())?;
    cancel(check)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(erased)
}

// The compound receipt owner validates its exact saved scope and file proof
// while holding the connection mutex, then commits rows and progress together.
pub(super) fn erase_rows(
    tx: &Connection,
    expected: &Scope,
    check: &impl Fn() -> bool,
) -> Result<u64, String> {
    if tx.is_autocommit() {
        return Err("DB行削除には同じ接続の確定トランザクションが必要です".into());
    }
    expected.validate_format(tx)?;
    let db = tx;
    cancel(check)?;
    // Keep this enabled on the connection even if cancellation rolls back.
    // WAL pages still need the separately resumable purge below.
    secure_delete(db)?;
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .map_err(|e| e.to_string())?;
    let triggers = Triggers(db);
    tx.execute("UPDATE meetings SET current_analysis_id=NULL,previous_analysis_id=NULL,current_minutes_id=NULL,previous_minutes_id=NULL WHERE id=?1", params![expected.meeting_id]).map_err(|e| e.to_string())?;
    let mut erased = 0;
    for ((name, predicate), table) in TARGETS.iter().zip(&expected.tables) {
        cancel(check)?;
        let mut count = 0;
        if matches!(
            *name,
            "processing_jobs" | "analysis_revisions" | "minutes_revisions"
        ) {
            let parent = if *name == "processing_jobs" {
                "retry_of_job_id"
            } else {
                "parent_revision_id"
            };
            while count < table.rows {
                let removed = tx.execute(&format!("DELETE FROM {} WHERE {predicate} AND NOT EXISTS(SELECT 1 FROM {} child WHERE child.{}={}.id)", quote(name), quote(name), quote(parent), quote(name)), params![expected.meeting_id]).map_err(|e| e.to_string())? as u64;
                if removed == 0 {
                    return Err(
                        "削除対象に循環または対象外の参照があります。DB削除を巻き戻しました".into(),
                    );
                }
                count += removed;
                cancel(check)?;
            }
        } else {
            count = tx
                .execute(
                    &format!("DELETE FROM {} WHERE {predicate}", quote(name)),
                    params![expected.meeting_id],
                )
                .map_err(|e| e.to_string())? as u64;
        }
        if count != table.rows {
            return Err(format!(
                "{name}の削除件数が確認対象と一致しません。DB削除を巻き戻しました"
            ));
        }
        erased += count;
    }
    if tx
        .prepare("PRAGMA foreign_key_check")
        .map_err(|e| e.to_string())?
        .query([])
        .map_err(|e| e.to_string())?
        .next()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("削除後の参照整合性が不正です。DB削除を巻き戻しました".into());
    }
    drop(triggers);
    if !db
        .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
        .map_err(|e| e.to_string())?
    {
        return Err("削除後の保護制約を再有効化できません".into());
    }
    cancel(check)?;
    Ok(erased)
}

pub(super) fn verify_rows_deleted(
    db: &Connection,
    expected: &Scope,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    stopped(db)?;
    verify_rows_absent(db, expected, check)
}
// Startup observation is not deletion admission. Unrelated stale running rows
// must not turn a clean completed receipt into an invalid one before recovery.
pub(super) fn verify_rows_absent(
    db: &Connection,
    expected: &Scope,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    expected.validate_format(db)?;
    for (name, predicate) in TARGETS {
        cancel(check)?;
        let exists: bool = db
            .query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM {} WHERE {predicate})",
                    quote(name)
                ),
                params![expected.meeting_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if exists {
            return Err("削除済み会議のDB行が残存または再出現しています".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Progress {
    database_deleted: bool,
    #[serde(default)]
    database_purged: bool,
}

pub(super) fn checkpoint(db: &Connection) -> Result<(), String> {
    let result: (i64, i64, i64) = db
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .map_err(|e| e.to_string())?;
    if result != (0, 0, 0) {
        return Err("DBのWALを回収できません。削除の後処理は未完了です".into());
    }
    Ok(())
}

// Same mutex/admission/file-phase requirements as erase_confirmed_rows.
// This may run after reopening a DB whose row phase committed before a crash.
// It purges application DB/WAL contents, not physical media or user backups.
#[cfg(test)]
pub(super) fn finish_database_purge(
    db: &Connection,
    operation: &str,
    expected: &Scope,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    cancel(check)?;
    if !db.is_autocommit() || !uuid(operation) || !uuid(&expected.meeting_id) {
        return Err("削除後処理の操作・トランザクションが不正です".into());
    }
    expected.validate_format(db)?;
    stopped(db)?;
    let (meeting, manifest, hash, consent, state, progress): (Option<String>, String, String, Option<i64>, String, String) = db.query_row("SELECT meeting_id,target_manifest_json,target_hash,consent_at,state,progress_json FROM maintenance_operations WHERE id=?1 AND kind='delete_meeting' AND state IN ('cleanup','completed')", params![operation], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|e| e.to_string())?;
    let progress: Progress = serde_json::from_str(&progress).map_err(|e| e.to_string())?;
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",
            params![expected.meeting_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if meeting.is_some()
        || exists
        || consent.is_none_or(|n| n < 0)
        || manifest.len() > MAX_MANIFEST
        || digest(manifest.as_bytes()) != hash
        || serde_json::from_str::<Scope>(&manifest).map_err(|e| e.to_string())? != *expected
        || !progress.database_deleted
        || (state == "completed" && !progress.database_purged)
    {
        return Err("DB削除の完了記録を検証できません。後処理を実行しません".into());
    }
    let mode: String = db
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if mode != "wal" {
        return Err("DBのWAL設定が想定と異なります。後処理を実行しません".into());
    }
    secure_delete(db)?;
    cancel(check)?;
    checkpoint(db)?;
    cancel(check)?;
    // VACUUM rebuilds the current DB in place without changing the selected
    // root or retaining a second application copy. Never VACUUM INTO a file.
    db.execute_batch("VACUUM").map_err(|e| e.to_string())?;
    cancel(check)?;
    checkpoint(db)?;
    cancel(check)?;
    if state == "cleanup" {
        db.execute("UPDATE maintenance_operations SET state='completed',progress_json='{\"databaseDeleted\":true,\"databasePurged\":true}',error_code=NULL,updated_at=?2 WHERE id=?1 AND state='cleanup'", params![operation,now_ms()?]).map_err(|e| e.to_string())?;
    }
    // A completed receipt contains no deleted text. Still finish its WAL
    // checkpoint before acknowledging; an acknowledgement retry is safe.
    checkpoint(db)?;
    Ok(())
}
