//! Live ownership is independent of crash-stale lifecycle/job rows. All admission
//! takes the connection lock before the ownership lock, including job claiming.
use super::Storage;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Blocker {
    pub kind: String,
    pub reason: String,
    pub job_id: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Availability {
    pub meeting_id: String,
    pub available: bool,
    pub blockers: Vec<Blocker>,
}
pub(crate) type Owners = Arc<Mutex<HashMap<String, HashMap<String, Blocker>>>>;
pub(crate) struct MeetingWork {
    owners: Owners,
    meeting: String,
    token: String,
}
impl Drop for MeetingWork {
    fn drop(&mut self) {
        let mut owners = self.owners.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(work) = owners.get_mut(&self.meeting) {
            work.remove(&self.token);
            if work.is_empty() {
                owners.remove(&self.meeting);
            }
        }
    }
}
impl Storage {
    pub(crate) fn require_global_data_writable(&self) -> Result<(), String> {
        self.require_data_writable()?;
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        if any_reserved(&db)? || super::deletion::fence::has_pending(&db)? {
            Err("未完了の会議削除があります。削除の完了・保持を確認してからデータ操作を行ってください".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn has_scoped_meeting_deletion(&self) -> Result<bool, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        any_reserved(&db)
    }
    pub(crate) fn deletion_operation_meeting(&self, operation: &str) -> Result<String, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        db.query_row("SELECT coalesce(meeting_id,json_extract(target_manifest_json,'$.scope.meetingId')) FROM maintenance_operations WHERE id=?1 AND kind='delete_meeting'", [operation], |r|r.get(0)).map_err(|e|e.to_string())
    }
    pub(crate) fn begin_meeting_work(
        &self,
        meeting: &str,
        kind: &str,
        job: Option<&str>,
    ) -> Result<MeetingWork, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.begin_meeting_work_locked(&db, meeting, kind, job)
    }
    pub(super) fn begin_meeting_work_locked(
        &self,
        db: &Connection,
        meeting: &str,
        kind: &str,
        job: Option<&str>,
    ) -> Result<MeetingWork, String> {
        self.require_data_writable()?;
        if self.maintenance.load(std::sync::atomic::Ordering::Acquire)
            || self.retired.load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("保存領域の切替・保守が進行中です".into());
        }
        if reserved(db, meeting)? {
            return Err("この会議は削除の確認・処理中です".into());
        }
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",
                [meeting],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("処理する会議が見つかりません".into());
        }
        let token = uuid::Uuid::new_v4().to_string();
        let mut owners = self
            .meeting_owners
            .lock()
            .map_err(|_| "会議の実行状態を確認できません")?;
        owners.entry(meeting.into()).or_default().insert(
            token.clone(),
            Blocker {
                kind: kind.into(),
                reason: match kind {
                    "recording" => "この会議の録音・音声保存が完了していません",
                    "export" => "この会議の書き出しが完了していません",
                    "playback" => "この会議の音声の解放を待っています",
                    "import" => "この会議の音声取り込みが完了していません",
                    _ => "この会議のジョブ・結果保存が完了していません",
                }
                .into(),
                job_id: job.map(str::to_owned),
            },
        );
        Ok(MeetingWork {
            owners: self.meeting_owners.clone(),
            meeting: meeting.into(),
            token,
        })
    }
    pub(crate) fn meeting_deletion_availability(
        &self,
        ids: &[String],
    ) -> Result<Vec<Availability>, String> {
        if ids.len() > 1000 {
            return Err("一度に確認する会議が多すぎます".into());
        }
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let owners = self
            .meeting_owners
            .lock()
            .map_err(|_| "会議の実行状態を確認できません")?;
        ids.iter()
            .map(|id| {
                if !uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == *id) {
                    return Err("会議IDが不正です".into());
                }
                // Playback is stopped and its native handles drained by the explicit
                // delete action. It must not make that action itself unavailable.
                let mut blockers: Vec<_> = owners
                    .get(id)
                    .into_iter()
                    .flat_map(|o| o.values().filter(|b| b.kind != "playback").cloned())
                    .collect();
                if let Err(reason) = self.require_data_writable() {
                    blockers.push(Blocker {
                        kind: "storage".into(),
                        reason,
                        job_id: None,
                    });
                }
                if reserved(&db, id)? {
                    blockers.push(Blocker {
                        kind: "deletion".into(),
                        reason: "この会議は削除の確認・処理中です".into(),
                        job_id: None,
                    });
                }
                let exists: bool = db
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",
                        [id],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if !exists {
                    blockers.push(Blocker {
                        kind: "missing".into(),
                        reason: "会議が見つかりません".into(),
                        job_id: None,
                    });
                }
                Ok(Availability {
                    meeting_id: id.clone(),
                    available: blockers.is_empty(),
                    blockers,
                })
            })
            .collect()
    }
    pub(crate) fn reserve_meeting_deletion(&self, meeting: &str) -> Result<(), String> {
        self.require_data_writable()?;
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.require_data_writable()?;
        let owners = self
            .meeting_owners
            .lock()
            .map_err(|_| "会議の実行状態を確認できません")?;
        if owners.get(meeting).is_some_and(|v| !v.is_empty()) {
            return Err(
                "この会議の処理が実行中です。録音画面・ジョブ一覧で停止完了後に削除してください"
                    .into(),
            );
        }
        if self.maintenance.load(std::sync::atomic::Ordering::Acquire)
            || self.retired.load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("保存領域の切替・保守が進行中です".into());
        }
        if !uuid::Uuid::parse_str(meeting).is_ok_and(|v| v.to_string() == meeting) {
            return Err("会議IDが不正です".into());
        }
        super::deletion::database::trusted_schema(&db)?;
        if let Err(error) = install(&db) {
            self.deletion_fence.fail_closed(&db);
            return Err(error);
        }
        self.deletion_fence.scoped_update(&db, || {
            db.execute("INSERT OR IGNORE INTO temp.mon_meeting_deletion_guard(meeting_id,bypass) VALUES(?1,0)", [meeting]).map_err(|e|e.to_string())?;
            Ok(())
        })
    }
    pub(crate) fn release_meeting_deletion(&self, meeting: &str) -> Result<(), String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.deletion_fence.scoped_update(&db, || {
            let pending: bool = db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM maintenance_operations WHERE kind='delete_meeting' AND (meeting_id=?1 OR json_extract(target_manifest_json,'$.scope.meetingId')=?1) AND {})", super::deletion::fence::UNFINISHED), [meeting], |r|r.get(0)).map_err(|e|e.to_string())?;
            if !pending { db.execute("DELETE FROM temp.mon_meeting_deletion_guard WHERE meeting_id=?1", [meeting]).map_err(|e|e.to_string())?; }
            Ok(())
        })
    }
}
pub(super) fn installed(db: &Connection) -> Result<bool, String> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_schema WHERE type='table' AND name='mon_meeting_deletion_guard')", [], |r|r.get(0)).map_err(|e|e.to_string())
}
pub(super) fn reserved(db: &Connection, meeting: &str) -> Result<bool, String> {
    if !installed(db)? {
        return Ok(false);
    }
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM temp.mon_meeting_deletion_guard WHERE meeting_id=?1)",
        [meeting],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}
pub(super) fn any_reserved(db: &Connection) -> Result<bool, String> {
    if !installed(db)? {
        return Ok(false);
    }
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM temp.mon_meeting_deletion_guard)",
        [],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}
pub(super) fn bypass(db: &Connection, enabled: bool) -> Result<(), String> {
    if installed(db)? {
        db.execute(
            "UPDATE temp.mon_meeting_deletion_guard SET bypass=?1",
            [enabled],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Use the deletion census's repository-owned predicates, including indirect
// children. Also guard incoming foreign keys so another meeting cannot acquire
// a reference after the census and before the file phase.
pub(super) fn install(db: &Connection) -> Result<(), String> {
    if installed(db)? {
        return Ok(());
    }
    db.execute_batch("CREATE TEMP TABLE mon_meeting_deletion_guard(meeting_id TEXT PRIMARY KEY NOT NULL,bypass INTEGER NOT NULL CHECK(bypass IN (0,1)));").map_err(|e|e.to_string())?;
    for (table, predicate) in super::deletion::database::TARGETS.iter().copied().chain([
        ("maintenance_operations", "meeting_id=?1"),
        (
            "maintenance_paths",
            "operation_id IN (SELECT id FROM maintenance_operations WHERE meeting_id=?1)",
        ),
    ]) {
        let columns: Vec<String> = db
            .prepare(&format!("PRAGMA main.table_info(\"{table}\")"))
            .map_err(|e| e.to_string())?
            .query_map([], |r| r.get(1))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        for (event, aliases) in [
            ("INSERT", vec!["NEW"]),
            ("UPDATE", vec!["OLD", "NEW"]),
            ("DELETE", vec!["OLD"]),
        ] {
            let tests: Vec<_> = aliases.iter().map(|alias| {
                // Only the first identifier belongs to the trigger row; identifiers
                // in subqueries are deliberately left in their own SQL scope.
                let split = predicate.find([' ', '=']).unwrap();
                let own = format!("{alias}.\"{}\"{}", &predicate[..split], &predicate[split..]);
                format!("EXISTS(SELECT 1 FROM temp.mon_meeting_deletion_guard g WHERE g.bypass=0 AND ({}))", own.replace("?1", "g.meeting_id"))
            }).collect();
            if columns.is_empty() {
                return Err(format!("会議の保護対象表がありません: {table}"));
            }
            db.execute_batch(&format!("CREATE TEMP TRIGGER mon_meeting_guard_{table}_{event} BEFORE {event} ON main.\"{table}\" WHEN {} BEGIN SELECT RAISE(ABORT,'meeting_deletion_reserved'); END;",tests.join(" OR "))).map_err(|e|e.to_string())?;
        }
    }
    // All foreign keys pointing into the deletion census get equivalent guards.
    let tables: Vec<String> = db.prepare("SELECT name FROM main.sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").map_err(|e|e.to_string())?.query_map([], |r|r.get(0)).map_err(|e|e.to_string())?.collect::<Result<_,_>>().map_err(|e|e.to_string())?;
    for child in tables {
        let mut fks: std::collections::BTreeMap<i64, (String, Vec<(String, String)>)> =
            Default::default();
        let mut q = db
            .prepare(&format!("PRAGMA main.foreign_key_list(\"{child}\")"))
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, parent, from, to) = row.map_err(|e| e.to_string())?;
            fks.entry(id)
                .or_insert_with(|| (parent, vec![]))
                .1
                .push((from, to));
        }
        for (id, (parent, cols)) in fks {
            let Some((_, scope)) = super::deletion::database::TARGETS
                .iter()
                .find(|(name, _)| *name == parent)
            else {
                continue;
            };
            for event in ["INSERT", "UPDATE"] {
                let join = cols
                    .iter()
                    .map(|(from, to)| format!("NEW.\"{from}\"=p.\"{to}\""))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                db.execute_batch(&format!("CREATE TEMP TRIGGER mon_meeting_ref_{child}_{id}_{event} BEFORE {event} ON main.\"{child}\" WHEN EXISTS(SELECT 1 FROM temp.mon_meeting_deletion_guard g WHERE g.bypass=0 AND EXISTS(SELECT 1 FROM main.\"{parent}\" p WHERE {join} AND p.rowid IN (SELECT rowid FROM main.\"{parent}\" WHERE {}))) BEGIN SELECT RAISE(ABORT,'meeting_deletion_reserved'); END;",scope.replace("?1","g.meeting_id"))).map_err(|e|e.to_string())?;
            }
        }
    }
    Ok(())
}
