use super::*;
use crate::saved_artifact::{RestoreAssets, Role};
use serde_json::Value;

pub(crate) fn manual_assignment_hash(
    lineages: &std::collections::BTreeSet<String>,
) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(lineages).map_err(|e| e.to_string())?)
    ))
}
pub(crate) fn restore_parameters(
    meeting: &str,
    input: Option<&str>,
    original: &Value,
    resolver: &impl crate::job_codec::Resolver,
) -> Result<Value, String> {
    use crate::job_codec::ReferenceDomain as Domain;
    let mut value = original.clone();
    let source: crate::storage::RecordedAudioPlan = serde_json::from_value(
        original
            .get("source")
            .ok_or("話者分離の固定音声がありません")?
            .clone(),
    )
    .map_err(|e| e.to_string())?;
    let known: Option<crate::storage::KnownInput> = original
        .get("knownInput")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|e| e.to_string())?;
    value["source"] =
        serde_json::to_value(resolver.audio_plan(meeting, &source, known.as_ref())?)
            .map_err(|e| e.to_string())?;
    let mut tracks: Vec<super::TrackSettings> =
        serde_json::from_value(original["tracks"].clone()).map_err(|e| e.to_string())?;
    for track in &mut tracks {
        track.track_id = resolver.id(Domain::Track, meeting, &track.track_id)?;
    }
    value["tracks"] = serde_json::to_value(tracks).map_err(|e| e.to_string())?;
    let old = resolver.manual_lineages(meeting, input.ok_or("話者分離の固定入力がありません")?)?;
    if original.get("manualAssignmentHash").and_then(Value::as_str)
        != Some(&manual_assignment_hash(&old)?)
    {
        return Err("話者分離の固定割り当てが元版と一致しません".into());
    }
    let new = old
        .iter()
        .map(|id| resolver.id(Domain::UtteranceLineage, meeting, id))
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    value["manualAssignmentHash"] = serde_json::json!(manual_assignment_hash(&new)?);
    Ok(value)
}

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
