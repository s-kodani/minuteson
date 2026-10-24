//! Exact local history for relocation. No deserialization, cleanup capability,
//! destination inode adoption, rewritten consent, or portable archive content.
use super::*;
use crate::storage::backup::publication::journal;

const MAX_BYTES: i64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(in crate::storage::backup) struct Record {
    pub id: String,
    pub path: String,
    pub proof: String,
    pub hash: String,
    pub state: String,
    pub error: Option<String>,
    pub entry_count: i64,
    pub entries: Vec<Entry>,
    pub intent_hash: String,
    // Valid historical records still do not own a destination inode.
    pub record_valid: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(in crate::storage::backup) struct Entry {
    pub path: String,
    pub kind: String,
    pub proof: String,
    pub hash: String,
    pub removed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(in crate::storage::backup) struct Keep {
    pub source: String,
    pub operation_id: String,
    pub intent_hash: String,
    pub receipt_json: String,
    pub receipt_hash: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(in crate::storage::backup) struct Restore {
    pub operation_id: String,
    pub intent_hash: String,
    pub status_hash: String,
    pub state: String,
    pub published: bool,
    pub record_valid: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(in crate::storage::backup) struct Bundle {
    version: u32,
    source_root: PathBuf,
    source_identity: Identity,
    pub areas: Vec<Record>,
    pub decisions: Vec<Keep>,
    pub restores: Vec<Restore>,
    pub remaining_root_objects: Vec<RootObject>,
    pub origins: Vec<history::Origin>,
    pub bindings: Vec<binding::Record>,
}
impl Bundle {
    pub(in crate::storage::backup) fn local_json(&self) -> Result<String, String> {
        let mut value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        value
            .as_object_mut()
            .ok_or("移行履歴の形式が不正です")?
            .remove("origins");
        serde_json::to_string(&value).map_err(|e| e.to_string())
    }
}

// Shallow observations only. These names remain at the source and are never a
// recursive deletion manifest. Links are recorded without opening their target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(in crate::storage::backup) struct RootObject {
    path: String,
    kind: String,
    identity: Identity,
    size: u64,
    mode: u32,
    links: u64,
    change: (i64, i64),
    modified: std::time::SystemTime,
}
fn remaining(
    root: &Path,
    excluded: &str,
    check: &impl Fn() -> bool,
) -> Result<Vec<RootObject>, String> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    let mut result = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        cancelled(check)?;
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry
            .file_name()
            .into_string()
            .map_err(|_| "旧保存先の対象名を表示できません")?;
        if path == excluded
            || matches!(
                path.as_str(),
                "meetings"
                    | "temp"
                    | "meetings.sqlite3"
                    | "meetings.sqlite3-wal"
                    | "meetings.sqlite3-shm"
            )
        {
            continue;
        }
        if result.len() >= LIMIT || path.len() > 1024 {
            return Err("旧保存先の残存対象が安全上限を超えています".into());
        }
        let m = crate::native_fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        let registry = path == ".mon-scratch.sqlite3";
        result.push(RootObject {
            path,
            kind: if registry {
                "source_registry"
            } else if m.is_dir() {
                "directory"
            } else if m.is_file() {
                "file"
            } else if m.file_type().is_symlink() {
                "link"
            } else {
                "other"
            }
            .into(),
            identity: Identity::read(&m)?,
            // The excluded native-stage registration changes this DB's bytes;
            // its included history and immutable identity are checked separately.
            size: if registry { 0 } else { m.len() },
            mode: m.mode(),
            links: m.nlink(),
            change: if registry {
                (0, 0)
            } else {
                (m.ctime(), m.ctime_nsec())
            },
            modified: if registry {
                UNIX_EPOCH
            } else {
                m.modified().map_err(|e| e.to_string())?
            },
        });
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

fn cancelled(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("端末内台帳の移行準備を中断しました。元の記録は保持します".into())
    } else {
        Ok(())
    }
}
fn uuid(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id)
}
impl Registry {
    pub(in crate::storage::backup) fn handoff(
        &self,
        excluded: &Path,
        meeting_db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<Bundle, String> {
        cancelled(check)?;
        self.guard()?;
        let excluded_path = self.relative(excluded)?;
        let excluded_id = excluded_path
            .strip_prefix(".mon-stage-")
            .filter(|id| uuid(id))
            .ok_or("除外する移行準備領域が不正です")?;
        // Only this live native snapshot is omitted: not every active, recovered
        // or retained operation. Refuse other live construction owners.
        let active = stages()
            .lock()
            .map_err(|_| "一時領域所有ロックを取得できません")?;
        let mut found = false;
        for (path, a) in active.iter() {
            if a.registry
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|r| std::ptr::eq(r.as_ref(), self))
            {
                if path != excluded || a.area != excluded_id {
                    return Err("別の構築コピーが利用中です。移行準備を開始しません".into());
                }
                found = true;
            }
        }
        if !found {
            return Err("移行準備領域の生存する所有者がありません".into());
        }
        drop(active);
        let (mut areas, decisions) = {
            let db = self
                .db
                .lock()
                .map_err(|_| "一時領域台帳ロックを取得できません")?;
            self.check_schema(&db)?;
            let (roots, json, sha):(i64,String,String)=db.query_row("SELECT (SELECT count(*) FROM metadata),proof,hash FROM metadata WHERE id=1 AND length(CAST(proof AS BLOB))<=1024",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
            if roots != 1
                || hash(json.as_bytes()) != sha
                || serde_json::from_str::<Identity>(&json).map_err(|e| e.to_string())?
                    != self.root_identity
            {
                return Err("端末内台帳の元ルート証拠が変更されています".into());
            }
            let integrity: String = db
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            let foreign: i64 = db
                .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                    r.get(0)
                })
                .map_err(|e| e.to_string())?;
            let flags:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM entries WHERE removed NOT IN (0,1) OR kind NOT IN ('directory','file'))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
            if integrity != "ok" || foreign != 0 || flags {
                return Err("端末内台帳の構造・状態が破損しています".into());
            }
            // Bound allocations before reading opaque proofs. Kept invalid
            // ownership may be exported as history, never used for deletion.
            let sizes:(i64,i64,i64)=db.query_row("SELECT (SELECT coalesce(sum(length(CAST(proof AS BLOB))+length(CAST(path AS BLOB))+coalesce(length(CAST(error AS BLOB)),0)+256),0) FROM areas),(SELECT coalesce(sum(length(CAST(proof AS BLOB))+length(CAST(path AS BLOB))+256),0) FROM entries),(SELECT coalesce(sum(length(CAST(receipt_json AS BLOB))+256),0) FROM recovery_decisions)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
            if sizes.0 < 0
                || sizes.1 < 0
                || sizes.2 < 0
                || sizes
                    .0
                    .checked_add(sizes.1)
                    .and_then(|n| n.checked_add(sizes.2))
                    .is_none_or(|n| n > MAX_BYTES)
            {
                return Err("端末内台帳の移行内容が安全容量を超えています".into());
            }
            let bad:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM areas WHERE length(CAST(path AS BLOB))>1024 OR length(CAST(proof AS BLOB))>4096 OR length(CAST(error AS BLOB))>8192) OR EXISTS(SELECT 1 FROM entries WHERE length(CAST(path AS BLOB))>1024 OR length(CAST(proof AS BLOB))>4096) OR EXISTS(SELECT 1 FROM recovery_decisions WHERE length(CAST(receipt_json AS BLOB))>4096) OR EXISTS(SELECT 1 FROM entries e WHERE NOT EXISTS(SELECT 1 FROM areas a WHERE a.id=e.area))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
            if bad {
                return Err("端末内台帳の対象が欠損・上限超過しています".into());
            }
            let mut q=db.prepare("SELECT id,path,proof,hash,state,error,entry_count FROM areas ORDER BY id LIMIT 200001").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([], |r| {
                    Ok(Record {
                        id: r.get(0)?,
                        path: r.get(1)?,
                        proof: r.get(2)?,
                        hash: r.get(3)?,
                        state: r.get(4)?,
                        error: r.get(5)?,
                        entry_count: r.get(6)?,
                        entries: vec![],
                        intent_hash: String::new(),
                        record_valid: false,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if rows.len() > LIMIT {
                return Err("端末内台帳の移行件数が安全上限を超えています".into());
            }
            let mut areas = Vec::new();
            let mut total = 0usize;
            for mut area in rows {
                cancelled(check)?;
                if !uuid(&area.id)
                    || !matches!(area.state.as_str(), "active" | "attention" | "recovered")
                    || !(1..=LIMIT as i64).contains(&area.entry_count)
                {
                    return Err("端末内台帳の状態・IDが不正です".into());
                }
                if area.id == excluded_id {
                    let entries = self.load(&db, &area.id)?;
                    let p = entries
                        .get(&excluded_path)
                        .ok_or("移行準備領域の親証拠がありません")?;
                    if p.1 || !p.0.identity.matches(&private_directory(excluded)?) {
                        return Err("移行準備領域が差し替えられています".into());
                    }
                    continue;
                }
                let mut q=db.prepare("SELECT path,kind,proof,hash,removed FROM entries WHERE area=?1 ORDER BY path LIMIT 200001").map_err(|e|e.to_string())?;
                area.entries = q
                    .query_map(params![area.id], |r| {
                        Ok(Entry {
                            path: r.get(0)?,
                            kind: r.get(1)?,
                            proof: r.get(2)?,
                            hash: r.get(3)?,
                            removed: r.get(4)?,
                        })
                    })
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                total = total
                    .checked_add(area.entries.len())
                    .ok_or("台帳件数が範囲外です")?;
                if total > LIMIT || area.entries.len() as i64 != area.entry_count {
                    return Err("端末内台帳の対象件数が不正です".into());
                }
                area.record_valid = self.load(&db, &area.id).is_ok();
                areas.push(area);
            }
            let mut q=db.prepare("SELECT source,operation_id,intent_hash,receipt_json,receipt_hash FROM recovery_decisions ORDER BY source,operation_id LIMIT 200001").map_err(|e|e.to_string())?;
            let decisions = q
                .query_map([], |r| {
                    Ok(Keep {
                        source: r.get(0)?,
                        operation_id: r.get(1)?,
                        intent_hash: r.get(2)?,
                        receipt_json: r.get(3)?,
                        receipt_hash: r.get(4)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if decisions.len() > LIMIT {
                return Err("保持記録の移行件数が安全上限を超えています".into());
            }
            (areas, decisions)
        };
        let mut intents = BTreeMap::new();
        for area in &mut areas {
            cancelled(check)?;
            let e = self.scratch_evidence(&area.id)?;
            if e.live {
                return Err("利用中の領域を履歴として移しません".into());
            }
            area.intent_hash = e.intent_hash;
            intents.insert(
                ("scratch".to_owned(), area.id.clone()),
                area.intent_hash.clone(),
            );
        }
        let mut q=meeting_db.prepare("SELECT id FROM maintenance_operations WHERE kind='restore_addition' ORDER BY id LIMIT 200001").map_err(|e|e.to_string())?;
        let ids = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if ids.len() > LIMIT {
            return Err("復元履歴の移行件数が安全上限を超えています".into());
        }
        let mut restores = Vec::new();
        for id in ids {
            cancelled(check)?;
            let e = journal::recovery_evidence(meeting_db, &id)?;
            intents.insert(
                ("restore_addition".to_owned(), id.clone()),
                e.intent_hash.clone(),
            );
            restores.push(Restore {
                operation_id: id.clone(),
                intent_hash: e.intent_hash,
                status_hash: e.status_hash,
                state: e.state,
                published: e.live,
                record_valid: journal::source_record_valid(&self.root, meeting_db, &id),
            });
        }
        for d in &decisions {
            cancelled(check)?;
            if intents.get(&(d.source.clone(), d.operation_id.clone())) != Some(&d.intent_hash) {
                return Err("保持記録の対象が欠落・変更されています。移行しません".into());
            }
            review::validate_decision(
                &d.source,
                &d.operation_id,
                &d.intent_hash,
                &d.receipt_json,
                &d.receipt_hash,
            )?;
        }
        self.guard()?;
        let bundle = Bundle {
            version: 3,
            source_root: self.root.clone(),
            source_identity: self.root_identity.clone(),
            areas,
            decisions,
            restores,
            remaining_root_objects: remaining(&self.root, &excluded_path, check)?,
            origins: self.origins()?,
            bindings: self.restore_bindings()?,
        };
        if serde_json::to_vec(&bundle)
            .map_err(|e| e.to_string())?
            .len()
            > MAX_BYTES as usize
        {
            return Err("移行用台帳が安全容量を超えています".into());
        }
        Ok(bundle)
    }
}
