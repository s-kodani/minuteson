use super::limits::SAFE_INTEGER;
use serde::Deserialize;

pub fn valid_owner(owner: &str) -> bool {
    let b = owner.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            14 => *c == b'4',
            19 => matches!(*c, b'8' | b'9' | b'a' | b'b'),
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(c),
        })
}
pub fn valid_uid(uid: &str) -> bool {
    !uid.trim().is_empty() && uid.len() <= 1024 && !uid.chars().any(|c| c.is_control())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MicrophoneSelectionRequest {
    schema_version: u32,
    owner_id: String,
    pub revision: u64,
    mode: String,
    pub uid: Option<String>,
}
impl MicrophoneSelectionRequest {
    pub fn parse(bytes: &[u8], owner: &str) -> Result<Self, &'static str> {
        if bytes.is_empty() || bytes.len() > 4096 || !valid_owner(owner) {
            return Err("invalid microphone selection envelope");
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| "invalid microphone selection fields")?;
        if value.schema_version != 1
            || value.owner_id != owner
            || value.revision == 0
            || value.revision > SAFE_INTEGER
            || !match value.mode.as_str() {
                "automatic" => value.uid.is_none(),
                "fixed" => value.uid.as_deref().is_some_and(valid_uid),
                _ => false,
            }
        {
            return Err("invalid microphone selection policy");
        }
        Ok(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
    #[test]
    fn selection_rejects_invalid_corpus() {
        let mut value = serde_json::json!({"schemaVersion":1,"ownerId":OWNER,"revision":1,"mode":"automatic","uid":null});
        assert!(
            MicrophoneSelectionRequest::parse(&serde_json::to_vec(&value).unwrap(), OWNER).is_ok()
        );
        for (field, bad) in [
            ("revision", serde_json::json!(SAFE_INTEGER + 1)),
            ("mode", serde_json::json!("fixed")),
            ("uid", serde_json::json!("wrong-mode")),
            ("unknown", serde_json::json!(true)),
        ] {
            let mut changed = value.clone();
            changed[field] = bad;
            assert!(
                MicrophoneSelectionRequest::parse(&serde_json::to_vec(&changed).unwrap(), OWNER)
                    .is_err(),
                "{field}"
            );
        }
        value["mode"] = serde_json::json!("fixed");
        value["uid"] = serde_json::json!("device");
        assert!(
            MicrophoneSelectionRequest::parse(&serde_json::to_vec(&value).unwrap(), OWNER).is_ok()
        );
        for uid in ["", "  ", "device\u{0080}"] {
            value["uid"] = serde_json::json!(uid);
            assert!(
                MicrophoneSelectionRequest::parse(&serde_json::to_vec(&value).unwrap(), OWNER)
                    .is_err()
            );
        }
        assert!(!valid_owner("735027d4-a03e-13d3-830a-21e767dfaf88"));
    }
}
