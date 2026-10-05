//! Append-only evidence for an explicitly reviewed remaining file scope.
//! These internal observations are never reconstructed from client JSON.
use super::{
    completion,
    files::{Census, Manifest},
    intent::{self, Observations, Prepared},
    removal::{self, Progress},
};
use crate::{
    analysis_worker::data_maintenance::DataMaintenanceLease,
    storage::{now_ms, Storage},
};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_GENERATIONS: u32 = 32;
const AUDIT_PROGRESS: &str = "{\"confirmationRecorded\":true}";
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("残りの削除範囲の再確認を停止しました。元の対象と進捗を保持します".into())
    } else {
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Reference {
    pub(super) operation_id: String,
    pub(super) intent_hash: String,
    pub(super) generation: u32,
}
impl Reference {
    pub(super) fn valid(&self) -> bool {
        self.generation > 0
            && self.generation <= MAX_GENERATIONS
            && uuid::Uuid::parse_str(&self.operation_id)
                .is_ok_and(|id| id.to_string() == self.operation_id)
            && self.intent_hash.len() == 64
            && self
                .intent_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Record {
    version: u32,
    source_operation: String,
    source_intent_hash: String,
    previous_progress: Progress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_database_phase: Option<completion::Phase>,
    observations: intent::Intent,
}
impl Record {
    fn encode(&self) -> Result<String, String> {
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if json.len() > 65536 {
            Err("再確認記録が安全上限を超えています".into())
        } else {
            Ok(json)
        }
    }
}

// Loading one generation at a time bounds retained inventories. The immutable
// audit links prove every earlier cursor/policy; no cursor is silently reset.
pub(super) fn active_manifest(
    db: &Connection,
    original: &Prepared,
    progress: &Progress,
    check: &impl Fn() -> bool,
) -> Result<Manifest, String> {
    let mut chain = Vec::new();
    let mut current = progress.confirmation.clone();
    let source_hash = original.intent_hash()?;
    while let Some(reference) = current {
        cancel(check)?;
        if !reference.valid()
            || reference.operation_id == original.operation()
            || chain.len() >= MAX_GENERATIONS as usize
        {
            return Err("削除再確認の世代・識別子が不正です".into());
        }
        let (meeting, kind, state, json, sha, consent, saved, created, updated): (Option<String>,String,String,String,String,Option<i64>,String,i64,i64) = db.query_row(
            "SELECT meeting_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at FROM maintenance_operations WHERE id=?1",
            params![reference.operation_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).map_err(|e| e.to_string())?;
        if meeting.is_some()
            || kind != "delete_meeting"
            || state != "completed"
            || json.len() > 65536
            || sha != reference.intent_hash
            || hash(json.as_bytes()) != sha
            || saved != AUDIT_PROGRESS
            || created < original.consent_at()
            || updated != created
            || consent != Some(created)
        {
            return Err("残りの削除範囲の固定確認記録を検証できません".into());
        }
        let record: Record = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if record.version != 1
            || record.source_operation != original.operation()
            || record.source_intent_hash != source_hash
            || record.encode()? != json
            || reference.generation
                != record
                    .previous_progress
                    .confirmation
                    .as_ref()
                    .map_or(1, |p| p.generation.saturating_add(1))
            || created < record.previous_progress.consent_at()
        {
            return Err("再確認と元の削除記録・同意・進捗の対応が不正です".into());
        }
        current = record.previous_progress.confirmation.clone();
        chain.push((reference, record, created));
    }
    let mut manifest = original.manifest().clone();
    let mut previous: Option<Reference> = None;
    let mut latest_consent = None;
    let mut database_phase = None;
    for (reference, record, consent) in chain.into_iter().rev() {
        if record.previous_progress.confirmation != previous
            || latest_consent.is_some_and(|c| c != record.previous_progress.consent_at())
        {
            return Err("再確認履歴の接続が一致しません".into());
        }
        record.previous_progress.validate(original, &manifest)?;
        match (
            record.previous_database_phase,
            record.previous_progress.database_phase,
        ) {
            (None, None) if database_phase.is_none() => {}
            (Some(_), None) if database_phase.is_none() && record.previous_progress.removed() => {}
            (Some(next), Some(old))
                if database_phase == Some(old)
                    && (next == old || (next > old && record.previous_progress.removed())) => {}
            _ => return Err("DB段階を忘れたり巻き戻す再確認履歴は採用できません".into()),
        }
        database_phase = record.previous_database_phase;
        let observations =
            Observations::load(db, &reference.operation_id, record.observations, check)?;
        if observations.scope() != original.scope()
            || observations.manifest.root != original.manifest().root
            || observations.manifest.database != original.manifest().database
            || observations.manifest.meeting_id != original.manifest().meeting_id
        {
            return Err("元のDB範囲・起動時実体を再確認で変更できません".into());
        }
        manifest = observations.manifest;
        previous = Some(reference);
        latest_consent = Some(consent);
    }
    if previous != progress.confirmation
        || database_phase != progress.database_phase
        || latest_consent.is_some_and(|c| c != progress.consent_at())
    {
        return Err("現在の削除計画が固定した再同意と一致しません".into());
    }
    progress.validate(original, &manifest)?;
    Ok(manifest)
}

pub(super) struct Review {
    operation: String,
    source_hash: String,
    previous_progress: String,
    census: Census,
}
enum Source<'a> {
    Files(&'a removal::Receipt),
    Database(&'a completion::Receipt),
}
impl Source<'_> {
    fn confirmed(&self) -> &Prepared {
        match self {
            Self::Files(r) => r.confirmed(),
            Self::Database(r) => r.confirmed(),
        }
    }
    fn files(&self) -> &Progress {
        match self {
            Self::Files(r) => &r.progress,
            Self::Database(r) => r.file_progress(),
        }
    }
    fn database_phase(&self) -> Option<completion::Phase> {
        match self {
            Self::Files(r) => r.progress.database_phase,
            Self::Database(r) => Some(r.database_phase()),
        }
    }
    fn progress_json(&self) -> Result<String, String> {
        match self {
            Self::Files(r) => r.progress_json(),
            Self::Database(r) => r.progress_json(),
        }
    }
    fn verify_saved(&self, db: &Connection, check: &impl Fn() -> bool) -> Result<(), String> {
        match self {
            Self::Files(r) => r.verify_saved(db, check),
            Self::Database(r) => r.verify_saved(db, check),
        }
    }
    fn census(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<Census, String> {
        match self {
            Self::Files(r) => r.review_census(storage, db, check),
            Self::Database(r) => r.review_census(storage, db, check),
        }
    }
    fn writer<'a>(
        &self,
        storage: &'a Storage,
        db: &'a Connection,
        check: &impl Fn() -> bool,
    ) -> Result<super::fence::Writer<'a>, String> {
        match self {
            Self::Files(r) => super::fence::write_removal(storage, db, r, check),
            Self::Database(r) => super::fence::write_completion(storage, db, r, check),
        }
    }
    fn state(&self) -> &'static str {
        match self {
            Self::Files(r) => r.progress.state(),
            Self::Database(r) => r.database_phase().state(),
        }
    }
}
impl Source<'_> {
    fn verify_archived_root(&self, storage: &Storage) -> Result<(), String> {
        match self {
            Self::Files(r) => r.verify_review_root(storage),
            Self::Database(r) => r.verify_review_root(storage),
        }
    }
}
impl Review {
    pub(super) fn capture(
        storage: &Storage,
        receipt: &removal::Receipt,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::capture_source(storage, Source::Files(receipt), check)
    }
    pub(super) fn capture_database(
        storage: &Storage,
        receipt: &completion::Receipt,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::capture_source(storage, Source::Database(receipt), check)
    }
    fn capture_source(
        storage: &Storage,
        source: Source<'_>,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        cancel(check)?;
        if source
            .files()
            .confirmation
            .as_ref()
            .is_some_and(|r| r.generation >= MAX_GENERATIONS)
        {
            return Err("残りの削除範囲の再確認が世代上限に達しました。対象を保持します".into());
        }
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        source.verify_saved(&db, check)?;
        let census = source.census(storage, &db, check)?;
        Ok(Self {
            operation: source.confirmed().operation().into(),
            source_hash: source.confirmed().intent_hash()?,
            previous_progress: source.progress_json()?,
            census,
        })
    }
    pub(super) fn manifest(&self) -> &Manifest {
        self.census.manifest()
    }
    pub(super) fn verify_live(
        &self,
        storage: &Storage,
        receipt: &removal::Receipt,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.verify(storage, &db, &Source::Files(receipt), check)
    }
    pub(super) fn verify_database_live(
        &self,
        storage: &Storage,
        receipt: &completion::Receipt,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.verify(storage, &db, &Source::Database(receipt), check)
    }
    fn verify(
        &self,
        storage: &Storage,
        db: &Connection,
        source: &Source<'_>,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        source.verify_saved(db, check)?;
        if self.operation != source.confirmed().operation()
            || self.source_hash != source.confirmed().intent_hash()?
            || self.previous_progress != source.progress_json()?
        {
            return Err("再確認中に元の進捗が変わりました。新しい一覧を確認してください".into());
        }
        let current = source.census(storage, db, check)?;
        if current.scope() != self.census.scope() || current.manifest() != self.census.manifest() {
            return Err("再同意後に残りの対象が変わりました。新しい一覧を確認してください".into());
        }
        Ok(())
    }
}

pub(super) fn accept(
    storage: &Storage,
    receipt: &mut removal::Receipt,
    review: &Review,
    include_unregistered: bool,
    admission: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    let next = accept_inner(
        storage,
        Source::Files(receipt),
        review,
        include_unregistered,
        admission,
        check,
        &|_| {},
    )?;
    *receipt = next;
    Ok(())
}
pub(super) fn accept_database(
    storage: &Storage,
    receipt: &completion::Receipt,
    review: &Review,
    include_unregistered: bool,
    admission: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<removal::Receipt, String> {
    accept_inner(
        storage,
        Source::Database(receipt),
        review,
        include_unregistered,
        admission,
        check,
        &|_| {},
    )
}
fn accept_inner(
    storage: &Storage,
    source: Source<'_>,
    review: &Review,
    include_unregistered: bool,
    admission: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<removal::Receipt, String> {
    admission.validate_active()?;
    let check = || check() || admission.cancelled();
    cancel(&check)?;
    // Reuse policy validation; unknown files are never approved by renewal alone.
    removal::Reviewed::confirm(&review.census, include_unregistered)?;
    let original = source.confirmed();
    let observations = Observations::capture(&review.census)?;
    let record = Record {
        version: 1,
        source_operation: original.operation().into(),
        source_intent_hash: original.intent_hash()?,
        previous_progress: source.files().clone(),
        previous_database_phase: source.database_phase(),
        observations: observations.header.clone(),
    };
    let json = record.encode()?;
    let reference = Reference {
        operation_id: uuid::Uuid::new_v4().to_string(),
        intent_hash: hash(json.as_bytes()),
        generation: source
            .files()
            .confirmation
            .as_ref()
            .map_or(1, |r| r.generation + 1),
    };
    let consent = now_ms()?.max(source.files().consent_at());
    let next = Progress::renewed(
        original,
        review.manifest(),
        reference.clone(),
        include_unregistered,
        consent,
        source.database_phase(),
    )?;
    removal::verify_reservation_names(original.operation(), review.manifest(), &next)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    review.verify(storage, &db, &source, &check)?;
    let _writer = source.writer(storage, &db, &check)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    review.verify(storage, &tx, &source, &check)?;
    tx.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES(?1,'delete_meeting','prepared',?2,?3,?4,?5,?4,?4)",params![reference.operation_id,json,reference.intent_hash,consent,AUDIT_PROGRESS]).map_err(|e| e.to_string())?;
    event("reviewIntentStored");
    observations.store_paths(&tx, &reference.operation_id, &check, event)?;
    tx.execute(
        "UPDATE maintenance_operations SET state='completed' WHERE id=?1 AND state='prepared'",
        params![reference.operation_id],
    )
    .map_err(|e| e.to_string())?;
    event("reviewRecorded");
    // The audit has no meeting FK; it does not change the original DB scope.
    review.verify(storage, &tx, &source, &check)?;
    if tx.execute("UPDATE maintenance_operations SET progress_json=?2,error_code=NULL,updated_at=max(updated_at,?3) WHERE id=?1 AND kind='delete_meeting' AND state=?5 AND progress_json=?4",params![original.operation(),next.encode()?,consent,source.progress_json()?,source.state()]).map_err(|e| e.to_string())? != 1 {
        return Err("再同意の切替対象が変更されています".into());
    }
    if active_manifest(&tx, original, &next, &check)? != *review.manifest() {
        return Err("再同意の保存結果が一致しません".into());
    }
    storage.managed_anchor.verify()?;
    event("beforeReviewCommit");
    cancel(&check)?;
    source.verify_archived_root(storage)?;
    // Recheck the actual inventory once more after publication, without using
    // the stale cursor that was just archived in the immutable audit record.
    let saved = removal::Receipt::load(&tx, original.operation(), &check)?;
    let live = saved.review_census(storage, &tx, &check)?;
    if live.manifest() != review.manifest() || live.scope() != original.scope() {
        return Err("再確認の確定前に対象が変わりました".into());
    }
    source.verify_archived_root(storage)?;
    cancel(&check)?;
    tx.commit().map_err(|e| e.to_string())?;
    event("reviewCommitted");
    Ok(saved)
}
#[cfg(test)]
pub(super) fn accept_observed(
    storage: &Storage,
    receipt: &mut removal::Receipt,
    review: &Review,
    include_unregistered: bool,
    admission: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<(), String> {
    let next = accept_inner(
        storage,
        Source::Files(receipt),
        review,
        include_unregistered,
        admission,
        check,
        event,
    )?;
    *receipt = next;
    Ok(())
}
#[cfg(test)]
pub(super) fn accept_database_observed(
    storage: &Storage,
    receipt: &completion::Receipt,
    review: &Review,
    include_unregistered: bool,
    admission: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<removal::Receipt, String> {
    accept_inner(
        storage,
        Source::Database(receipt),
        review,
        include_unregistered,
        admission,
        check,
        event,
    )
}
