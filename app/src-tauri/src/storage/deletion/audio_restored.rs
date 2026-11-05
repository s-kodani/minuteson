//! Read-only unavailability after a verified additive restore. This receipt
//! cannot be loaded by audio deletion/removal APIs and carries no consent.
use super::super::{
    database::{self, Table},
    fs::{Identity, RootAnchor},
};
use crate::storage::backup::{ledger, restored_audio::Seed};
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const SUBTYPE: &str = "restoredAudioUnavailable";
const PROGRESS: &str = "{\"version\":1,\"sourceAudioUnavailable\":true}";
const MAX_ASSETS: usize = 200_000;

fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s)
}
fn hash_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("音声利用不可の検証を取り消しました".into())
    } else {
        Ok(())
    }
}
fn digest(json: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"mon/restored-audio-unavailable/v1\0");
    h.update(json.as_bytes());
    format!("{:x}", h.finalize())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::storage) struct History {
    tables: Vec<Table>,
    assets: BTreeSet<String>,
}
impl History {
    pub(in crate::storage) fn capture(
        db: &Connection,
        meeting: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let tables = database::audio_tables(db, meeting, check)?;
        let available: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%' AND state!='missing') OR EXISTS(SELECT 1 FROM audio_chunks WHERE meeting_id=?1 AND state!='missing') OR EXISTS(SELECT 1 FROM audio_tracks WHERE meeting_id=?1 AND state!='failed')", [meeting], |r|r.get(0)).map_err(|e|e.to_string())?;
        if available {
            return Err("復元履歴の音声利用状態が復活しています".into());
        }
        let mut query = db.prepare("SELECT id,relative_path,size_bytes,content_hash FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%' ORDER BY id LIMIT 200001").map_err(|e|e.to_string())?;
        let mut rows = query.query([meeting]).map_err(|e| e.to_string())?;
        let mut assets = BTreeSet::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            cancel(check)?;
            let (id, path, size, hash): (String, String, i64, String) = (
                row.get(0).map_err(|e| e.to_string())?,
                row.get(1).map_err(|e| e.to_string())?,
                row.get(2).map_err(|e| e.to_string())?,
                row.get(3).map_err(|e| e.to_string())?,
            );
            if !uuid(&id)
                || path != format!("meetings/{meeting}/restored/{id}")
                || size < 0
                || !hash_valid(&hash)
                || !assets.insert(id)
                || assets.len() > MAX_ASSETS
            {
                return Err("復元履歴の音声資産・管理パスが不正です".into());
            }
        }
        cancel(check)?;
        Ok(Self { tables, assets })
    }
    pub(in crate::storage) fn asset_ids(&self) -> BTreeSet<String> {
        self.assets.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Header {
    version: u32,
    subtype: String,
    operation: String,
    meeting: String,
    source_archive: String,
    source_meeting: String,
    restore_batch: String,
    archive_fingerprint: String,
    tables: Vec<Table>,
    root: Identity,
    database: Identity,
    parents: Vec<Identity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Receipt {
    header: Header,
    history: History,
}
impl Receipt {
    fn load(db: &Connection, operation: &str, check: &impl Fn() -> bool) -> Result<Self, String> {
        let header = Self::read_header(db, operation, None, check)?;
        let history = History::capture(db, &header.meeting, check)?;
        if history.tables != header.tables {
            return Err("復元音声の資産・区間・トラック履歴が変更されています".into());
        }
        cancel(check)?;
        Ok(Self { header, history })
    }
    fn read_header(
        db: &Connection,
        operation: &str,
        supersession: Option<&crate::storage::deletion::audio_supersession::Proof>,
        check: &impl Fn() -> bool,
    ) -> Result<Header, String> {
        cancel(check)?;
        database::trusted_schema(db)?;
        let (meeting,kind,state,batch,json,hash,consent,progress,error,created,updated):
            (Option<String>,String,String,Option<String>,Option<String>,String,Option<i64>,Option<String>,Option<String>,i64,i64) = db.query_row(
            "SELECT meeting_id,kind,state,restore_batch_id,CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 THEN target_manifest_json END,target_hash,consent_at,CASE WHEN length(CAST(progress_json AS BLOB))<=4096 THEN progress_json END,error_code,created_at,updated_at FROM maintenance_operations WHERE id=?1",
            [operation], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?)),
        ).map_err(|e|e.to_string())?;
        let json = json.ok_or("復元音声履歴の容量が上限を超えています")?;
        let header: Header = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if header.version != 1
            || header.subtype != SUBTYPE
            || header.operation != operation
            || !uuid(operation)
            || [
                &header.meeting,
                &header.source_archive,
                &header.source_meeting,
                &header.restore_batch,
            ]
            .iter()
            .any(|s| !uuid(s))
            || !hash_valid(&header.archive_fingerprint)
            || serde_json::to_string(&header).map_err(|e| e.to_string())? != json
            || digest(&json) != hash
            || !match supersession {
                None => meeting.as_deref() == Some(&header.meeting),
                Some(proof) => meeting.is_none() && proof.authorizes(operation, &header.meeting),
            }
            || kind != "delete_assets"
            || state != "completed"
            || batch.is_some()
            || consent.is_some()
            || error.is_some()
            || progress.as_deref() != Some(PROGRESS)
            || created < 0
            || updated != created
            || header.parents.len() != 3
            || header.tables.len() != 3
            || header
                .tables
                .iter()
                .zip(["audio_chunks", "audio_tracks", "file_assets"])
                .any(|(t, name)| {
                    t.name != name || t.rows > MAX_ASSETS as u64 || !hash_valid(&t.sha256)
                })
        {
            return Err("復元音声の利用不可記録が不正です。削除要求として採用しません".into());
        }
        header.root.validate(true)?;
        header.database.validate(false)?;
        for p in &header.parents {
            p.validate(true)?;
        }
        let paths: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM maintenance_paths WHERE operation_id=?1)",
                [operation],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if paths {
            return Err("復元音声履歴に削除対象を登録しません".into());
        }
        let published = ledger::verify_restored_audio_origin(
            db,
            &header.restore_batch,
            &header.source_archive,
            &header.archive_fingerprint,
            &header.source_meeting,
            &header.meeting,
            operation,
            check,
        )?;
        if published != created {
            return Err("復元音声履歴の公開時刻が一致しません".into());
        }
        cancel(check)?;
        Ok(header)
    }
    fn absent(
        &self,
        db: &Connection,
        anchor: &RootAnchor,
        check: &impl Fn() -> bool,
    ) -> Result<bool, String> {
        let h = &self.header;
        if !anchor.matches_history_root(db, &h.operation, &h.root, &h.database)? {
            return Err("復元音声履歴の保存領域・DB実体が一致しません".into());
        }
        let mut dir = anchor.directory_ref()?.child("meetings")?;
        for (i, component) in ["meetings", h.meeting.as_str(), "restored"]
            .iter()
            .enumerate()
        {
            cancel(check)?;
            let Some(current) = dir else {
                anchor.verify()?;
                return Ok(true);
            };
            let path = match i {
                0 => "meetings".to_string(),
                1 => format!("meetings/{}", h.meeting),
                _ => format!("meetings/{}/restored", h.meeting),
            };
            anchor.verify_history_directory(db, &h.operation, &path, &current, &h.parents[i])?;
            if i == 2 {
                for id in &self.history.assets {
                    cancel(check)?;
                    if current.optional_kind(id)?.is_some() {
                        return Ok(false);
                    }
                }
                current.verify()?;
                anchor.verify()?;
                cancel(check)?;
                return Ok(true);
            }
            // Component zero was already opened above.
            let next = if *component == "meetings" {
                h.meeting.as_str()
            } else {
                "restored"
            };
            dir = current.child(next)?;
        }
        unreachable!()
    }
}
// A restored receipt is an immutable source of unavailability and exact asset
// paths, NEVER a deletion consent/removal receipt. No serde grant or writer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::storage::deletion) struct Source {
    receipt: Receipt,
    row: (i64, i64),
    seal: String,
    bytes: usize,
}
impl Source {
    pub(in crate::storage::deletion) fn load(
        db: &Connection,
        operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let receipt = Receipt::load(db, operation, check)?;
        let row = db
            .query_row(
                "SELECT rowid,created_at FROM maintenance_operations WHERE id=?1",
                [operation],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let json = serde_json::to_vec(&("restored_audio_unavailable_source", row, &receipt.header))
            .map_err(|e| e.to_string())?;
        // Include bounded exact paths in the chain's aggregate byte budget.
        let path_bytes = receipt.history.assets.iter().try_fold(0usize, |n, id| {
            n.checked_add(format!("meetings/{}/restored/{id}", receipt.header.meeting).len())
                .ok_or("復元音声の履歴容量が範囲外です")
        })?;
        let bytes = json
            .len()
            .checked_add(path_bytes)
            .ok_or("復元音声の履歴容量が範囲外です")?;
        cancel(check)?;
        Ok(Self {
            receipt,
            row,
            seal: format!("{:x}", Sha256::digest(&json)),
            bytes,
        })
    }
    pub(in crate::storage::deletion) fn operation(&self) -> &str {
        &self.receipt.header.operation
    }
    pub(in crate::storage::deletion) fn meeting(&self) -> &str {
        &self.receipt.header.meeting
    }
    pub(in crate::storage::deletion) fn published(&self) -> i64 {
        self.row.1
    }
    pub(in crate::storage::deletion) fn seal(&self) -> &str {
        &self.seal
    }
    pub(in crate::storage::deletion) fn bytes(&self) -> usize {
        self.bytes
    }
    pub(in crate::storage::deletion) fn paths(&self) -> BTreeSet<String> {
        self.receipt
            .history
            .assets
            .iter()
            .map(|id| format!("meetings/{}/restored/{id}", self.meeting()))
            .collect()
    }
    pub(in crate::storage::deletion) fn verify_anchor(
        &self,
        db: &Connection,
        anchor: &RootAnchor,
    ) -> Result<(), String> {
        let h = &self.receipt.header;
        if !anchor.matches_history_root(db, &h.operation, &h.root, &h.database)? {
            return Err("復元音声の元履歴と現在の所有領域が一致しません".into());
        }
        Ok(())
    }
}
// Metadata only under a typed full-meeting supersession. The frozen first
// native child's original observation supplies the exact restored asset IDs;
// this value is deliberately not a live Source, Receipt or file capability.
pub(in crate::storage::deletion::audio) struct SupersededBaseline {
    pub(in crate::storage::deletion::audio) paths: BTreeSet<String>,
    pub(in crate::storage::deletion::audio) seal: String,
    pub(in crate::storage::deletion::audio) bytes: usize,
    pub(in crate::storage::deletion::audio) published: i64,
    pub(in crate::storage::deletion::audio) meeting: String,
}
pub(in crate::storage::deletion::audio) fn superseded_baseline(
    db: &Connection,
    operation: &str,
    proof: &crate::storage::deletion::audio_supersession::Proof,
    child: &super::Snapshot,
    check: &impl Fn() -> bool,
) -> Result<SupersededBaseline, String> {
    let header = Receipt::read_header(db, operation, Some(proof), check)?;
    if !proof.matches_root(
        db,
        operation,
        &header.meeting,
        &header.root,
        &header.database,
    )? || header.meeting != child.scope.meeting_id
        || header.tables[2].rows != child.records.audio_assets.len() as u64
        || child.records.audio_assets.iter().any(|(path, id)| {
            !uuid(id) || *path != format!("meetings/{}/restored/{id}", header.meeting)
        })
    {
        return Err("引き継ぐ復元音声の資産集合・管理領域が一致しません".into());
    }
    let paths: BTreeSet<_> = child.records.audio_assets.keys().cloned().collect();
    let ids: BTreeSet<_> = child.records.audio_assets.values().collect();
    if ids.len() != paths.len() || paths.len() > MAX_ASSETS {
        return Err("引き継ぐ復元音声の資産IDが重複・範囲外です".into());
    }
    let row: (i64, i64) = db
        .query_row(
            "SELECT rowid,created_at FROM maintenance_operations WHERE id=?1",
            [operation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let json = serde_json::to_vec(&("restored_audio_unavailable_source", row, &header))
        .map_err(|e| e.to_string())?;
    let bytes = paths.iter().try_fold(json.len(), |n, path| {
        n.checked_add(path.len())
            .ok_or("復元音声の履歴容量が範囲外です")
    })?;
    cancel(check)?;
    Ok(SupersededBaseline {
        paths,
        seal: format!("{:x}", Sha256::digest(&json)),
        bytes,
        published: row.1,
        meeting: header.meeting,
    })
}
pub(in crate::storage::deletion) fn validate_superseded(
    db: &Connection,
    operation: &str,
    proof: &crate::storage::deletion::audio_supersession::Proof,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    let header = Receipt::read_header(db, operation, Some(proof), check)?;
    if !proof.matches_root(
        db,
        operation,
        &header.meeting,
        &header.root,
        &header.database,
    )? {
        return Err("引き継ぐ復元音声履歴の管理領域が一致しません".into());
    }
    cancel(check)
}

// Route hints never grant admission: every caller follows this with typed load.
pub(in crate::storage) fn hinted(db: &Connection, operation: &str) -> Result<bool, String> {
    let json: Option<String> = db.query_row("SELECT CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 THEN target_manifest_json END FROM maintenance_operations WHERE id=?1",[operation],|r|r.get(0)).map_err(|e|e.to_string())?;
    Ok(json
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .is_some_and(|v| v["subtype"] == SUBTYPE))
}
pub(in crate::storage) fn deleted(
    db: &Connection,
    operation: &str,
    meeting: &str,
) -> Result<(), String> {
    let receipt = Receipt::load(db, operation, &|| false)?;
    if receipt.header.meeting != meeting {
        return Err("復元音声履歴の会議が一致しません".into());
    }
    Ok(())
}
pub(in crate::storage) fn backup_meeting(
    db: &Connection,
    anchor: &RootAnchor,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<String, String> {
    let receipt = Receipt::load(db, operation, check)?;
    if !receipt.absent(db, anchor, check)? {
        return Err("復元済みの欠落音声が再出現しています。黙って除外しません".into());
    }
    Ok(receipt.header.meeting)
}
pub(in crate::storage) fn inspect_one(
    db: &Connection,
    anchor: &RootAnchor,
    operation: &str,
) -> Result<Option<&'static str>, String> {
    Ok(match Receipt::load(db, operation, &|| false) {
        Err(_) => Some("restored_audio_receipt_invalid"),
        Ok(receipt) => match receipt.absent(db, anchor, &|| false) {
            Ok(true) => None,
            Ok(false) => Some("restored_audio_scope_reappeared"),
            Err(_) => Some("restored_audio_scope_uninspectable"),
        },
    })
}

// The caller's transaction must already contain the appended meetings and the
// published batch. No autocommit insert and no operation chosen by JSON.
pub(in crate::storage) fn retain(
    tx: &Transaction<'_>,
    anchor: &RootAnchor,
    seed: &Seed,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    cancel(check)?;
    let mut prepared = Vec::new();
    for m in seed.meetings() {
        let history = History::capture(tx, &m.target, check)?;
        if history.tables != m.history.tables || history.assets != m.history.assets {
            return Err("追加公開後の音声履歴が封印した履歴と一致しません".into());
        }
        let published = ledger::verify_restored_audio_origin(
            tx,
            seed.batch(),
            seed.archive(),
            seed.fingerprint(),
            &m.source,
            &m.target,
            &m.operation,
            check,
        )?;
        let (root, database) = anchor.identity()?;
        let meetings = anchor
            .directory_ref()?
            .child("meetings")?
            .ok_or("復元会議の親がありません")?;
        let meeting = meetings
            .child(&m.target)?
            .ok_or("復元会議の管理フォルダーがありません")?;
        let restored = meeting
            .child("restored")?
            .ok_or("復元音声の親がありません")?;
        let header = Header {
            version: 1,
            subtype: SUBTYPE.into(),
            operation: m.operation.clone(),
            meeting: m.target.clone(),
            source_archive: seed.archive().into(),
            source_meeting: m.source.clone(),
            restore_batch: seed.batch().into(),
            archive_fingerprint: seed.fingerprint().into(),
            tables: history.tables.clone(),
            root,
            database,
            parents: vec![
                meetings.identity.clone(),
                meeting.identity.clone(),
                restored.identity.clone(),
            ],
        };
        let receipt = Receipt { header, history };
        if !receipt.absent(tx, anchor, check)? {
            return Err("利用不可記録の確定前に音声が再出現しました".into());
        }
        let json = serde_json::to_string(&receipt.header).map_err(|e| e.to_string())?;
        if json.len() > 65536 {
            return Err("復元音声履歴が容量上限を超えています".into());
        }
        prepared.push((
            m.operation.clone(),
            m.target.clone(),
            digest(&json),
            json,
            published,
        ));
    }
    for (operation, meeting, hash, json, published) in prepared {
        cancel(check)?;
        tx.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_assets','completed',?3,?4,?5,?6,?6)",params![operation,meeting,json,hash,PROGRESS,published]).map_err(|e|e.to_string())?;
        let receipt = Receipt::load(tx, &operation, check)?;
        if !receipt.absent(tx, anchor, check)? {
            return Err("利用不可記録の保存後に音声が再出現しました".into());
        }
    }
    cancel(check)
}

// Final publication recheck after every metadata pass, with the same private
// sealed input. Neither a route hint nor a loose meeting flag is sufficient.
pub(in crate::storage) fn verify_retained(
    tx: &Transaction<'_>,
    anchor: &RootAnchor,
    seed: &Seed,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    for m in seed.meetings() {
        cancel(check)?;
        let receipt = Receipt::load(tx, &m.operation, check)?;
        let h = &receipt.header;
        if h.restore_batch != seed.batch()
            || h.source_archive != seed.archive()
            || h.archive_fingerprint != seed.fingerprint()
            || h.source_meeting != m.source
            || h.meeting != m.target
            || receipt.history.tables != m.history.tables
            || receipt.history.assets != m.history.assets
            || !receipt.absent(tx, anchor, check)?
        {
            return Err("確定直前の音声利用不可記録が封印した履歴と一致しません".into());
        }
    }
    cancel(check)
}
