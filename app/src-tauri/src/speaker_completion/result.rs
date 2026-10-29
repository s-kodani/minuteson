//! Decode and summarize this domain's historical completion receipt.
use crate::storage::JobSnapshot;
use serde_json::Value;

pub(crate) trait OutputResolver {
    fn verify(&self, job: &JobSnapshot, analysis_id: &str) -> Result<(), String>;
}
pub(crate) fn notification_summary(
    job: &JobSnapshot,
    checkpoint: Option<&Value>,
    resolver: &impl OutputResolver,
) -> Result<Vec<String>, String> {
    let c = checkpoint.ok_or("話者補完の完了記録がありません")?;
    if c["completionResult"] != true
        || job.completed_units != 1
        || job.total_units != Some(1)
        || c["jobId"] != job.id
        || job.output_analysis_id.is_some()
        || job.output_minutes_id.is_some()
    {
        return Err("話者補完の完了記録が一致しません".into());
    }
    let count = c["updatedCount"]
        .as_u64()
        .ok_or("話者補完の件数が不正です")?;
    if let Some(id) = c["outputAnalysisId"].as_str() {
        if count == 0 {
            return Err("話者補完の保存版が一致しません".into());
        }
        resolver.verify(job, id)?;
    } else if count != 0 {
        return Err("話者補完の保存版がありません".into());
    }
    let skipped = c["skipped"]
        .as_object()
        .ok_or("話者補完の見送り件数がありません")?
        .values()
        .map(|v| v.as_u64().unwrap_or(0))
        .try_fold(0u64, |total, n| total.checked_add(n))
        .ok_or("話者補完の見送り件数が不正です")?;
    let mut details = vec![format!("{}件を補完、{}件を見送りました", count, skipped)];
    if let Some(n) = c["needsReviewCount"].as_u64().filter(|n| *n > 0) {
        details.push(format!("うち{}件は低信頼のため要確認です", n));
    }
    for (reason, n) in c["skipped"].as_object().unwrap() {
        let label = match reason.as_str() {
            "manual" => "手動指定を保護",
            "detected_interval" => "検出済み区間あり",
            "too_short" => "発言が1秒未満",
            "too_long" => "切り出しが30秒超",
            "missing_audio" => "音声欠損・不連続",
            "feature_unavailable" => "特徴量を取得できず",
            "no_reference" => "参照音声なし",
            "tie" => "候補が同点",
            "different_winners" => "本体と前後付きの候補が不一致",
            "other_speaker_in_context" => "前後に別話者あり",
            "below_threshold" => "閾値未達",
            _ => "条件外",
        };
        details.push(format!("{}：{}件", label, n));
    }
    Ok(details)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    fn job() -> JobSnapshot {
        serde_json::from_value(json!({
            "id":"job","meetingId":"meeting","kind":"speaker_completion","state":"succeeded",
            "operationId":"operation","inputHash":"input","specHash":"spec",
            "completedUnits":1,"totalUnits":1,"errorCode":null,"createdAt":1,"updatedAt":1,
            "outputAnalysisId":null,"outputMinutesId":null,"pauseReason":null,"retryOfJobId":null,"attempt":1
        })).unwrap()
    }
    struct Proof {
        calls: Cell<usize>,
        accepted: bool,
    }
    impl OutputResolver for Proof {
        fn verify(&self, job: &JobSnapshot, id: &str) -> Result<(), String> {
            assert_eq!(job.meeting_id, "meeting");
            assert_eq!(job.id, "job");
            assert_eq!(id, "analysis");
            self.calls.set(self.calls.get() + 1);
            if self.accepted {
                Ok(())
            } else {
                Err("saved output differs".into())
            }
        }
    }
    #[test]
    fn completion_summary_requires_exact_persisted_output_and_preserves_review_details() {
        let proof = Proof {
            calls: Cell::new(0),
            accepted: true,
        };
        let receipt = json!({"completionResult":true,"jobId":"job","updatedCount":2,"needsReviewCount":1,
            "outputAnalysisId":"analysis","skipped":{"manual":1,"future_reason":2}});
        let result = notification_summary(&job(), Some(&receipt), &proof).unwrap();
        assert_eq!(proof.calls.get(), 1);
        assert_eq!(
            result,
            vec![
                "2件を補完、3件を見送りました",
                "うち1件は低信頼のため要確認です",
                "条件外：2件",
                "手動指定を保護：1件"
            ]
        );
        assert!(notification_summary(
            &job(),
            Some(&receipt),
            &Proof {
                calls: Cell::new(0),
                accepted: false
            }
        )
        .is_err());
        let mut bad = receipt.clone();
        bad["jobId"] = json!("other-job");
        assert!(notification_summary(&job(), Some(&bad), &proof).is_err());
        assert_eq!(
            proof.calls.get(),
            1,
            "wrong job must not resolve a saved analysis"
        );
    }
    #[test]
    fn zero_update_history_needs_no_output_and_corrupt_count_cannot_overflow() {
        let proof = Proof {
            calls: Cell::new(0),
            accepted: true,
        };
        let receipt = json!({"completionResult":true,"jobId":"job","updatedCount":0,"outputAnalysisId":null,"skipped":{"manual":2}});
        assert_eq!(
            notification_summary(&job(), Some(&receipt), &proof).unwrap()[0],
            "0件を補完、2件を見送りました"
        );
        assert_eq!(proof.calls.get(), 0);
        let mut bad = receipt.clone();
        bad["skipped"] = json!({"manual":u64::MAX,"missing_audio":1});
        assert!(notification_summary(&job(), Some(&bad), &proof).is_err());
        bad = receipt.clone();
        bad["updatedCount"] = json!(1);
        assert!(notification_summary(&job(), Some(&bad), &proof).is_err());
        assert!(notification_summary(&job(), None, &proof).is_err());
    }
}
