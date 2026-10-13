//! Fixed file completion -> atomic row removal -> resumable DB/WAL purge.
//! Native shared admission and explicit user-facing consent are still required.
use super::{
    database,
    intent::{self, Prepared},
    removal::{self, Finished},
};
use crate::storage::{now_ms, Storage};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};

fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("削除を停止しました。既に消したデータは復元しません".into())
    } else {
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Phase {
    RowsPending,
    RowsDeleted,
    Finished,
}
impl Phase {
    pub(super) fn state(self) -> &'static str {
        if self == Self::Finished {
            "completed"
        } else {
            "cleanup"
        }
    }
    pub(super) fn rows_committed(self) -> bool {
        self != Self::RowsPending
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Progress {
    version: u32,
    files: Finished,
    database_phase: Phase,
}
impl Progress {
    fn encode(&self) -> Result<String, String> {
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if json.len() > 4096 {
            return Err("複合削除進捗が安全上限を超えています".into());
        }
        Ok(json)
    }
}
pub(super) struct Receipt {
    confirmed: Prepared,
    progress: Progress,
    manifest: super::files::Manifest,
}
impl Receipt {
    pub(super) fn database_phase(&self) -> Phase {
        self.progress.database_phase
    }
    pub(super) fn file_progress(&self) -> &removal::Progress {
        self.progress.files.progress()
    }
    pub(super) fn progress_json(&self) -> Result<String, String> {
        self.progress.encode()
    }
    pub(super) fn review_census(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<super::files::Census, String> {
        self.verify_saved(db, check)?;
        let other = super::fence::other_pending(db, Some(self.confirmed.operation()))?;
        if other {
            return Err("別の未完了保守操作があります。対象を保持します".into());
        }
        let roots =
            super::renewal::reconfirmation_roots(db, &self.confirmed, self.file_progress(), check)?;
        let census = if self.row_phase_committed() {
            super::files::Census::capture_deleted_locked(
                storage,
                db,
                &self.confirmed,
                check,
                None,
                &roots,
            )?
        } else {
            super::files::Census::capture_removal_locked(
                storage,
                db,
                &self.confirmed.scope().meeting_id,
                self.confirmed.operation(),
                check,
                None,
                &roots,
            )?
        };
        if census.scope() != self.confirmed.scope() {
            return Err("元のDB範囲・起動時実体を再確認で変更できません".into());
        }
        super::renewal::verify_review_identity(
            storage,
            db,
            &self.confirmed,
            &self.manifest,
            self.completed(),
        )?;
        self.verify_saved(db, check)?;
        census.verify_tree_roots(storage)?;
        Ok(census)
    }
    pub(super) fn confirmed(&self) -> &Prepared {
        &self.confirmed
    }
    pub(super) fn manifest(&self) -> &super::files::Manifest {
        &self.manifest
    }
    pub(super) fn verify_resume(
        &self,
        storage: &Storage,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.verify_live(storage, &db, check)
    }
    pub(super) fn row_phase_committed(&self) -> bool {
        self.progress.database_phase != Phase::RowsPending
    }
    pub(super) fn load(
        db: &Connection,
        operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let confirmed = intent::read_database(db, operation, check)?;
        let (json, state, meeting): (String, String, Option<String>) = db
            .query_row(
                "SELECT progress_json,state,meeting_id FROM maintenance_operations WHERE id=?1",
                params![operation],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| e.to_string())?;
        let progress: Progress = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        let manifest = progress.files.manifest(db, &confirmed, check)?;
        let state_matches = match progress.database_phase {
            Phase::RowsPending => {
                state == "cleanup"
                    && meeting.as_deref() == Some(confirmed.scope().meeting_id.as_str())
            }
            Phase::RowsDeleted => state == "cleanup" && meeting.is_none(),
            Phase::Finished => state == "completed" && meeting.is_none(),
        };
        if progress.version != 1
            || !state_matches
            || progress.encode()? != json
            || progress
                .files
                .progress()
                .database_phase
                .is_some_and(|phase| phase > progress.database_phase)
        {
            return Err("複合削除の進捗と会議参照を検証できません".into());
        }
        Ok(Self {
            confirmed,
            progress,
            manifest,
        })
    }
    pub(super) fn verify_saved(
        &self,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let current = Self::load(db, self.confirmed.operation(), check)?;
        self.confirmed.matches_saved(&current.confirmed)?;
        if self.progress != current.progress || self.manifest != current.manifest {
            return Err("複合削除の進捗が変更されています".into());
        }
        Ok(())
    }
    pub(super) fn completed(&self) -> bool {
        self.progress.database_phase == Phase::Finished
    }
    fn verify_live(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        // Historical correspondence permits a separate review, never an old
        // acknowledgement, checkpoint, or delete at a different root.
        if storage.managed_anchor.identity()?
            != (self.manifest.root.clone(), self.manifest.database.clone())
        {
            return Err("移動後の保存先では現在の一覧と新しい同意が必要です".into());
        }
        let census = self.review_census(storage, db, check)?;
        if census
            .manifest()
            .entries
            .iter()
            .any(|entry| entry.proof.is_some())
        {
            return Err("削除済み範囲にファイル・フォルダーが再出現しています。削除せず現在の対象を保持します".into());
        }
        Ok(())
    }
}
pub(super) fn read(
    storage: &Storage,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<Receipt, String> {
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    storage.managed_anchor.verify()?;
    Receipt::load(&db, operation, check)
}
pub(super) fn start(
    storage: &Storage,
    files: &removal::Receipt,
    check: &impl Fn() -> bool,
) -> Result<Receipt, String> {
    cancel(check)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    storage.managed_anchor.verify()?;
    let state: String = db
        .query_row(
            "SELECT state FROM maintenance_operations WHERE id=?1 AND kind='delete_meeting'",
            params![files.confirmed().operation()],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let file_form: Option<bool> = db.query_row("SELECT json_extract(progress_json,'$.version')=3 FROM maintenance_operations WHERE id=?1", params![files.confirmed().operation()], |r|r.get(0)).map_err(|e|e.to_string())?;
    if matches!(state.as_str(), "cleanup" | "completed") && file_form != Some(true) {
        let current = Receipt::load(&db, files.confirmed().operation(), check)?;
        files.matches_completion(&current.confirmed, &current.progress.files)?;
        current.verify_live(storage, &db, check)?;
        return Ok(current);
    }
    let _writer = super::fence::write_removal(storage, &db, files, check)?;
    let finished = files.verify_finished(storage, &db, check)?;
    let progress = Progress {
        version: 1,
        files: finished,
        database_phase: files.progress.database_phase.unwrap_or(Phase::RowsPending),
    };
    let p = files.confirmed();
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    files.verify_saved(&tx, check)?;
    storage.managed_anchor.verify()?;
    if tx.execute("UPDATE maintenance_operations SET state=?2,progress_json=?3,error_code=NULL,updated_at=max(updated_at,?4) WHERE id=?1 AND kind='delete_meeting' AND state=?5 AND progress_json=?6", params![p.operation(), progress.database_phase.state(), progress.encode()?, now_ms()?, files.progress.state(), files.progress_json()?]).map_err(|e| e.to_string())? != 1 {
        return Err("DB削除開始の対象記録が変更されています".into());
    }
    cancel(check)?;
    tx.commit().map_err(|e| e.to_string())?;
    Receipt::load(&db, p.operation(), check)
}
fn save_phase(db: &Connection, receipt: &Receipt, phase: Phase) -> Result<Progress, String> {
    let mut next = receipt.progress.clone();
    let state = match (&next.database_phase, &phase) {
        (Phase::RowsPending, Phase::RowsDeleted) => "cleanup",
        (Phase::RowsDeleted, Phase::Finished) => "completed",
        _ => return Err("複合削除の進捗遷移が不正です".into()),
    };
    next.database_phase = phase;
    if db.execute("UPDATE maintenance_operations SET state=?2,progress_json=?3,error_code=NULL,updated_at=max(updated_at,?4) WHERE id=?1 AND kind='delete_meeting' AND state='cleanup' AND progress_json=?5", params![receipt.confirmed.operation(),state,next.encode()?,now_ms()?,receipt.progress.encode()?]).map_err(|e| e.to_string())? != 1 {
        return Err("複合削除の保存対象が変更されています".into());
    }
    Ok(next)
}
pub(super) fn run(
    storage: &Storage,
    receipt: &mut Receipt,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    run_inner(storage, receipt, check, &|_| {})
}
fn run_inner(
    storage: &Storage,
    receipt: &mut Receipt,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<(), String> {
    cancel(check)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    run_locked(storage, &db, receipt, check, event)
}
fn run_locked(
    storage: &Storage,
    db: &Connection,
    receipt: &mut Receipt,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<(), String> {
    let _writer = super::fence::write_completion(storage, db, receipt, check)?;
    receipt.verify_live(storage, &db, check)?;
    if receipt.progress.database_phase == Phase::RowsPending {
        let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        receipt.verify_live(storage, &tx, check)?;
        storage.managed_anchor.verify_current_history()?;
        let supersession = super::audio_supersession::retain(
            &tx,
            &storage.managed_anchor,
            &receipt.confirmed,
            check,
        )?;
        database::erase_rows(&tx, receipt.confirmed.scope(), check)?;
        event("rowsErased");
        let next = save_phase(&tx, receipt, Phase::RowsDeleted)?;
        if let Some(operation) = supersession {
            super::audio_supersession::inspect(&tx, &storage.managed_anchor, &operation, check)?;
        }
        event("beforeRowsCommit");
        cancel(check)?;
        receipt
            .progress
            .files
            .verify_absent(storage, db, &receipt.confirmed)?;
        tx.commit().map_err(|e| e.to_string())?;
        receipt.progress = next;
        event("rowsCommitted");
    }
    receipt.verify_live(storage, &db, check)?;
    if receipt.progress.database_phase == Phase::RowsDeleted {
        let mode: String = db
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if mode != "wal" {
            return Err("DBのWAL設定が想定と異なります".into());
        }
        cancel(check)?;
        storage.managed_anchor.verify()?;
        database::secure_delete(&db)?;
        database::checkpoint(&db)?;
        event("checkpointed");
        cancel(check)?;
        storage.managed_anchor.verify()?;
        db.execute_batch("VACUUM").map_err(|e| e.to_string())?;
        event("vacuumed");
        cancel(check)?;
        storage.managed_anchor.verify()?;
        database::checkpoint(&db)?;
        event("purged");
        receipt.verify_live(storage, &db, check)?;
        let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        receipt.verify_saved(&tx, check)?;
        let next = save_phase(&tx, receipt, Phase::Finished)?;
        event("beforeCompletionCommit");
        cancel(check)?;
        receipt
            .progress
            .files
            .verify_absent(storage, db, &receipt.confirmed)?;
        tx.commit().map_err(|e| e.to_string())?;
        receipt.progress = next;
        event("completed");
    }
    // This WAL contains only the non-text completion receipt. Do not return
    // success until its final checkpoint succeeds; acknowledgement can replay.
    cancel(check)?;
    receipt.verify_live(storage, &db, check)?;
    storage.managed_anchor.verify()?;
    database::compact_receipt_pages(&db)?;
    cancel(check)?;
    database::checkpoint(&db)?;
    event("ackCheckpointed");
    super::startup::resolve(storage, db, receipt, check)?;
    Ok(())
}

#[cfg(test)]
pub(super) fn run_observed(
    storage: &Storage,
    receipt: &mut Receipt,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<(), String> {
    run_inner(storage, receipt, check, event)
}

#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;
