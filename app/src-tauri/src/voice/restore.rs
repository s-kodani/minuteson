//! Voice history must not resurrect a deleted person or become a runnable plan.
use super::Plan;
use crate::job_codec::{ReferenceDomain as Domain, Resolver};
use serde_json::Value;

pub(crate) fn parameters(
    meeting: &str,
    value: &Value,
    resolver: &impl Resolver,
) -> Result<Value, String> {
    let mut plan: Plan = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    for window in &mut plan.windows {
        window.speaker_id = resolver.id(Domain::Speaker, meeting, &window.speaker_id)?;
        window.track_id = resolver.id(Domain::Track, meeting, &window.track_id)?;
    }
    if let Some(person) = plan.person_id.as_deref() {
        plan.person_id = if resolver.person_exists(person)? {
            Some(resolver.id(Domain::Person, meeting, person)?)
        } else {
            None
        };
    }
    // Registry/editor hashes keep their original historical meaning.
    serde_json::to_value(plan).map_err(|e| e.to_string())
}
pub(crate) fn checkpoint(value: Value) -> Result<Value, String> {
    if value.get("voiceResult").and_then(Value::as_bool).is_none() {
        return Err("声の処理の再開情報が不正です".into());
    }
    Ok(value)
}
