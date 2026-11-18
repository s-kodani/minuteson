use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct CloseExpectations {
    pub import_token: Option<String>,
    pub data_token: Option<String>,
    #[serde(default)]
    pub recording_save: Option<RecordingSaveExpectation>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RecordingSaveExpectation {
    pub run_id: String,
    pub revision: u64,
    pub session_ids: Vec<String>,
}
impl RecordingSaveExpectation {
    pub(crate) fn require_complete(
        &self,
        snapshot: &crate::recording_recovery::Snapshot,
    ) -> Result<(), String> {
        let mut expected = self.session_ids.clone();
        expected.sort();
        let mut actual: Vec<_> = snapshot
            .items
            .iter()
            .map(|i| i.session_id.clone())
            .collect();
        actual.sort();
        if expected.windows(2).any(|ids| ids[0] == ids[1])
            || self.run_id != snapshot.run_id
            || self.revision != snapshot.revision
            || expected != actual
            || snapshot.phase != "completed"
            || snapshot.error.is_some()
        {
            return Err(
                "録音の保存対象または状態が変更されました。保存完了を再確認してください。".into(),
            );
        }
        let scoped: Vec<_> = snapshot
            .items
            .iter()
            .filter(|item| {
                snapshot.origin == "startup"
                    || snapshot.target_session_id.as_deref() == Some(&item.session_id)
            })
            .collect();
        if snapshot.origin != "startup" && scoped.len() != 1
            || scoped
                .iter()
                .any(|item| !["succeeded", "skipped"].contains(&item.phase.as_str()))
        {
            return Err(
                "録音の保存完了を確認できません。アプリを表示したままにしています。".into(),
            );
        }
        Ok(())
    }
}
pub(crate) fn require_target(
    expected: &Option<String>,
    actual: &Option<String>,
) -> Result<(), String> {
    if expected != actual {
        return Err(
            "終了対象の処理が変更されました。別の処理には取消・終了を送っていません".into(),
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_never_adopts_new_or_missing_operation_tokens() {
        assert!(require_target(&None, &None).is_ok());
        assert!(require_target(&Some("a".into()), &Some("a".into())).is_ok());
        assert!(require_target(&None, &Some("a".into())).is_err());
        assert!(require_target(&Some("a".into()), &None).is_err());
        assert!(require_target(&Some("a".into()), &Some("b".into())).is_err());
    }
    #[test]
    fn close_expectations_reject_unknown_fields_and_wrong_types() {
        assert!(serde_json::from_str::<CloseExpectations>(
            r#"{"importToken":null,"dataToken":null}"#
        )
        .is_ok());
        assert!(serde_json::from_str::<CloseExpectations>(
            r#"{"importToken":false,"dataToken":null}"#
        )
        .is_err());
        assert!(serde_json::from_str::<CloseExpectations>(
            r#"{"importToken":null,"dataToken":null,"force":true}"#
        )
        .is_err());
    }

    #[test]
    fn recording_save_exit_requires_the_same_inventory_revision_and_scoped_confirmed_result() {
        use crate::recording_recovery::{Item, Snapshot};
        let make = |session: &str, phase: &str| Item {
            session_id: session.into(),
            meeting_id: session.into(),
            meeting_title: "会議".into(),
            phase: phase.into(),
            started_at_ms: Some(100),
            error: None,
            progress: None,
            no_audio: false,
        };
        let mut snapshot = Snapshot {
            origin: "retry".into(),
            target_session_id: Some("current".into()),
            phase: "completed".into(),
            items: vec![make("old", "failed"), make("current", "succeeded")],
            ..Default::default()
        };
        let expected = RecordingSaveExpectation {
            run_id: snapshot.run_id.clone(),
            revision: snapshot.revision,
            session_ids: vec!["current".into(), "old".into()],
        };
        assert!(expected.require_complete(&snapshot).is_ok());
        snapshot.items[1].phase = "failed".into();
        assert!(expected.require_complete(&snapshot).is_err());
        snapshot.items[1].phase = "succeeded".into();
        snapshot.revision += 1;
        assert!(expected.require_complete(&snapshot).is_err());
        snapshot.revision = expected.revision;
        snapshot.items.push(make("new", "succeeded"));
        assert!(expected.require_complete(&snapshot).is_err());
        snapshot.items.pop();
        snapshot.origin = "startup".into();
        snapshot.target_session_id = None;
        assert!(expected.require_complete(&snapshot).is_err());
        snapshot.items[0].phase = "skipped".into();
        assert!(expected.require_complete(&snapshot).is_ok());
    }
}
