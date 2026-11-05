//! The only writable/readable application contracts. No implicit upgrades.
use serde_json::Value;
pub(crate) const TRANSCRIPTION: u32 = mon_contracts::JobKind::Transcription.format();
pub(crate) const DIARIZATION: u32 = mon_contracts::JobKind::Diarization.format();
pub(crate) const MINUTES: u32 = mon_contracts::JobKind::MinutesGeneration.format();
pub(crate) const VOICE: u32 = mon_contracts::JobKind::VoiceMatching.format();
pub(crate) const COMPLETION: u32 = mon_contracts::JobKind::SpeakerCompletion.format();
pub(crate) const CAPTURE: u32 = 4;
pub(crate) const TRANSCRIPT_DRAFT: u32 = 4;
pub(crate) const BACKUP: u32 = 8;
pub(crate) const OPTIMIZATION: u32 = 1;

pub(crate) fn job(kind: &str, parameters: &Value) -> Result<(), String> {
    let kind = kind
        .parse::<mon_contracts::JobKind>()
        .map_err(|_| "非対応のジョブ種別です".to_string())?;
    let expected = kind.format();
    let value = kind
        .parameter_path()
        .iter()
        .fold(parameters, |value, key| &value[*key]);
    if value.as_u64() != Some(u64::from(expected)) {
        return Err(format!(
            "非対応の{kind}ジョブ形式です。最新形式で新しく実行してください"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_current_job_versions_are_accepted() {
        for (kind, current) in [
            ("transcription", TRANSCRIPTION),
            ("diarization", DIARIZATION),
            ("minutes_generation", MINUTES),
            ("voice_registration", VOICE),
            ("voice_matching", VOICE),
            ("speaker_completion", COMPLETION),
        ] {
            for version in 0..=current + 1 {
                let parameters = match kind {
                    "minutes_generation" => json!({"generation":{"schemaVersion":version}}),
                    "voice_registration" | "voice_matching" => {
                        json!({"voice":{"schemaVersion":version}})
                    }
                    "speaker_completion" => json!({"completion":{"schemaVersion":version}}),
                    _ => json!({"schemaVersion":version}),
                };
                assert_eq!(
                    job(kind, &parameters).is_ok(),
                    version == current,
                    "{kind} {version}"
                );
            }
            assert!(job(kind, &json!({})).is_err());
        }
    }
    #[test]
    fn fresh_database_and_current_draft_are_canonical() {
        let temp = tempfile::tempdir().unwrap();
        let storage = crate::storage::Storage::open(temp.path().to_owned()).unwrap();
        let draft = crate::storage::TranscriptDraftPayload::default();
        let value = serde_json::to_value(draft).unwrap();
        assert_eq!(value["schema_version"], TRANSCRIPT_DRAFT);
        for key in ["manual_speakers", "speaker_merges", "utterance_splits"] {
            assert_eq!(value[key], json!([]));
        }
        crate::storage::with_test_database(&storage, |db| {
            assert_eq!(
                db.query_row("SELECT max(version) FROM schema_migrations", [], |r| r
                    .get::<_, u32>(0))
                    .unwrap(),
                crate::storage::SCHEMA_VERSION as u32
            );
            assert!(!db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name IN ('edit_operations','transcript_edit_recovery','audio_upgrade_progress'))",
                [], |row| row.get::<_, bool>(0),
            ).unwrap());
        });
    }
}

pub(crate) fn optional<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(d)
}
