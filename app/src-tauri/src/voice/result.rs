//! The voice domain owns the shape of its completed checkpoint.
use crate::storage::JobSnapshot;
use serde_json::Value;

pub(crate) fn validate_notification(
    job: &JobSnapshot,
    checkpoint: Option<&Value>,
) -> Result<(), String> {
    if checkpoint.and_then(|c| c["voiceResult"].as_bool()) != Some(true)
        || job.completed_units != 1
        || job.total_units != Some(1)
        || job.output_analysis_id.is_some()
        || job.output_minutes_id.is_some()
    {
        return Err("声の処理の完了記録が一致しません".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn voice_summary_accepts_only_completed_single_unit_without_published_revision() {
        let mut job:JobSnapshot=serde_json::from_value(json!({
            "id":"job","meetingId":"meeting","kind":"voice_matching","state":"succeeded",
            "operationId":"operation","inputHash":"input","specHash":"spec",
            "completedUnits":1,"totalUnits":1,"errorCode":null,"createdAt":1,"updatedAt":1,
            "outputAnalysisId":null,"outputMinutesId":null,"pauseReason":null,"retryOfJobId":null,"attempt":1
        })).unwrap();
        assert!(validate_notification(&job, Some(&json!({"voiceResult":true}))).is_ok());
        for receipt in [
            json!({"voiceResult":false}),
            json!({"voiceResult":"true"}),
            json!({}),
        ] {
            assert!(validate_notification(&job, Some(&receipt)).is_err());
        }
        assert!(validate_notification(&job, None).is_err());
        job.output_analysis_id = Some("unrelated".into());
        assert!(validate_notification(&job, Some(&json!({"voiceResult":true}))).is_err());
        job.output_analysis_id = None;
        job.completed_units = 0;
        assert!(validate_notification(&job, Some(&json!({"voiceResult":true}))).is_err());
    }
}
