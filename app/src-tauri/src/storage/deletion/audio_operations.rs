//! Server-held audio-only confirmations; serialized IDs/rows are not permits.
use super::{
    audio,
    intent::{self, reconfirmation, removal},
    Census, Entry, Retained, Snapshot, Track,
};
use crate::{analysis_worker::data_maintenance::DataMaintenanceLease, storage::Storage};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    pub confirmation_id: String,
    pub operation_id: String,
    pub meeting_id: String,
    pub source_operation_id: Option<String>,
    // Display attribution for a live new-scope review, never a permit.
    pub reconfirmation_source_kind: Option<String>,
    pub phase: String,
    pub pending: bool,
    pub execute_allowed: bool,
    pub resume_reason: Option<String>,
    pub keep_allowed: bool,
    pub reconfirmation_allowed: bool,
    pub include_unregistered_recorded: Option<bool>,
    pub path_count: usize,
    pub processed_files: usize,
    pub audio_files: usize,
    pub audio_bytes: u64,
    pub missing_audio_files: usize,
    pub changed_audio_files: usize,
    pub unregistered_audio_files: usize,
    pub unclassified_files: usize,
    pub retained: Retained,
    pub tracks: Vec<Track>,
    pub prior_deletion_completed: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PathPage {
    pub confirmation_id: String,
    pub offset: usize,
    pub total: usize,
    pub next_offset: Option<usize>,
    pub entries: Vec<Entry>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    pub operation_id: String,
    pub meeting_id: String,
    pub audio_deleted: bool,
    pub retained: bool,
    pub history_retained: bool,
    pub restart_required: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    pub operation_id: String,
    pub state: String,
    pub meeting_id: Option<String>,
    pub review_allowed: bool,
    pub error_message: Option<String>,
    pub startup_reason: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Listing {
    pub items: Vec<Item>,
    pub write_blocked: bool,
    pub restart_required: bool,
    pub reason: Option<String>,
}
enum Target {
    Fresh(Census),
    Unstarted(intent::Prepared),
    Files(removal::Receipt),
    Remaining(removal::renewal::Review),
    Reconfirmation(reconfirmation::Review),
    Restored(reconfirmation::Review),
    Kept(intent::Prepared),
}
impl Target {
    fn snapshot(&self) -> Snapshot {
        match self {
            Self::Fresh(c) => c.snapshot(),
            Self::Unstarted(p) | Self::Kept(p) => p.snapshot().clone(),
            Self::Files(r) => r.snapshot().clone(),
            Self::Remaining(r) => r.snapshot().clone(),
            Self::Reconfirmation(r) | Self::Restored(r) => r.snapshot().clone(),
        }
    }
    fn verify(
        &self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        match self {
            Self::Fresh(c) => c.verify(storage, &|| check() || lease.cancelled())?,
            Self::Unstarted(p) => p.verify_current(storage, lease, check)?,
            Self::Files(r) => r.verify_live(storage, lease, check)?,
            Self::Remaining(r) => r.verify_live(storage, lease, check)?,
            Self::Reconfirmation(r) => r.verify_live(storage, lease, check)?,
            Self::Restored(r) => {
                r.verify_live(storage, lease, check)?;
                return Err("復元元の利用不可記録には削除要求がありません。現在の音声を新しい操作で確認してください".into());
            }
            Self::Kept(_) => return Err("この音声削除は未開始のまま保持・中止済みです".into()),
        }
        active(lease, check)
    }
}
fn active(lease: &DataMaintenanceLease, check: &impl Fn() -> bool) -> Result<(), String> {
    lease.validate_active()?;
    super::cancel(check)
}
pub(crate) fn authorize_review(storage: &Storage, operation: Option<&str>) -> Result<(), String> {
    if let Some(operation) = operation {
        intent::authorize_review(storage, operation)
    } else {
        storage.require_data_writable()
    }
}
enum Loaded {
    Native(Target),
    Restored(super::restored::Source),
}
impl Loaded {
    fn meeting(&self) -> String {
        match self {
            Self::Native(t) => t.snapshot().scope.meeting_id,
            Self::Restored(r) => r.meeting().to_owned(),
        }
    }
    fn live(
        self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<Target, String> {
        match self {
            Self::Native(t) => Ok(t),
            Self::Restored(r) => Ok(Target::Restored(reconfirmation::Review::capture(
                storage,
                r.operation(),
                lease,
                check,
            )?)),
        }
    }
}
fn load(storage: &Storage, operation: &str) -> Result<Loaded, String> {
    intent::authorize_review(storage, operation)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let state: String = db
        .query_row(
            "SELECT state FROM maintenance_operations WHERE id=?1 AND kind='delete_assets'",
            params![operation],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if super::restored::hinted(&db, operation)? {
        return Ok(Loaded::Restored(super::restored::Source::load(
            &db,
            operation,
            &|| false,
        )?));
    }
    let target = match state.as_str() {
        "prepared" => Target::Unstarted(intent::read(&db, operation, &|| false)?),
        "recovered" => Target::Kept(intent::read(&db, operation, &|| false)?),
        "installing" | "cleanup" | "completed" => {
            Target::Files(removal::Receipt::load(&db, operation, &|| false)?)
        }
        _ => return Err("この音声削除記録は自動採用できません。履歴と対象を保持します".into()),
    };
    Ok(Loaded::Native(target))
}
pub(crate) fn listing(storage: &Storage) -> Result<Listing, String> {
    let rows = {
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        storage.managed_anchor.verify()?;
        // Routing hints only. Unknown rows remain represented and fenced;
        // this list never grants admission or suppresses startup validation.
        let sql = format!("SELECT id,state FROM maintenance_operations WHERE kind='delete_assets' AND {} AND coalesce(CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 AND json_valid(target_manifest_json) THEN json_extract(target_manifest_json,'$.subtype') ELSE NULL END,'')!='modelDeletion' ORDER BY created_at,id LIMIT 101",super::super::fence::predicate(&db)?);
        let mut q = db.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        let collected = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        collected
    };
    if rows.len() > 100 {
        return Err("音声削除の確認待ち件数が安全上限を超えています".into());
    }
    let mut items = Vec::new();
    for (operation_id, state) in rows {
        let startup_reason = {
            let db = storage
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            super::super::startup::reason(&db, &operation_id)?
        };
        let target = load(storage, &operation_id);
        items.push(match target {
            Ok(target) => Item {
                meeting_id: Some(target.meeting()),
                operation_id,
                state,
                review_allowed: true,
                error_message: None,
                startup_reason,
            },
            Err(error) => Item {
                meeting_id: None,
                operation_id,
                state,
                review_allowed: false,
                error_message: Some(error),
                startup_reason,
            },
        });
    }
    Ok(Listing {
        items,
        write_blocked: storage.data_write_blocked(),
        restart_required: storage.restart_required(),
        reason: storage.require_data_writable().err(),
    })
}
pub(crate) struct Prepared {
    preview: Preview,
    target: Target,
}
impl Prepared {
    fn static_preview(operation: &str, target: &Target) -> Result<Preview, String> {
        let s = target.snapshot();
        let entries = s.entries()?;
        let audio_entries: Vec<_> = entries.iter().filter(|e| audio(&e.disposition)).collect();
        let audio_bytes = audio_entries.iter().try_fold(0u64, |n, e| {
            n.checked_add(e.bytes.unwrap_or(0))
                .ok_or("音声容量が範囲外です")
        })?;
        let (phase, policy, source, processed, completed) = match target {
            Target::Fresh(_) => ("review", None, None, 0, false),
            Target::Unstarted(p) => (
                "not_started",
                Some(p.policy()),
                p.origin_operation(),
                0,
                false,
            ),
            Target::Kept(p) => ("retained", Some(p.policy()), p.origin_operation(), 0, false),
            Target::Files(r) => (
                if r.completed() {
                    "awaiting_ack"
                } else if r.files_removed() {
                    "availability"
                } else {
                    "files"
                },
                Some(r.policy()),
                r.confirmed().origin_operation(),
                r.processed_files(),
                r.completed(),
            ),
            Target::Reconfirmation(r) => (
                "reconfirmation",
                None,
                Some(r.source_operation()),
                0,
                !r.source_was_restored(),
            ),
            Target::Remaining(r) => ("reconfirmation", None, Some(r.operation()), 0, false),
            Target::Restored(_) => ("restored_unavailable", None, None, 0, false),
        };
        Ok(Preview {
            confirmation_id: Uuid::new_v4().to_string(),
            operation_id: operation.into(),
            meeting_id: s.scope.meeting_id.clone(),
            source_operation_id: source.map(str::to_owned),
            reconfirmation_source_kind: match target {
                Target::Remaining(_) => Some("native_audio_pending".into()),
                Target::Reconfirmation(r) => Some(
                    if r.source_was_restored() {
                        "restored_audio_unavailable"
                    } else {
                        "native_audio_deletion"
                    }
                    .into(),
                ),
                _ => None,
            },
            phase: phase.into(),
            pending: !matches!(target, Target::Fresh(_) | Target::Kept(_)),
            execute_allowed: false,
            resume_reason: None,
            keep_allowed: matches!(target, Target::Unstarted(_) | Target::Kept(_)),
            reconfirmation_allowed: false,
            include_unregistered_recorded: policy.map(|p| p.include_unregistered_audio),
            path_count: entries.len(),
            processed_files: processed,
            audio_files: audio_entries.len(),
            audio_bytes,
            missing_audio_files: audio_entries.iter().filter(|e| !e.present).count(),
            changed_audio_files: audio_entries.iter().filter(|e| e.changed).count(),
            unregistered_audio_files: audio_entries
                .iter()
                .filter(|e| e.asset_id.is_none())
                .count(),
            unclassified_files: entries
                .iter()
                .filter(|e| e.disposition == super::Disposition::Unclassified)
                .count(),
            retained: s.retained()?,
            tracks: s.records.tracks.clone(),
            prior_deletion_completed: completed,
        })
    }
    fn build(
        storage: &Storage,
        operation: String,
        target: Target,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        active(lease, check)?;
        let mut preview = Self::static_preview(&operation, &target)?;
        match target.verify(storage, lease, check) {
            Ok(()) => preview.execute_allowed = true,
            Err(reason) => preview.resume_reason = Some(reason),
        }
        if !preview.execute_allowed {
            preview.reconfirmation_allowed =
                Self::can_reconfirm(storage, &operation, &target, lease, check);
        }
        active(lease, check)?;
        Ok(Self { preview, target })
    }
    fn can_reconfirm(
        storage: &Storage,
        operation: &str,
        target: &Target,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> bool {
        match target {
            Target::Files(r) if !r.completed() => {
                removal::renewal::Review::capture(storage, operation, lease, check).is_ok()
            }
            Target::Files(_) | Target::Restored(_) => {
                reconfirmation::Review::capture(storage, operation, lease, check).is_ok()
            }
            _ => false,
        }
    }
    pub(crate) fn fresh(
        storage: &Storage,
        meeting: &str,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        active(lease, check)?;
        authorize_review(storage, None)?;
        let prior: Option<String> = {
            let db = storage
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            // A previous completion requires explicit review, never another
            // v1 consent that silently reuses unavailable audio history.
            db.query_row("SELECT id FROM maintenance_operations WHERE meeting_id=?1 AND kind='delete_assets' AND state='completed' ORDER BY created_at DESC,id DESC LIMIT 1",params![meeting],|r|r.get(0)).optional().map_err(|e|e.to_string())?
        };
        if let Some(id) = prior {
            return Self::pending(storage, &id, lease, check);
        }
        storage.preserve_voice_audio(Some(meeting), check)?;
        let census = Census::capture(storage, meeting, &|| check() || lease.cancelled())?;
        Self::build(
            storage,
            Uuid::new_v4().to_string(),
            Target::Fresh(census),
            lease,
            check,
        )
    }
    pub(crate) fn pending(
        storage: &Storage,
        operation: &str,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        active(lease, check)?;
        Self::build(
            storage,
            operation.into(),
            load(storage, operation)?.live(storage, lease, check)?,
            lease,
            check,
        )
    }
    pub(crate) fn admission_operation(&self) -> Option<&str> {
        match &self.target {
            Target::Fresh(_) => None,
            Target::Reconfirmation(r) => Some(r.source_operation()),
            _ => Some(&self.preview.operation_id),
        }
    }
    pub(crate) fn preview(&self) -> Preview {
        self.preview.clone()
    }
    fn confirmation(&self, confirmation: &str) -> Result<(), String> {
        if self.preview.confirmation_id == confirmation {
            Ok(())
        } else {
            Err("音声削除の確認内容が変更されています。現在の一覧を再確認してください".into())
        }
    }
    pub(crate) fn page(&self, confirmation: &str, offset: usize) -> Result<PathPage, String> {
        self.confirmation(confirmation)?;
        let entries = self.target.snapshot().entries()?;
        if offset > entries.len() || offset % 50 != 0 {
            return Err("音声削除一覧の位置が不正です".into());
        }
        let end = offset.saturating_add(50).min(entries.len());
        Ok(PathPage {
            confirmation_id: confirmation.into(),
            offset,
            total: entries.len(),
            next_offset: (end < entries.len()).then_some(end),
            entries: entries[offset..end].to_vec(),
        })
    }
    pub(crate) fn refresh(
        &mut self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        let target =
            match &self.target {
                Target::Fresh(_) => {
                    Target::Fresh(Census::capture(storage, &self.preview.meeting_id, &|| {
                        check() || lease.cancelled()
                    })?)
                }
                Target::Reconfirmation(r) => Target::Reconfirmation(
                    reconfirmation::Review::capture(storage, r.source_operation(), lease, check)?,
                ),
                Target::Remaining(r) => Target::Remaining(removal::renewal::Review::capture(
                    storage,
                    r.operation(),
                    lease,
                    check,
                )?),
                _ => load(storage, &self.preview.operation_id)?.live(storage, lease, check)?,
            };
        *self = Self::build(
            storage,
            self.preview.operation_id.clone(),
            target,
            lease,
            check,
        )?;
        Ok(())
    }
    pub(crate) fn review_remaining(
        &mut self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        confirmation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        self.confirmation(confirmation)?;
        if !self.preview.reconfirmation_allowed {
            return Err(
                "この音声削除段階では追加削除要求を開始できません。残りを保持します".into(),
            );
        }
        let (operation, target) = match &self.target {
            Target::Files(r) if !r.completed() => (
                self.preview.operation_id.clone(),
                Target::Remaining(removal::renewal::Review::capture(
                    storage,
                    &self.preview.operation_id,
                    lease,
                    check,
                )?),
            ),
            Target::Files(_) | Target::Restored(_) => (
                Uuid::new_v4().to_string(),
                Target::Reconfirmation(reconfirmation::Review::capture(
                    storage,
                    &self.preview.operation_id,
                    lease,
                    check,
                )?),
            ),
            _ => return Err("この音声削除段階では追加削除要求を開始できません".into()),
        };
        *self = Self::build(storage, operation, target, lease, check)?;
        Ok(())
    }
    pub(crate) fn execute(
        &mut self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        confirmation: &str,
        include: bool,
        check: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<Receipt, String> {
        active(lease, check)?;
        self.confirmation(confirmation)?;
        if !self.preview.execute_allowed {
            return Err(self
                .preview
                .resume_reason
                .clone()
                .unwrap_or("音声削除対象を再確認してください".into()));
        }
        if include
            && !matches!(
                self.target,
                Target::Fresh(_) | Target::Reconfirmation(_) | Target::Remaining(_)
            )
        {
            return Err("保存済み音声削除の方針を変更できません".into());
        }
        let result = self.execute_inner(storage, lease, include, check, progress);
        if result.is_err() {
            if let Ok(target) = load(storage, &self.preview.operation_id)
                .and_then(|t| t.live(storage, lease, check))
            {
                self.target = target;
            }
            if let Ok(p) = Self::static_preview(&self.preview.operation_id, &self.target) {
                self.preview = p;
            }
            self.preview.execute_allowed = false;
            self.preview.resume_reason = Some(
                "処理を停止しました。既に消した音声は復元せず、残りの状態を再検査してください"
                    .into(),
            );
            self.preview.reconfirmation_allowed = Self::can_reconfirm(
                storage,
                &self.preview.operation_id,
                &self.target,
                lease,
                check,
            );
        }
        result
    }
    fn execute_inner(
        &mut self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        include: bool,
        check: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<Receipt, String> {
        self.target.verify(storage, lease, check)?;
        let policy = intent::Policy {
            include_unregistered_audio: include,
            retain_unclassified: true,
        };
        if let Target::Fresh(c) = &self.target {
            progress("fixing_audio_delete_scope");
            let p = intent::prepare(
                storage,
                &self.preview.operation_id,
                c,
                policy.clone(),
                lease,
                check,
            )?;
            self.target = Target::Unstarted(p);
            self.preview.pending = true;
            self.preview.keep_allowed = true;
            progress("audio_delete_scope_fixed");
        }
        if let Target::Reconfirmation(r) = &self.target {
            progress("fixing_audio_delete_scope");
            let p = reconfirmation::accept(
                storage,
                &self.preview.operation_id,
                r,
                policy,
                lease,
                check,
            )?;
            self.target = Target::Unstarted(p);
            self.preview.pending = true;
            self.preview.keep_allowed = true;
            progress("audio_delete_scope_fixed");
        }
        if let Target::Remaining(r) = &self.target {
            progress("fixing_audio_delete_scope");
            let receipt = removal::renewal::accept(storage, r, include, lease, check)?;
            self.target = Target::Files(receipt);
            self.preview.pending = true;
            self.preview.keep_allowed = false;
            progress("audio_delete_scope_fixed");
        }
        if matches!(self.target, Target::Unstarted(_)) {
            self.target = Target::Files(removal::start(
                storage,
                &self.preview.operation_id,
                lease,
                check,
            )?);
            self.preview.keep_allowed = false;
        }
        if matches!(self.target, Target::Files(_)) {
            // A lost completion response only needs typed absence/availability
            // verification. Do not enter a pending writer for completed history.
            if !matches!(&self.target, Target::Files(r) if r.completed()) {
                progress("deleting_audio_files");
                removal::run(storage, &self.preview.operation_id, lease, check)?;
            }
            progress("fixing_audio_unavailable");
            let done =
                removal::completion::finish(storage, &self.preview.operation_id, lease, check)?;
            self.target = Target::Files(done);
            self.preview = Self::static_preview(&self.preview.operation_id, &self.target)?;
            self.preview.phase = "completed".into();
            self.preview.pending = false;
            self.preview.execute_allowed = false;
            progress("audio_delete_completed");
            Ok(self.receipt(storage, true))
        } else {
            Err("保持・中止済みの音声削除は実行できません".into())
        }
    }
    fn receipt(&self, storage: &Storage, deleted: bool) -> Receipt {
        Receipt {
            operation_id: self.preview.operation_id.clone(),
            meeting_id: self.preview.meeting_id.clone(),
            audio_deleted: deleted,
            retained: !deleted,
            history_retained: true,
            restart_required: storage.restart_required(),
        }
    }
    pub(crate) fn keep(
        &mut self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        confirmation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Receipt, String> {
        active(lease, check)?;
        self.confirmation(confirmation)?;
        if !self.preview.keep_allowed {
            return Err("保持できるのは音声削除を開始していない記録だけです".into());
        }
        intent::keep_unstarted(storage, &self.preview.operation_id, lease, check)?;
        self.target = load(storage, &self.preview.operation_id)?.live(storage, lease, check)?;
        self.preview = Self::static_preview(&self.preview.operation_id, &self.target)?;
        self.preview.pending = false;
        Ok(self.receipt(storage, false))
    }
}
