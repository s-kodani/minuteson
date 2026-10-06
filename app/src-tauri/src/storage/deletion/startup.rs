//! Read-only completed-receipt discovery before mutable startup recovery.
//! TEMP rows only hold attention hints; no scope or consent is rewritten.
use super::{completion, database, fs::RootAnchor, intent::Prepared, removal, renewal};
use crate::storage::Storage;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeMap;

const MAX_COMPLETED: usize = 10_000;
const REAPPEARED: &str = "completed_scope_reappeared";
const INVALID: &str = "completed_receipt_invalid";
const UNINSPECTABLE: &str = "completed_scope_uninspectable";
const CREATE_ATTENTION: &str = "CREATE TEMP TABLE mon_deletion_startup_attention(operation_id TEXT PRIMARY KEY NOT NULL,reason TEXT NOT NULL);";

type TempSchema = Vec<(String, String, String, Option<String>)>;
fn temp_schema(db: &Connection) -> Result<TempSchema, String> {
    db.prepare("SELECT type,name,tbl_name,sql FROM sqlite_temp_schema ORDER BY type,name")
        .map_err(|e| e.to_string())?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
}
pub(super) fn verify_temp_schema(db: &Connection) -> Result<(), String> {
    let actual = temp_schema(db)?;
    if actual.is_empty() {
        return Ok(());
    }
    // Do not whitelist a name: verify every TEMP table, index and trigger
    // against this binary's exact isolated schema. Shadow tables stay denied.
    static EXPECTED: std::sync::OnceLock<Result<TempSchema, String>> = std::sync::OnceLock::new();
    let expected = EXPECTED
        .get_or_init(|| {
            let reference = Connection::open_in_memory().map_err(|e| e.to_string())?;
            reference
                .execute_batch(CREATE_ATTENTION)
                .map_err(|e| e.to_string())?;
            temp_schema(&reference)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    if &actual != expected {
        return Err("削除対象DBに未対応の一時オブジェクトがあります".into());
    }
    Ok(())
}

fn exists(db: &Connection) -> Result<bool, String> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_schema WHERE type='table' AND name='mon_deletion_startup_attention')", [], |r| r.get(0)).map_err(|e| e.to_string())
}
pub(super) fn reason(db: &Connection, operation: &str) -> Result<Option<String>, String> {
    if !exists(db)? {
        return Ok(None);
    }
    db.query_row(
        "SELECT reason FROM temp.mon_deletion_startup_attention WHERE operation_id=?1",
        params![operation],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}
pub(super) fn contains(db: &Connection, operation: &str) -> Result<bool, String> {
    Ok(reason(db, operation)?.is_some())
}
pub(super) fn verify_completed_rows(
    storage: &Storage,
    db: &Connection,
    original: &Prepared,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    // Only a cold, previously completed receipt can have unrelated stale
    // running rows whose normal recovery was deliberately deferred. This is
    // an absence check, not row deletion or a runtime-worker admission.
    // Native execution still requires the real shared maintenance lease.
    if storage.deletion_fence.resume_restart_required() {
        // A new, explicitly accepted file review temporarily changes the
        // source to installing. Its canonical progress must retain the fully
        // finished DB phase, including after another cold restart when TEMP
        // hints no longer exist. A hint never substitutes for this proof.
        let (current, files) = source(db, original.operation())?;
        original.matches_saved(&current)?;
        let finished = files.database_phase == Some(completion::Phase::Finished)
            || completion::Receipt::load(db, original.operation(), check)
                .is_ok_and(|receipt| receipt.completed());
        if finished {
            return database::verify_rows_absent(db, original.scope(), check);
        }
    }
    database::verify_rows_deleted(db, original.scope(), check)
}
fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s)
}

// Loading validates the entire canonical source and append-only audit chain.
// A source's raw state/version is only a routing hint, never an authority.
fn source(db: &Connection, operation: &str) -> Result<(Prepared, removal::Progress), String> {
    let (state, progress): (String, String) = db.query_row("SELECT state,progress_json FROM maintenance_operations WHERE id=?1 AND kind='delete_meeting'", params![operation], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e| e.to_string())?;
    if progress.len() > 4096 {
        return Err("削除進捗が容量上限を超えています".into());
    }
    if state == "installing"
        || serde_json::from_str::<serde_json::Value>(&progress)
            .ok()
            .is_some_and(|p| p["version"] == 3)
    {
        let receipt = removal::Receipt::load(db, operation, &|| false)?;
        Ok((receipt.confirmed().clone(), receipt.progress))
    } else {
        let receipt = completion::Receipt::load(db, operation, &|| false)?;
        Ok((receipt.confirmed().clone(), receipt.file_progress().clone()))
    }
}
pub(super) fn absent(anchor: &RootAnchor, meeting: &str, roots: &[String]) -> Result<bool, String> {
    anchor.verify()?;
    let Some(meetings) = anchor.directory_ref()?.child("meetings")? else {
        return Ok(true);
    };
    // Test presence without following links or treating a regular file as a
    // safe directory. Do not enumerate siblings or read their contents.
    if meetings.optional_kind(meeting)?.is_some() {
        return Ok(false);
    }
    for path in roots {
        if meetings
            .optional_kind(path.strip_prefix("meetings/").ok_or("予約名が不正です")?)?
            .is_some()
        {
            return Ok(false);
        }
    }
    anchor.verify()?;
    Ok(true)
}

// Typed row-phase evidence, including a subsequent explicitly accepted v3
// file reconfirmation. A raw completed/version label is never sufficient.
pub(super) fn supersession_source(
    db: &Connection,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<(Prepared, removal::Progress), String> {
    let (original, files) = source(db, operation)?;
    let committed = files.database_phase.is_some_and(|p| p.rows_committed())
        || completion::Receipt::load(db, operation, check)?.row_phase_committed();
    if !committed {
        return Err("音声履歴の引継ぎより先に会議DB削除を確定してください".into());
    }
    database::verify_rows_absent(db, original.scope(), check)?;
    Ok((original, files))
}

pub(in crate::storage) fn inspect(
    db: &Connection,
    anchor: &RootAnchor,
    models_root: &std::path::Path,
) -> Result<(), String> {
    if exists(db)? {
        return Err("起動時の削除確認情報を再採用できません".into());
    }
    let attention = completed_attention(db, anchor, models_root)?;
    if attention.is_empty() {
        return Ok(());
    }
    // In-memory TEMP metadata cannot change canonical main-schema receipts.
    db.execute_batch("PRAGMA temp_store=MEMORY;")
        .map_err(|e| e.to_string())?;
    db.execute_batch(CREATE_ATTENTION)
        .map_err(|e| e.to_string())?;
    for (operation, reason) in attention {
        db.execute(
            "INSERT INTO temp.mon_deletion_startup_attention VALUES(?1,?2)",
            params![operation, reason],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Reuse the typed startup readers against a query-only relocation image.
// Discovery itself never creates TEMP hints or grants root/cleanup authority.
pub(super) fn completed_attention(
    db: &Connection,
    anchor: &RootAnchor,
    models_root: &std::path::Path,
) -> Result<Vec<(String, &'static str)>, String> {
    let present: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='maintenance_operations')", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    let mut attention = Vec::new();
    if present {
        let mut q = db.prepare("SELECT id,CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 THEN target_manifest_json ELSE '{}' END,CASE WHEN length(CAST(progress_json AS BLOB))<=4096 THEN progress_json ELSE '{}' END FROM maintenance_operations WHERE kind='delete_meeting' AND state='completed' ORDER BY created_at,id LIMIT 10001").map_err(|e| e.to_string())?;
        let mut rows = q.query([]).map_err(|e| e.to_string())?;
        let mut count = 0;
        // Cache only within this locked, synchronous startup scan. A later
        // writer must load the live typed receipt again; this grants nothing.
        let mut linked: BTreeMap<String, Result<Vec<renewal::Reference>, String>> = BTreeMap::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            count += 1;
            if count > MAX_COMPLETED {
                return Err(
                    "完了済み削除履歴が起動時検査の上限を超えています。対象を保持します".into(),
                );
            }
            let (operation, json, progress): (String, String, String) = (
                row.get(0).map_err(|e| e.to_string())?,
                row.get(1).map_err(|e| e.to_string())?,
                row.get(2).map_err(|e| e.to_string())?,
            );
            if !uuid(&operation) {
                return Err("完了済み削除の操作IDが不正です。対象を保持します".into());
            }
            let result = (|| -> Result<Option<&str>, String> {
                if json.len() > 65536 || progress.len() > 4096 {
                    return Err("削除履歴が容量上限を超えています".into());
                }
                if progress == "{\"confirmationRecorded\":true}" {
                    let parent = renewal::audit_source(&json)?;
                    let audits = linked
                        .entry(parent.clone())
                        .or_insert_with(|| {
                            let (original, files) = source(db, &parent)?;
                            renewal::audit_references(db, &original, &files)
                        })
                        .as_ref()
                        .map_err(Clone::clone)?;
                    if !audits.iter().any(|r| r.operation_id == operation) {
                        return Err("元の削除計画に接続していない確認記録です".into());
                    }
                    return Ok(None);
                }
                // Old row-only receipts are internal legacy data, not public
                // file consent. Validate them rather than ignoring a marker.
                if let Ok(scope) = serde_json::from_str::<database::Scope>(&json) {
                    let (meeting, sha, consent, created, updated): (Option<String>,String,Option<i64>,i64,i64) = db.query_row("SELECT meeting_id,target_hash,consent_at,created_at,updated_at FROM maintenance_operations WHERE id=?1",params![operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|e|e.to_string())?;
                    if scope.encode()? != json
                        || scope.hash()? != sha
                        || meeting.is_some()
                        || consent.is_none_or(|n| n < 0)
                        || created < 0
                        || updated < created
                        || progress != "{\"databaseDeleted\":true,\"databasePurged\":true}"
                    {
                        return Err("DBだけの完了記録が不正です".into());
                    }
                    database::verify_rows_absent(db, &scope, &|| false)?;
                    return Ok(None);
                }
                let (original, files) = source(db, &operation)?;
                if !anchor.matches_history_root(
                    db,
                    &operation,
                    &original.manifest().root,
                    &original.manifest().database,
                )? {
                    return Err("完了した会議削除の保存領域・DB実体が一致しません".into());
                }
                // Canonical v3 file-only completion still awaits the DB
                // wrapper/acknowledgement. The existing unfinished fence
                // already covers it; do not label it a corrupt final receipt.
                if serde_json::from_str::<serde_json::Value>(&progress)
                    .is_ok_and(|p| p["version"] == 3)
                {
                    return Ok(None);
                }
                database::verify_rows_absent(db, original.scope(), &|| false)?;
                let roots = renewal::reconfirmation_roots(db, &original, &files, &|| false)?;
                match absent(anchor, &original.scope().meeting_id, &roots) {
                    Ok(true) => Ok(None),
                    Ok(false) => Ok(Some(REAPPEARED)),
                    Err(_) => Ok(Some(UNINSPECTABLE)),
                }
            })();
            match result {
                Ok(None) => {}
                Ok(Some(reason)) => attention.push((operation, reason)),
                Err(_) => attention.push((operation, INVALID)),
            }
        }
    }
    anchor.verify()?;
    if present {
        attention.extend(super::models::startup::inspect(db, anchor, models_root)?);
    }
    anchor.verify()?;
    Ok(attention)
}

pub(super) fn resolve(
    storage: &Storage,
    db: &Connection,
    receipt: &completion::Receipt,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    let operation = receipt.confirmed().operation();
    if !contains(db, operation)? {
        return Ok(());
    }
    receipt.verify_saved(db, check)?;
    if !receipt.completed() {
        return Err("起動時検出した削除は未完了です".into());
    }
    verify_completed_rows(storage, db, receipt.confirmed(), check)?;
    let roots =
        renewal::reconfirmation_roots(db, receipt.confirmed(), receipt.file_progress(), check)?;
    if check()
        || !absent(
            &storage.managed_anchor,
            &receipt.confirmed().scope().meeting_id,
            &roots,
        )?
    {
        return Err("削除の最終確認後に対象が再出現しました。確認待ちを維持します".into());
    }
    db.execute(
        "DELETE FROM temp.mon_deletion_startup_attention WHERE operation_id=?1",
        params![operation],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;
