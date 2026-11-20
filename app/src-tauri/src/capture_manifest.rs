//! Decoding capacity only. This does not validate or authorize a recording.
//! Both current native lane envelopes may carry a
//! bounded whole-track map while physical chunks retain their existing limits.
use crate::current_formats::CAPTURE as CAPTURE_VERSION;
pub(crate) const SMALL_MANIFEST_BYTES: u64 = 256 * 1024;
pub(crate) const MAX_BYTES: u64 = 32 * 1024 * 1024;

// Value normally keeps the last duplicate field. Capture evidence must have
// one unambiguous meaning before either the new or legacy adapter sees it.
struct UniqueValue(serde_json::Value);
impl<'de> serde::Deserialize<'de> for UniqueValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate object fields")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(serde_json::Value::Null))
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|v| UniqueValue(v.into()))
                    .ok_or_else(|| E::custom("nonfinite capture value"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(values.into()))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, UniqueValue(value))) =
                    map.next_entry::<String, UniqueValue>()?
                {
                    if values.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate capture field"));
                    }
                }
                Ok(UniqueValue(values.into()))
            }
        }
        d.deserialize_any(Visitor)
    }
}

pub(crate) fn validate_size(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err("録音manifestの容量が不正です".into());
    }
    let value = serde_json::from_slice::<UniqueValue>(bytes)
        .map_err(|e| e.to_string())?
        .0;
    if ![CAPTURE_VERSION, crate::current_formats::LEGACY_CAPTURE]
        .iter()
        .any(|v| value["schemaVersion"] == *v)
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
    if value["schemaVersion"] == CAPTURE_VERSION
        && crate::input_levels::valid_owner(value["recordingOwnerId"].as_str().unwrap_or(""))
        && value["singleWav"]["schemaVersion"] == 1
        && value["singleWav"]["ownerId"] == value["recordingOwnerId"]
    {
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
        if envelope.schema_version == crate::current_formats::LEGACY_CAPTURE
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
