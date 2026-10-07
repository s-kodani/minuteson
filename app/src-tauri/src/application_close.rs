use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct CloseExpectations {
    pub import_token: Option<String>,
    pub data_token: Option<String>,
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
}
