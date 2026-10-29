//! Preserve historical completion evidence while mapping application identities.
use super::Plan;
use crate::job_codec::{ReferenceDomain as Domain, Resolver};
use serde_json::{json, Value};

pub(crate) fn parameters(
    meeting: &str,
    value: &Value,
    resolver: &impl Resolver,
) -> Result<Value, String> {
    let mut plan: Plan = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    plan.source = resolver.audio_plan(meeting, &plan.source, Some(&plan.known))?;
    for window in &mut plan.references {
        window.speaker_id = resolver.id(Domain::Speaker, meeting, &window.speaker_id)?;
        window.track_id = resolver.id(Domain::Track, meeting, &window.track_id)?;
    }
    for target in &mut plan.targets {
        target.lineage = resolver.id(Domain::UtteranceLineage, meeting, &target.lineage)?;
        target.body.track_id = resolver.id(Domain::Track, meeting, &target.body.track_id)?;
    }
    // Historical working hashes remain source evidence, not fresh consent.
    serde_json::to_value(plan).map_err(|e| e.to_string())
}
pub(crate) fn checkpoint(
    meeting: &str,
    mut value: Value,
    resolver: &impl Resolver,
) -> Result<Value, String> {
    if value["completionResult"].as_bool().is_none() {
        return Err("話者補完の再開情報が不正です".into());
    }
    if let Some(job) = value["jobId"].as_str() {
        value["jobId"] = json!(resolver.id(Domain::Job, meeting, job)?);
    }
    if let Some(id) = value["outputAnalysisId"].as_str() {
        value["outputAnalysisId"] = json!(resolver.id(Domain::Analysis, meeting, id)?);
    }
    if let Some(rows) = value["decisions"].as_array_mut() {
        for row in rows {
            let lineage = row["lineage"]
                .as_str()
                .ok_or("話者補完の発言がありません")?;
            row["lineage"] = json!(resolver.id(Domain::UtteranceLineage, meeting, lineage)?);
            for key in ["speakerId", "bodyBest"] {
                if let Some(id) = row[key].as_str() {
                    row[key] = json!(resolver.id(Domain::Speaker, meeting, id)?);
                }
            }
        }
    }
    Ok(value)
}
