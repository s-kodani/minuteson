//! Decoding capacity only. This does not validate or authorize a recording.
//! Both current native lane envelopes may carry a
//! bounded whole-track map while physical chunks retain their existing limits.
use crate::current_formats::CAPTURE as CAPTURE_VERSION;
pub(crate) const SMALL_MANIFEST_BYTES: u64 = 256 * 1024;
pub(crate) const MAX_BYTES: u64 = 32 * 1024 * 1024;

pub(crate) fn validate_size(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err("録音manifestの容量が不正です".into());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if value["schemaVersion"] != CAPTURE_VERSION
        || !matches!(
            value["inputDiagnosticsState"].as_str(),
            Some("unavailable" | "unconfirmed" | "reported")
        )
    {
        return Err("非対応の録音manifest形式です".into());
    }
    if bytes.len() as u64 <= SMALL_MANIFEST_BYTES {
        return Ok(());
    }
    {
        use crate::recording_segments::{Lane, Status};
        use serde::Deserialize;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Track {
            raw_segments: Status,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Envelope {
            schema_version: u32,
            recording_owner_id: String,
            input_diagnostics_state: String,
            microphone: Track,
            system: Track,
        }
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let owner = &envelope.recording_owner_id;
        if envelope.schema_version == crate::current_formats::CAPTURE
            && ["unavailable", "unconfirmed", "reported"]
                .contains(&envelope.input_diagnostics_state.as_str())
            && crate::input_levels::valid_owner(owner)
            && envelope
                .microphone
                .raw_segments
                .capacity_owner(owner, Lane::Microphone)
            && envelope
                .system
                .raw_segments
                .capacity_owner(owner, Lane::System)
        {
            return Ok(());
        }
    }
    Err("原音声の所属を持たない録音manifestが旧容量上限を超えています".into())
}

#[cfg(test)]
mod tests;
