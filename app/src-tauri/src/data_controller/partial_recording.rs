//! Server-owned review capability. IDs/summary received over IPC cannot mint
//! it; refresh always needs a new explicit choice. Keeping changes no files.
use super::*;
use crate::analysis_worker::data_maintenance::DataMaintenanceLease;
use crate::storage::recordings::partial::{Review, Summary};

#[cfg(test)]
thread_local! {pub(super) static PANIC_AFTER_COMMIT:std::cell::Cell<bool>=const {std::cell::Cell::new(false)};}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    pub confirmation_id: String,
    pub summary: Summary,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    pub confirmation_id: String,
    pub summary: Summary,
    pub recovered: bool,
}

pub(super) struct Prepared {
    review: Review,
    confirmation: String,
}

impl Prepared {
    pub(super) fn fresh(
        storage: &Storage,
        session: &str,
        lease: &DataMaintenanceLease,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        lease.validate_active()?;
        let check = || cancelled() || lease.validate_active().is_err();
        let review = storage.review_partial_recording_checked(session, &check)?;
        review.summary.validate()?;
        Ok(Self {
            review,
            confirmation: Uuid::new_v4().to_string(),
        })
    }
    pub(super) fn preview(&self) -> Preview {
        Preview {
            confirmation_id: self.confirmation.clone(),
            summary: self.review.summary.clone(),
        }
    }
    pub(super) fn refresh(
        &mut self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        *self = Self::fresh(storage, &self.review.summary.session_id, lease, cancelled)?;
        Ok(())
    }
    pub(super) fn resolve(
        &self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        confirmation: &str,
        recover: bool,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<Receipt, String> {
        lease.validate_active()?;
        if confirmation != self.confirmation {
            return Err("部分救出の確認内容が変更されています".into());
        }
        let check = || cancelled() || lease.validate_active().is_err();
        if check() {
            return Err("部分救出の受付を取り消しました。音声は保持します".into());
        }
        if recover {
            progress("publishing");
            storage.recover_partial_recording_checked(&self.review, true, &check)?;
            #[cfg(test)]
            PANIC_AFTER_COMMIT.with(|inject| {
                if inject.replace(false) {
                    panic!("synthetic lost result after partial commit");
                }
            });
        }
        // A committed recovery wins over cancellation arriving afterwards.
        Ok(Receipt {
            confirmation_id: self.confirmation.clone(),
            summary: self.review.summary.clone(),
            recovered: recover,
        })
    }
    pub(super) fn committed_result(&self, storage: &Storage) -> Result<Option<Receipt>, String> {
        storage
            .partial_recovery_result(
                &self.review.summary.session_id,
                &self.review.summary.review_hash,
            )?
            .map(|summary| {
                if summary != self.review.summary {
                    return Err("保存済み部分救出と確認内容が一致しません".into());
                }
                Ok(Receipt {
                    confirmation_id: self.confirmation.clone(),
                    summary,
                    recovered: true,
                })
            })
            .transpose()
    }
}
