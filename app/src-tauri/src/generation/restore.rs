//! Semantic replay of current Markdown generation in an owned restore workspace.
use super::*;

pub(crate) struct Context<'a> {
    pub job: &'a JobSnapshot,
    pub spec: &'a JobSpec,
    pub input: &'a MinutesInput,
    pub initial_operation: &'a str,
}

pub(crate) use crate::saved_artifact::RestoreAssets as Assets;

pub(crate) fn checkpoint(
    old: Context<'_>,
    new: Context<'_>,
    value: Value,
    assets: &mut impl Assets,
    check: &impl Fn() -> bool,
) -> Result<Value, String> {
    if !markdown_job::is_markdown(old.spec) || !markdown_job::is_markdown(new.spec) {
        return Err(REMOVED_GENERATION.into());
    }
    markdown_job::restore::checkpoint(old, new, value, assets, check)
}
