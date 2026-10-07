//! Full-meeting deletion supersedes checked audio/kept history, never its consent.
//! Original receipts and paths are retained unchanged except the schema's FK
//! detachment. The complete original maintenance fingerprint binds membership.
use super::{
    database,
    fs::{Identity, RootAnchor},
    intent::Prepared,
    renewal, startup,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
const SUBTYPE: &str = "audioHistorySupersession";
const PROGRESS: &str = "{\"version\":1,\"audioHistorySuperseded\":true}";
fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s)
}
fn hash(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"mon/audio-history-supersession/v1\0");
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("音声履歴の引継ぎ検査を取り消しました".into())
    } else {
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Header {
    version: u32,
    subtype: String,
    operation: String,
    delete_operation: String,
    meeting: String,
    scope_hash: String,
    root: Identity,
    database: Identity,
    // Original row order is bound by Scope's native table fingerprint.
    maintenance_ids: Vec<String>,
    audio_operations: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    kept_audio_operations: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    kept_meeting_operations: Vec<String>,
}
// Non-serializable read-only capability. Cannot authorize an unlink, row erase,
// source read, relaxed ready state, or an arbitrary detached meeting FK.
pub(super) struct Proof<'a> {
    header: Header,
    anchor: &'a RootAnchor,
}
impl<'a> Proof<'a> {
    pub(super) fn authorizes(&self, operation: &str, meeting: &str) -> bool {
        self.header.meeting == meeting
            && self
                .header
                .audio_operations
                .iter()
                .any(|id| id == operation)
    }
    pub(super) fn authorizes_kept(&self, operation: &str, meeting: &str) -> bool {
        self.header.meeting == meeting
            && self
                .header
                .kept_audio_operations
                .iter()
                .any(|id| id == operation)
    }
    pub(super) fn matches_root(
        &self,
        db: &Connection,
        operation: &str,
        meeting: &str,
        root: &Identity,
        database: &Identity,
    ) -> Result<bool, String> {
        if !self.authorizes(operation, meeting)
            && !self.authorizes_kept(operation, meeting)
            && !self.authorizes_kept_meeting(operation, meeting)
        {
            return Ok(false);
        }
        // Historical aliases are held, owned Bootstrap facts, never a root
        // supplied by the supersession JSON or an execution permission.
        self.anchor
            .matches_history_root(db, operation, root, database)
    }
    pub(super) fn authorizes_kept_meeting(&self, operation: &str, meeting: &str) -> bool {
        self.header.meeting == meeting
            && self
                .header
                .kept_meeting_operations
                .iter()
                .any(|id| id == operation)
    }
    fn load(
        db: &Connection,
        anchor: &'a RootAnchor,
        operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let proof = Self::load_metadata(db, anchor, operation, check)?;
        let (original, files) =
            startup::supersession_source(db, &proof.header.delete_operation, check)?;
        let roots = renewal::reconfirmation_roots(db, &original, &files, check)?;
        if !startup::absent(anchor, &proof.header.meeting, &roots)? {
            return Err("完全削除済みの会議範囲が再出現しています".into());
        }
        cancel(check)?;
        Ok(proof)
    }
    // The parent full-meeting operation owns physical-scope startup attention;
    // its persistent fence also covers unfinished purge/reconfirmation. This
    // metadata-only read grants no backup, file/source or deletion permission.
    fn load_metadata(
        db: &Connection,
        anchor: &'a RootAnchor,
        operation: &str,
        check: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        // Share this substantial reader across caller-specific cancellation
        // closures; the original probe is still invoked at every checkpoint.
        let check = &|| check();
        cancel(check)?;
        database::trusted_history_schema(db)?;
        let (fk,kind,state,batch,json,sha,consent,progress,error,created,updated):(Option<String>,String,String,Option<String>,Option<String>,String,Option<i64>,Option<String>,Option<String>,i64,i64)=db.query_row("SELECT meeting_id,kind,state,restore_batch_id,CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 THEN target_manifest_json END,target_hash,consent_at,CASE WHEN length(CAST(progress_json AS BLOB))<=4096 THEN progress_json END,error_code,created_at,updated_at FROM maintenance_operations WHERE id=?1",[operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?))).map_err(|e|e.to_string())?;
        let json = json.ok_or("音声履歴引継ぎの容量が上限を超えています")?;
        let h: Header = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        let ids: BTreeSet<_> = h.maintenance_ids.iter().collect();
        let audio: BTreeSet<_> = h.audio_operations.iter().collect();
        let kept: BTreeSet<_> = h.kept_audio_operations.iter().collect();
        let kept_meetings: BTreeSet<_> = h.kept_meeting_operations.iter().collect();
        if !matches!(
            (h.version, kept.is_empty(), kept_meetings.is_empty()),
            (1, true, true) | (2, false, true) | (3, _, false)
        ) || h.subtype != SUBTYPE
            || h.operation != operation
            || !uuid(operation)
            || !uuid(&h.delete_operation)
            || !uuid(&h.meeting)
            || !valid_hash(&h.scope_hash)
            || fk.is_some()
            || kind != "delete_assets"
            || state != "completed"
            || batch.is_some()
            || consent.is_some()
            || error.is_some()
            || progress.as_deref() != Some(PROGRESS)
            || created < 0
            || updated != created
            || serde_json::to_string(&h).map_err(|e| e.to_string())? != json
            || hash(&json) != sha
            || ids.len() != h.maintenance_ids.len()
            || ids.len() > 1024
            || (audio.is_empty() && kept.is_empty() && kept_meetings.is_empty())
            || audio.len() != h.audio_operations.len()
            || kept.len() != h.kept_audio_operations.len()
            || kept_meetings.len() != h.kept_meeting_operations.len()
            || audio.len() + kept.len() + kept_meetings.len() > 256
            || !audio.is_disjoint(&kept)
            || !audio.is_disjoint(&kept_meetings)
            || !kept.is_disjoint(&kept_meetings)
            || ids.iter().any(|id| !uuid(id))
            || !audio.is_subset(&ids)
            || !kept.is_subset(&ids)
            || !kept_meetings.is_subset(&ids)
            || ids.contains(&h.operation)
            || ids.contains(&h.delete_operation)
        {
            return Err("音声履歴の引継ぎ記録が不正です".into());
        }
        h.root.validate(true)?;
        h.database.validate(false)?;
        if !anchor.matches_history_root(db, operation, &h.root, &h.database)? {
            return Err("音声履歴引継ぎの保存領域が一致しません".into());
        }
        let paths: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM maintenance_paths WHERE operation_id=?1)",
                [operation],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if paths {
            return Err("音声履歴引継ぎへ削除対象を登録しません".into());
        }
        let duplicates: i64 = db.query_row(
            "SELECT count(*) FROM (SELECT id FROM maintenance_operations WHERE kind='delete_assets' AND json_extract(target_manifest_json,'$.subtype')='audioHistorySupersession' AND json_extract(target_manifest_json,'$.deleteOperation')=?1 LIMIT 2)",
            [&h.delete_operation], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        if duplicates != 1 {
            return Err("完全削除に対応する音声履歴の引継ぎが重複・欠落しています".into());
        }
        let (original, files) = startup::supersession_source(db, &h.delete_operation, check)?;
        let m = original.manifest();
        if original.scope().meeting_id != h.meeting
            || original.scope().hash()? != h.scope_hash
            || m.root != h.root
            || m.database != h.database
            || database::detached_maintenance_hash(db, &h.meeting, &h.maintenance_ids, check)?
                != original.scope().maintenance_hash
        {
            return Err("完全削除の確認対象と元の保守履歴が一致しません".into());
        }
        renewal::reconfirmation_roots(db, &original, &files, check)?;
        let proof = Self { header: h, anchor };
        let mut expected = BTreeSet::new();
        let mut expected_kept = BTreeSet::new();
        let mut expected_kept_meetings = BTreeSet::new();
        for id in &proof.header.maintenance_ids {
            cancel(check)?;
            let (kind, state): (String, String) = db
                .query_row(
                    "SELECT kind,state FROM maintenance_operations WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|e| e.to_string())?;
            if kind == "delete_assets" && state == "completed" {
                expected.insert(id);
            } else if kind == "delete_assets" && state == "recovered" {
                let subtype: Option<String> = db.query_row(
                    "SELECT json_extract(target_manifest_json,'$.subtype') FROM maintenance_operations WHERE id=?1",
                    [id], |r| r.get(0),
                ).map_err(|e| e.to_string())?;
                if subtype.as_deref() == Some("audioOnlyDeletion") {
                    expected_kept.insert(id);
                }
            } else if kind == "delete_meeting" && state == "recovered" {
                expected_kept_meetings.insert(id);
            }
        }
        if expected != proof.header.audio_operations.iter().collect()
            || expected_kept != proof.header.kept_audio_operations.iter().collect()
            || expected_kept_meetings != proof.header.kept_meeting_operations.iter().collect()
        {
            return Err("完全削除が引き継ぐ音声履歴の集合が一致しません".into());
        }
        for id in &proof.header.audio_operations {
            cancel(check)?;
            if super::audio::restored::hinted(db, id)? {
                super::audio::restored::validate_superseded(db, id, &proof, check)?;
            } else {
                super::audio::intent::removal::validate_superseded(db, id, &proof, check)?;
            }
        }
        for id in &proof.header.kept_audio_operations {
            super::audio::intent::reconfirmation::validate_kept_superseded(db, id, &proof, check)?;
        }
        for id in &proof.header.kept_meeting_operations {
            super::intent::validate_kept_superseded(db, id, &proof, check)?;
        }
        cancel(check)?;
        Ok(proof)
    }
}

// Read-only validation of a retained, never-started operation. The ordinary
// reader and all execution APIs still reject a detached meeting FK.
pub(super) fn inspect_kept(
    db: &Connection,
    anchor: &RootAnchor,
    operation: &str,
    check: &dyn Fn() -> bool,
) -> Result<(), String> {
    let check = &|| check();
    cancel(check)?;
    let meeting: Option<String> = db
        .query_row(
            "SELECT meeting_id FROM maintenance_operations WHERE id=?1",
            [operation],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if meeting.is_some() {
        let saved = super::audio::intent::read(db, operation, check)?;
        saved.verify_kept_history_anchor(db, anchor)?;
        saved.verify_kept_ancestry(db, check)?;
    } else {
        let markers = db.prepare("SELECT id FROM maintenance_operations WHERE kind='delete_assets' AND state='completed' AND json_extract(target_manifest_json,'$.subtype')='audioHistorySupersession' AND EXISTS(SELECT 1 FROM json_each(target_manifest_json,'$.keptAudioOperations') WHERE value=?1) LIMIT 2")
            .map_err(|e| e.to_string())?.query_map([operation], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        if markers.len() != 1 {
            return Err("参照が外れた保持音声履歴の引継ぎ元が欠落・重複しています".into());
        }
        let proof = Proof::load_metadata(db, anchor, &markers[0], check)?;
        if !proof
            .header
            .kept_audio_operations
            .iter()
            .any(|id| id == operation)
        {
            return Err("保持音声履歴が完全削除の確認範囲に所属していません".into());
        }
    }
    cancel(check)
}

// Exact kept-meeting metadata only; no detached Prepared escapes this reader.
pub(super) fn inspect_kept_meeting(
    db: &Connection,
    anchor: &RootAnchor,
    operation: &str,
    check: &dyn Fn() -> bool,
) -> Result<(), String> {
    let check = &|| check();
    cancel(check)?;
    let (meeting, kind, state, batch, error): (Option<String>, String, String, Option<String>, Option<String>) = db.query_row(
        "SELECT meeting_id,kind,state,restore_batch_id,error_code FROM maintenance_operations WHERE id=?1",
        [operation], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    ).map_err(|e| e.to_string())?;
    if kind != "delete_meeting" || state != "recovered" || batch.is_some() || error.is_some() {
        return Err("未開始保持の会議削除の状態・由来が不正です".into());
    }
    if meeting.is_some() {
        let saved = super::intent::read_kept_database(db, operation, check)?;
        if !anchor.matches_history_root(
            db,
            operation,
            &saved.manifest().root,
            &saved.manifest().database,
        )? {
            return Err("保持した会議削除履歴の保存先・DB実体が一致しません".into());
        }
    } else {
        let markers = db.prepare("SELECT id FROM maintenance_operations WHERE kind='delete_assets' AND state='completed' AND json_extract(target_manifest_json,'$.subtype')='audioHistorySupersession' AND EXISTS(SELECT 1 FROM json_each(target_manifest_json,'$.keptMeetingOperations') WHERE value=?1) LIMIT 2")
            .map_err(|e| e.to_string())?.query_map([operation], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        if markers.len() != 1 {
            return Err("参照が外れた保持会議履歴の引継ぎ元が欠落・重複しています".into());
        }
        let proof = Proof::load_metadata(db, anchor, &markers[0], check)?;
        if !proof
            .header
            .kept_meeting_operations
            .iter()
            .any(|id| id == operation)
        {
            return Err("保持会議履歴が完全削除の確認範囲に所属していません".into());
        }
    }
    cancel(check)
}

pub(super) fn verify_detached_kept(
    db: &Connection,
    anchor: &RootAnchor,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    let ids = db.prepare("SELECT id,kind FROM maintenance_operations WHERE state='recovered' AND meeting_id IS NULL AND (kind='delete_meeting' OR (kind='delete_assets' AND coalesce(CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 AND json_valid(target_manifest_json) THEN json_extract(target_manifest_json,'$.subtype') END,'')!='modelDeletion')) ORDER BY id LIMIT 10001")
        .map_err(|e| e.to_string())?.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    if ids.len() > 10_000 {
        return Err("参照が外れた保持削除履歴が検査上限を超えています".into());
    }
    for (id, kind) in ids {
        if kind == "delete_meeting" {
            inspect_kept_meeting(db, anchor, &id, check)?;
        } else {
            inspect_kept(db, anchor, &id, check)?;
        }
    }
    cancel(check)
}

pub(super) fn hinted(db: &Connection, operation: &str) -> Result<bool, String> {
    let json:Option<String>=db.query_row("SELECT CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 THEN target_manifest_json END FROM maintenance_operations WHERE id=?1",[operation],|r|r.get(0)).optional().map_err(|e|e.to_string())?.flatten();
    Ok(json
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .is_some_and(|v| v["subtype"] == SUBTYPE))
}
pub(super) fn inspect(
    db: &Connection,
    anchor: &RootAnchor,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    Proof::load(db, anchor, operation, check).map(|_| ())
}
pub(super) fn superseded(
    db: &Connection,
    anchor: &RootAnchor,
    audio: &str,
    check: &impl Fn() -> bool,
) -> Result<bool, String> {
    match marker_for(db, audio)? {
        None => Ok(false),
        Some(id) => Proof::load(db, anchor, &id, check).map(|_| true),
    }
}
fn marker_for(db: &Connection, audio: &str) -> Result<Option<String>, String> {
    let ids=db.prepare("SELECT m.id FROM maintenance_operations m WHERE kind='delete_assets' AND state='completed' AND json_extract(target_manifest_json,'$.subtype')='audioHistorySupersession' AND EXISTS(SELECT 1 FROM json_each(target_manifest_json,'$.audioOperations') j WHERE j.value=?1) LIMIT 2").map_err(|e|e.to_string())?.query_map([audio],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    if ids.len() > 1 {
        return Err("音声履歴の引継ぎ元が重複しています".into());
    }
    Ok(ids.into_iter().next())
}
pub(super) fn remaining_audit_member(
    db: &Connection,
    anchor: &RootAnchor,
    parent: &str,
    audit: &str,
) -> Result<bool, String> {
    let Some(marker) = marker_for(db, parent)? else {
        return Ok(false);
    };
    let proof = Proof::load_metadata(db, anchor, &marker, &|| false)?;
    if !super::audio::intent::removal::superseded_audit_member(db, parent, audit, &proof)? {
        return Err("完全削除の元音声履歴に接続しない確認記録です".into());
    }
    Ok(true)
}
pub(super) fn startup_parent(
    db: &Connection,
    anchor: &RootAnchor,
    operation: &str,
) -> Result<Option<String>, String> {
    let marker = if hinted(db, operation)? {
        Some(operation.to_owned())
    } else {
        marker_for(db, operation)?
    };
    marker
        .map(|id| {
            Proof::load_metadata(db, anchor, &id, &|| false).map(|p| p.header.delete_operation)
        })
        .transpose()
}

// Called inside the exact fully confirmed row-deletion transaction, while all
// original audio metadata/FKs still exist. Not a client-supplied JSON input.
pub(super) fn retain(
    db: &Connection,
    anchor: &RootAnchor,
    original: &Prepared,
    check: &impl Fn() -> bool,
) -> Result<Option<String>, String> {
    cancel(check)?;
    if db.is_autocommit() {
        return Err("音声履歴の引継ぎはDB削除と同時に確定してください".into());
    }
    let meeting = &original.scope().meeting_id;
    let all:Vec<(String,String,String)>=db.prepare("SELECT id,kind,state FROM maintenance_operations WHERE meeting_id=?1 AND id!=?2 ORDER BY rowid LIMIT 1025").map_err(|e|e.to_string())?.query_map(params![meeting,original.operation()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?.collect::<Result<_,_>>().map_err(|e|e.to_string())?;
    if all.len() > 1024 {
        return Err("音声履歴の引継ぎ対象が安全上限を超えています".into());
    }
    let mut audio = Vec::new();
    let mut kept = Vec::new();
    let mut kept_meetings = Vec::new();
    for (id, kind, state) in &all {
        cancel(check)?;
        if kind == "delete_meeting" && state == "recovered" {
            inspect_kept_meeting(db, anchor, id, check)?;
            kept_meetings.push(id.clone());
            continue;
        }
        if kind == "delete_assets" && state == "recovered" {
            let subtype: Option<String> = db.query_row(
                "SELECT json_extract(target_manifest_json,'$.subtype') FROM maintenance_operations WHERE id=?1",
                [id], |r| r.get(0),
            ).map_err(|e| e.to_string())?;
            if subtype.as_deref() == Some("audioOnlyDeletion") {
                inspect_kept(db, anchor, id, check)?;
                kept.push(id.clone());
            }
            continue;
        }
        if kind != "delete_assets" || state != "completed" {
            continue;
        }
        if super::audio::restored::hinted(db, id)? {
            if super::audio::restored::backup_meeting(db, anchor, id, check)? != *meeting {
                return Err("引継ぐ音声履歴の会議が一致しません".into());
            }
        } else if super::audio::intent::removal::completion::inspect_one(db, anchor, id)?.is_some()
        {
            return Err("引継ぐ音声削除の完了証拠を検証できません".into());
        }
        audio.push(id.clone());
    }
    if audio.is_empty() && kept.is_empty() && kept_meetings.is_empty() {
        return Ok(None);
    }
    let (root, database) = anchor.identity()?;
    if (root.clone(), database.clone())
        != (
            original.manifest().root.clone(),
            original.manifest().database.clone(),
        )
    {
        return Err("引継ぎと完全削除の管理領域が一致しません".into());
    }
    let operation = uuid::Uuid::new_v4().to_string();
    let header = Header {
        version: if !kept_meetings.is_empty() {
            3
        } else if kept.is_empty() {
            1
        } else {
            2
        },
        subtype: SUBTYPE.into(),
        operation: operation.clone(),
        delete_operation: original.operation().into(),
        meeting: meeting.clone(),
        scope_hash: original.scope().hash()?,
        root,
        database,
        maintenance_ids: all.into_iter().map(|r| r.0).collect(),
        audio_operations: audio,
        kept_audio_operations: kept,
        kept_meeting_operations: kept_meetings,
    };
    let json = serde_json::to_string(&header).map_err(|e| e.to_string())?;
    if json.len() > 65536
        || header.audio_operations.len()
            + header.kept_audio_operations.len()
            + header.kept_meeting_operations.len()
            > 256
    {
        return Err("音声履歴引継ぎが容量上限を超えています".into());
    }
    let now = crate::storage::now_ms()?;
    db.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,'delete_assets','completed',?2,?3,?4,?5,?5)",params![operation,json,hash(&json),PROGRESS,now]).map_err(|e|e.to_string())?;
    cancel(check)?;
    Ok(Some(operation))
}

#[cfg(test)]
#[path = "audio_supersession_tests.rs"]
mod tests;
