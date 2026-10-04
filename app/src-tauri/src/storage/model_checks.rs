//! Append-only device-local trials. Native ready means this fixed CPU smoke
//! protocol passed, not an applied profile or availability on another host.
use super::*;
use crate::model_compatibility::{canonical_id, CheckState, CheckSummary, ProbeReport};
use crate::model_placement::PlacementRecord;

pub(super) struct SavedCheck {
    pub summary: CheckSummary,
    pub report: Option<ProbeReport>,
}
pub(super) fn read(db: &Connection, id: &str) -> Result<Option<SavedCheck>, String> {
    let row: Option<(String,String,String,i64,String,Option<String>,Option<String>,Option<String>,i64,Option<i64>)> = db.query_row("SELECT id,package_id,review_hash,adapter_revision,state,report_json,report_hash,error_message,created_at,finished_at FROM model_compatibility_checks WHERE id=?1", params![id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, package, review, revision, state, json, hash, error, created, finished)) = row
    else {
        return Ok(None);
    };
    let state: CheckState =
        serde_json::from_value(serde_json::Value::String(state)).map_err(|e| e.to_string())?;
    if !canonical_id(&id)
        || !canonical_id(&package)
        || revision != 1
        || created < 0
        || review.len() != 64
        || !review
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || finished.is_some_and(|n| n < created)
        || error
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 4096 || s.contains('\0'))
    {
        return Err("モデル互換性の操作記録が不正です".into());
    }
    let report = match state {
        CheckState::Preparing => {
            if finished.is_some() || json.is_some() || hash.is_some() || error.is_some() {
                return Err("未完了の互換性記録に確定結果があります".into());
            }
            None
        }
        CheckState::Succeeded => {
            let json = json.ok_or("互換性の成功証拠がありません")?;
            if json.len() > 65536
                || finished.is_none()
                || error.is_some()
                || hash.as_deref()
                    != Some(format!("{:x}", Sha256::digest(json.as_bytes())).as_str())
            {
                return Err("互換性の成功証拠・SHAが一致しません".into());
            }
            let report: ProbeReport = serde_json::from_str(&json).map_err(|e| e.to_string())?;
            if report.operation_id != id
                || report.package_id != package
                || report.review_hash != review
                || report.adapter_revision != 1
            {
                return Err("互換性の応答と操作の対象が一致しません".into());
            }
            Some(report)
        }
        _ => {
            if finished.is_none() || json.is_some() || hash.is_some() || error.is_none() {
                return Err("互換性確認の失敗・中断記録が不正です".into());
            }
            None
        }
    };
    Ok(Some(SavedCheck {
        summary: CheckSummary {
            operation_id: id,
            package_id: package,
            review_hash: review,
            adapter_revision: 1,
            state,
            error_message: error,
        },
        report,
    }))
}
pub(super) fn latest(db: &Connection, package: &str) -> Result<Option<SavedCheck>, String> {
    let id: Option<String> = db.query_row("SELECT id FROM model_compatibility_checks WHERE package_id=?1 ORDER BY rowid DESC LIMIT 1",params![package],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    id.map(|id| read(db, &id)?.ok_or("互換性の操作が欠損しています".into()))
        .transpose()
}
pub(super) fn validate_package_state(
    db: &Connection,
    record: &PlacementRecord,
    placed: bool,
    state: &str,
) -> Result<(), String> {
    let check = latest(db, &record.package_id)?;
    if let Some(check) = &check {
        if !placed || check.summary.review_hash != record.review_hash {
            return Err("配置と互換性確認の対象が一致しません".into());
        }
        if let Some(report) = &check.report {
            report.validate(record)?;
        }
    }
    let valid = match (state, check.as_ref().map(|c| c.summary.state)) {
        ("unavailable", None) => true,
        ("preparing", None) => !placed,
        ("preparing", Some(CheckState::Preparing)) => placed,
        ("ready", Some(CheckState::Succeeded)) => placed,
        ("incompatible", Some(CheckState::Failed)) => placed,
        ("unavailable", Some(CheckState::Cancelled | CheckState::Interrupted)) => placed,
        _ => false,
    };
    if !valid {
        return Err("モデル準備状態と最後の互換性確認が一致しません".into());
    }
    Ok(())
}
impl Storage {
    pub(crate) fn model_package_for_check(
        &self,
        package: &str,
        review: &str,
    ) -> Result<PlacementRecord, String> {
        if !canonical_id(package) {
            return Err("モデルパッケージIDが不正です".into());
        }
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let (record, placed) =
            super::model_placements::load(&db, package)?.ok_or("モデル配置記録がありません")?;
        if !placed || record.review_hash != review {
            return Err("確認対象の配置・SHAが一致しません".into());
        }
        Ok(record)
    }
    pub(crate) fn model_check_record(
        &self,
        operation: &str,
    ) -> Result<Option<(CheckSummary, Option<ProbeReport>)>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let saved = read(&db, operation)?;
        if let Some(saved) = &saved {
            if latest(&db, &saved.summary.package_id)?
                .is_none_or(|latest| latest.summary.operation_id != operation)
            {
                return Err("古い互換性の操作です。最後の記録を確認してください".into());
            }
        }
        Ok(saved.map(|saved| (saved.summary, saved.report)))
    }
    pub(crate) fn begin_model_check(
        &self,
        operation: &str,
        expected: &PlacementRecord,
    ) -> Result<(), String> {
        if !canonical_id(operation) {
            return Err("互換性の操作IDが不正です".into());
        }
        expected.validate()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let (record, placed) = super::model_placements::load(&tx, &expected.package_id)?
            .ok_or("モデル配置記録がありません")?;
        if !placed
            || record != *expected
            || !super::model_placements::unused_identity(&tx, operation)?
        {
            return Err("互換性の対象・操作IDが一致しません".into());
        }
        tx.execute("INSERT INTO model_compatibility_checks(id,package_id,review_hash,adapter_revision,state,created_at) VALUES(?1,?2,?3,1,'preparing',?4)",params![operation,expected.package_id,expected.review_hash,now_ms().map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
        tx.execute(
            "UPDATE model_packages SET state='preparing' WHERE id=?1",
            params![expected.package_id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub(crate) fn finish_model_check(
        &self,
        operation: &str,
        expected: &PlacementRecord,
        report: Option<&ProbeReport>,
        state: CheckState,
        error: Option<&str>,
    ) -> Result<CheckSummary, String> {
        let (json, hash) = match report {
            Some(report) => {
                if state != CheckState::Succeeded || report.operation_id != operation {
                    return Err("互換性の成功対象が一致しません".into());
                }
                let (j, h) = report.encoded(expected)?;
                (Some(j), Some(h))
            }
            None => (None, None),
        };
        if !matches!(
            state,
            CheckState::Succeeded | CheckState::Failed | CheckState::Cancelled
        ) || if state == CheckState::Succeeded {
            error.is_some() || json.is_none()
        } else {
            error.is_none() || json.is_some()
        } {
            return Err("互換性の終了状態が不正です".into());
        }
        let error = error.map(|value| {
            value
                .chars()
                .filter(|c| *c != '\0')
                .scan(0, |bytes, c| {
                    *bytes += c.len_utf8();
                    (*bytes <= 4096).then_some(c)
                })
                .collect::<String>()
        });
        if error.as_ref().is_some_and(String::is_empty) {
            return Err("互換性の終了理由が空です".into());
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let (record, placed) = super::model_placements::load(&tx, &expected.package_id)?
            .ok_or("モデル配置が欠損しています")?;
        let saved = read(&tx, operation)?.ok_or("互換性の意図記録がありません")?;
        if !placed
            || record != *expected
            || saved.summary.state != CheckState::Preparing
            || saved.summary.package_id != expected.package_id
            || saved.summary.review_hash != expected.review_hash
        {
            return Err("互換性の意図・確認対象が変わっています".into());
        }
        let changed=tx.execute("UPDATE model_compatibility_checks SET state=?1,report_json=?2,report_hash=?3,error_message=?4,finished_at=max(created_at,?5) WHERE id=?6 AND state='preparing'",params![state.text(),json,hash,error,now_ms().map_err(|e|e.to_string())?,operation]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("互換性の終了記録を確定できません".into());
        }
        let package_state = match state {
            CheckState::Succeeded => "ready",
            CheckState::Failed => "incompatible",
            _ => "unavailable",
        };
        tx.execute(
            "UPDATE model_packages SET state=?1 WHERE id=?2",
            params![package_state, expected.package_id],
        )
        .map_err(|e| e.to_string())?;
        let summary = read(&tx, operation)?
            .ok_or("互換性の終了記録がありません")?
            .summary;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(summary)
    }
}
