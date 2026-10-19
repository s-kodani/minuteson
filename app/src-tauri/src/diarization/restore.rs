use super::*;
use crate::saved_artifact::{RestoreAssets, Role};
use serde_json::Value;

#[allow(clippy::too_many_arguments)]
pub(crate) fn restore_saved_results(
    job: &JobSnapshot,
    old: &JobSpec,
    new: &JobSpec,
    value: Value,
    old_initial: &str,
    new_initial: &str,
    assets: &mut impl RestoreAssets,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    known::restore_saved_results(
        job,
        old,
        new,
        value,
        old_initial,
        new_initial,
        assets,
        cancelled,
    )
}
