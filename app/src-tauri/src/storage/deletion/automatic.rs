//! Finish only previously fixed deletion scopes; never create consent or adopt
//! changed files. Unstarted operations are cancelled without removing data.
use crate::{analysis_worker::data_maintenance::DataMaintenanceLease, storage::Storage};
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Keep,
    Finish,
    Preserve,
}

fn action(pending: bool, phase: &str, executable: bool, keep: bool) -> Action {
    if !pending {
        return Action::Preserve;
    }
    if phase == "not_started" && keep {
        return Action::Keep;
    }
    if executable
        && matches!(
            phase,
            "files" | "database_files" | "database" | "availability" | "awaiting_ack"
        )
    {
        Action::Finish
    } else {
        Action::Preserve
    }
}

pub(crate) fn meeting(
    storage: &Storage,
    lease: &DataMaintenanceLease,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<bool, String> {
    lease.validate_active()?;
    let mut prepared =
        super::operations::Prepared::pending(storage, operation, &|| lease.cancelled() || check())?;
    let p = prepared.preview();
    match action(p.pending, &p.phase, p.execute_allowed, p.keep_allowed) {
        Action::Keep => {
            prepared.keep(storage, lease, &p.confirmation_id, check)?;
        }
        Action::Finish => {
            prepared.execute(storage, lease, &p.confirmation_id, false, check, &|_| {})?;
        }
        Action::Preserve => return Ok(false),
    }
    Ok(true)
}

pub(crate) fn audio(
    storage: &Storage,
    lease: &DataMaintenanceLease,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<bool, String> {
    let mut prepared =
        super::audio::operations::Prepared::pending(storage, operation, lease, check)?;
    let p = prepared.preview();
    match action(p.pending, &p.phase, p.execute_allowed, p.keep_allowed) {
        Action::Keep => {
            prepared.keep(storage, lease, &p.confirmation_id, check)?;
        }
        Action::Finish => {
            prepared.execute(storage, lease, &p.confirmation_id, false, check, &|_| {})?;
        }
        Action::Preserve => return Ok(false),
    }
    Ok(true)
}

pub(crate) fn model(
    storage: &Storage,
    root: &Path,
    lease: &DataMaintenanceLease,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<bool, String> {
    let mut prepared =
        super::models::operations::Prepared::pending(storage, root, operation, lease, check)?;
    let p = prepared.preview();
    match action(p.pending, &p.phase, p.execute_allowed, p.keep_allowed) {
        Action::Keep => {
            prepared.keep(storage, lease, &p.confirmation_id, check)?;
        }
        Action::Finish => {
            prepared.execute(storage, lease, &p.confirmation_id, false, check, &|_| {})?;
        }
        Action::Preserve => return Ok(false),
    }
    Ok(true)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn lease() -> DataMaintenanceLease {
        crate::analysis_worker::AnalysisController::default()
            .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
            .unwrap()
    }

    #[test]
    fn automatic_audio_cleanup_cancels_unstarted_and_finishes_fixed_scope_only() {
        for started in [false, true] {
            let f = super::super::audio::tests::Fixture::new();
            let lease = lease();
            let mut p = super::super::audio::operations::Prepared::fresh(
                &f.storage,
                &f.meeting,
                &lease,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            let preview = p.preview();
            let stop = Cell::new(false);
            let stop_phase = if started {
                "deleting_audio_files"
            } else {
                "audio_delete_scope_fixed"
            };
            assert!(p
                .execute(
                    &f.storage,
                    &lease,
                    &preview.confirmation_id,
                    false,
                    &|| stop.get(),
                    &|phase| if phase == stop_phase {
                        stop.set(true)
                    }
                )
                .is_err());
            assert!(audio(
                &f.storage,
                &lease,
                &preview.operation_id,
                &crate::test_support::never_cancelled
            )
            .unwrap());
            assert_eq!(f.path.exists(), !started);
            assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
            assert!(super::super::audio::operations::listing(&f.storage)
                .unwrap()
                .items
                .is_empty());
        }
    }

    #[test]
    fn automatic_model_cleanup_cancels_unstarted_and_finishes_fixed_scope_only() {
        for started in [false, true] {
            let f = super::super::models::tests::Fixture::new();
            let lease = crate::analysis_worker::AnalysisController::default()
                .begin_model_deletion(false, false, &|| false)
                .unwrap();
            let mut p = super::super::models::operations::Prepared::fresh(
                &f.storage,
                &f.root,
                &f.package,
                &f.review,
                &lease,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            let preview = p.preview();
            let stop = Cell::new(false);
            let stop_phase = if started {
                "deleting_model_files"
            } else {
                "model_delete_scope_fixed"
            };
            assert!(p
                .execute(
                    &f.storage,
                    &lease,
                    &preview.confirmation_id,
                    false,
                    &|| stop.get(),
                    &|phase| if phase == stop_phase {
                        stop.set(true)
                    }
                )
                .is_err());
            assert!(
                model(&f.storage, &f.root, &lease, &preview.operation_id, &|| {
                    false
                })
                .unwrap()
            );
            assert_eq!(f.path("cli").exists(), !started);
            assert!(!f.storage.list_meetings().unwrap().is_empty());
            assert!(
                super::super::models::operations::listing(&f.storage, &f.root)
                    .unwrap()
                    .items
                    .is_empty()
            );
        }
    }
    #[test]
    fn automatic_cleanup_never_creates_or_extends_consent() {
        for phase in [
            "review",
            "reconfirmation",
            "database_reconfirmation",
            "restored_unavailable",
            "retained",
            "unexpected",
        ] {
            assert_eq!(action(true, phase, true, true), Action::Preserve);
        }
        for phase in [
            "files",
            "database_files",
            "database",
            "availability",
            "awaiting_ack",
        ] {
            assert_eq!(action(true, phase, true, false), Action::Finish);
            assert_eq!(action(true, phase, false, false), Action::Preserve);
            assert_eq!(action(false, phase, true, true), Action::Preserve);
        }
        assert_eq!(action(true, "not_started", true, true), Action::Keep);
        assert_eq!(action(true, "not_started", true, false), Action::Preserve);
    }
}
