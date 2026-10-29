//! Typed restore boundary. Domains own their saved parameters/checkpoints;
//! backup supplies identity, hash and audio provenance resolution only.
use crate::saved_artifact::Role;
use crate::storage::{JobKind, JobSnapshot, KnownInput, RecordedAudioPlan};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReferenceDomain {
    Meeting,
    Analysis,
    Draft,
    Operation,
    Speaker,
    Track,
    UtteranceLineage,
    Person,
    Job,
}
impl ReferenceDomain {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Meeting => "meetings",
            Self::Analysis => "analysis_revisions",
            Self::Draft => "working_drafts",
            Self::Operation => "operation",
            Self::Speaker => "speakers",
            Self::Track => "audio_tracks",
            Self::UtteranceLineage => "utterance_lineage",
            Self::Person => "voice_people",
            Self::Job => "processing_jobs",
        }
    }
    pub(crate) fn global(self) -> bool {
        matches!(self, Self::Meeting | Self::Person)
    }
}
pub(crate) trait Resolver {
    fn id(&self, domain: ReferenceDomain, meeting: &str, id: &str) -> Result<String, String>;
    fn hash(
        &self,
        domain: ReferenceDomain,
        meeting: &str,
        id: &str,
        hash: &str,
    ) -> Result<String, String>;
    fn audio_plan(
        &self,
        meeting: &str,
        plan: &RecordedAudioPlan,
        known: Option<&KnownInput>,
    ) -> Result<RecordedAudioPlan, String>;
    fn person_exists(&self, id: &str) -> Result<bool, String>;
    fn manual_lineages(&self, meeting: &str, input: &str) -> Result<BTreeSet<String>, String>;
    fn request(&self, meeting: &str, value: &Value) -> Result<Value, String>;
    fn vocabulary(&self, meeting: &str, value: &Value) -> Result<Value, String>;
}

pub(crate) fn parameters(
    meeting: &str,
    input: Option<&str>,
    original: &Value,
    resolver: &impl Resolver,
) -> Result<Value, String> {
    if !original.is_object() {
        return Err("固定条件はオブジェクトである必要があります".into());
    }
    let mut value = original.clone();
    if let Some(request) = original.get("assignmentRequest").filter(|v| !v.is_null()) {
        value["assignmentRequest"] =
            crate::storage::transcript_consolidation::restore_request(meeting, request, resolver)?;
    }
    if let Some(plan) = original.get("completion") {
        value["completion"] =
            crate::speaker_completion::restore::parameters(meeting, plan, resolver)?;
    }
    if let Some(plan) = original.get("voice") {
        value["voice"] = crate::voice::restore::parameters(meeting, plan, resolver)?;
        value["voiceRequest"] = Value::Null;
    }
    crate::transcription::restore_parameters(meeting, original, &mut value, resolver)?;
    if let Some(request) = original.get("request") {
        value["request"] = resolver.request(meeting, request)?;
    }
    if let Some(vocabulary) = original.get("vocabulary").filter(|v| !v.is_null()) {
        value["vocabulary"] = resolver.vocabulary(meeting, vocabulary)?;
    }
    if let Some(plan) = original.get("diarization") {
        value["diarization"] =
            crate::diarization::restore_parameters(meeting, input, plan, resolver)?;
    }
    if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
        return Err("再構成した固定条件が容量上限を超えています".into());
    }
    Ok(value)
}

pub(crate) fn checkpoint(
    job: &JobSnapshot,
    parameters: &Value,
    value: Value,
    resolver: &impl Resolver,
    map: &mut impl FnMut(Role, &str, &str) -> Result<(String, String), String>,
) -> Result<Value, String> {
    match job.kind {
        JobKind::SpeakerCompletion => {
            crate::speaker_completion::restore::checkpoint(&job.meeting_id, value, resolver)
        }
        JobKind::VoiceRegistration | JobKind::VoiceMatching => {
            crate::voice::restore::checkpoint(value)
        }
        JobKind::Transcription => {
            crate::transcription::restore_checkpoint_references(job, value, map)
        }
        JobKind::Diarization => crate::diarization::restore_checkpoint_references(job, value, map),
        JobKind::MinutesGeneration => crate::generation::restore_checkpoint_references(
            job,
            parameters["generation"].clone(),
            value,
            map,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct Mapping;
    impl Resolver for Mapping {
        fn id(&self, domain: ReferenceDomain, meeting: &str, id: &str) -> Result<String, String> {
            Ok(format!(
                "{}:{}:{id}",
                domain.name(),
                if domain.global() { "" } else { meeting }
            ))
        }
        fn hash(&self, _: ReferenceDomain, _: &str, _: &str, _: &str) -> Result<String, String> {
            Ok("b".repeat(64))
        }
        fn audio_plan(
            &self,
            _: &str,
            plan: &RecordedAudioPlan,
            _: Option<&KnownInput>,
        ) -> Result<RecordedAudioPlan, String> {
            Ok(plan.clone())
        }
        fn person_exists(&self, _: &str) -> Result<bool, String> {
            Ok(false)
        }
        fn manual_lineages(&self, _: &str, _: &str) -> Result<BTreeSet<String>, String> {
            Ok(BTreeSet::new())
        }
        fn request(&self, _: &str, value: &Value) -> Result<Value, String> {
            Ok(value.clone())
        }
        fn vocabulary(&self, _: &str, value: &Value) -> Result<Value, String> {
            Ok(value.clone())
        }
    }
    #[test]
    fn assignment_domain_maps_fixed_identity_but_rejects_another_meeting() {
        let original = json!({"assignmentRequest":{"meetingId":"meeting","analysisId":"analysis",
            "expectedContentHash":"a".repeat(64),"expectedDraft":null,"operationId":"operation"},
            "unrelated":{"keep":"history"}});
        let mapped = parameters("meeting", None, &original, &Mapping).unwrap();
        assert_eq!(
            mapped["assignmentRequest"]["analysisId"],
            "analysis_revisions:meeting:analysis"
        );
        assert_eq!(
            mapped["assignmentRequest"]["expectedContentHash"],
            "b".repeat(64)
        );
        assert_eq!(
            mapped["assignmentRequest"]["meetingId"],
            "meetings::meeting"
        );
        assert_eq!(mapped["unrelated"], original["unrelated"]);
        assert!(parameters("another", None, &original, &Mapping).is_err());
        assert_eq!(original["assignmentRequest"]["analysisId"], "analysis");
    }
    #[test]
    fn completion_checkpoint_maps_results_without_erasing_history_or_missing_speakers() {
        let value = json!({"completionResult":true,"jobId":"job","outputAnalysisId":"analysis",
            "decisions":[{"lineage":"lineage","speakerId":null,"bodyBest":"speaker","reason":"historical"}]});
        let mapped =
            crate::speaker_completion::restore::checkpoint("meeting", value, &Mapping).unwrap();
        assert_eq!(mapped["jobId"], "processing_jobs:meeting:job");
        assert_eq!(
            mapped["outputAnalysisId"],
            "analysis_revisions:meeting:analysis"
        );
        assert_eq!(
            mapped["decisions"][0]["lineage"],
            "utterance_lineage:meeting:lineage"
        );
        assert!(mapped["decisions"][0]["speakerId"].is_null());
        assert_eq!(
            mapped["decisions"][0]["bodyBest"],
            "speakers:meeting:speaker"
        );
        assert_eq!(mapped["decisions"][0]["reason"], "historical");
        assert!(crate::speaker_completion::restore::checkpoint(
            "meeting",
            json!({"completionResult":true,"decisions":[{}]}),
            &Mapping
        )
        .is_err());
    }
    #[test]
    fn voice_history_keeps_its_result_and_rejects_an_untyped_checkpoint() {
        let history = json!({"voiceResult":false,"matches":["historical evidence"]});
        assert_eq!(
            crate::voice::restore::checkpoint(history.clone()).unwrap(),
            history
        );
        assert!(crate::voice::restore::checkpoint(json!({"voiceResult":"true"})).is_err());
        assert!(
            crate::speaker_completion::restore::checkpoint("meeting", json!({}), &Mapping).is_err()
        );
    }
}
