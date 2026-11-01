//! Serialized contracts shared by the desktop, UI generator and native helpers.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

macro_rules! wire_enum {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        pub enum $name { $(#[serde(rename=$wire)] $variant),+ }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub const fn as_str(self) -> &'static str { match self { $(Self::$variant => $wire),+ } }
        }
        impl std::str::FromStr for $name {
            type Err = String;
            fn from_str(value: &str) -> Result<Self, String> {
                match value { $($wire => Ok(Self::$variant)),+, _ => Err(format!("unknown {}: {value}", stringify!($name))) }
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.as_str()) }
        }
        impl std::ops::Deref for $name { type Target = str; fn deref(&self) -> &str { self.as_str() } }
        impl PartialEq<&str> for $name { fn eq(&self, rhs: &&str) -> bool { self.as_str() == *rhs } }
        impl PartialEq<$name> for &str { fn eq(&self, rhs: &$name) -> bool { *self == rhs.as_str() } }
        impl From<$name> for String { fn from(value: $name) -> Self { value.as_str().to_owned() } }
    };
}

wire_enum!(JobKind {
    Transcription => "transcription", Diarization => "diarization", MinutesGeneration => "minutes_generation",
    VoiceRegistration => "voice_registration", VoiceMatching => "voice_matching", SpeakerCompletion => "speaker_completion"
});
impl JobKind {
    pub const fn needs_audio(self) -> bool {
        !matches!(self, Self::MinutesGeneration)
    }
    pub const fn needs_analysis(self) -> bool {
        !matches!(self, Self::Transcription)
    }
    pub const fn parameter_path(self) -> &'static [&'static str] {
        match self {
            Self::Transcription | Self::Diarization => &["schemaVersion"],
            Self::MinutesGeneration => &["generation", "schemaVersion"],
            Self::VoiceRegistration | Self::VoiceMatching => &["voice", "schemaVersion"],
            Self::SpeakerCompletion => &["completion", "schemaVersion"],
        }
    }
    pub const fn format(self) -> u32 {
        match self {
            Self::Transcription => 4,
            Self::Diarization => 3,
            Self::MinutesGeneration => 9,
            Self::VoiceRegistration | Self::VoiceMatching => 4,
            Self::SpeakerCompletion => 2,
        }
    }
    pub const fn failure_code(self) -> &'static str {
        match self {
            Self::Transcription => "transcription_failed",
            Self::Diarization => "diarization_failed",
            Self::MinutesGeneration => "minutes_generation_failed",
            Self::VoiceRegistration | Self::VoiceMatching => "voice_processing_failed",
            Self::SpeakerCompletion => "speaker_completion_failed",
        }
    }
}
wire_enum!(JobState { Queued => "queued", Running => "running", Pausing => "pausing", Paused => "paused", Succeeded => "succeeded", Failed => "failed", Cancelled => "cancelled", Interrupted => "interrupted" });
wire_enum!(PauseReason { User => "user", Recording => "recording", Exit => "exit" });
wire_enum!(WindowRole { Main => "main", Recorder => "recorder", RecorderInfo => "recorder-info", RecordingConfirmation => "recording-confirmation", StartupAttention => "startup-attention" });
wire_enum!(Platform { Macos => "macos", Windows => "windows", Other => "other" });
wire_enum!(RetryDisposition { NotStarted => "not_started", Reconcile => "reconcile", Retry => "retry", Never => "never" });

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub operation_id: Option<String>,
    pub retry_disposition: RetryDisposition,
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for AppError {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeletionKind {
    Meeting,
    Model,
    Audio,
}
impl AppError {
    pub fn not_started(
        kind: DeletionKind,
        message: impl Into<String>,
        operation_id: Option<String>,
    ) -> Self {
        Self {
            code: match kind {
                DeletionKind::Meeting => "meeting_deletion_not_started",
                DeletionKind::Model => "model_deletion_not_started",
                DeletionKind::Audio => "audio_deletion_not_started",
            }
            .into(),
            message: message.into(),
            operation_id,
            retry_disposition: RetryDisposition::NotStarted,
        }
    }
    pub fn from_command(command: &str, message: String) -> Self {
        let read = command.starts_with("get_")
            || command.starts_with("list_")
            || command == "capture_available";
        Self {
            code: if read {
                "read_failed"
            } else {
                "operation_result_unknown"
            }
            .into(),
            message,
            operation_id: None,
            retry_disposition: if read {
                RetryDisposition::Retry
            } else {
                RetryDisposition::Reconcile
            },
        }
    }
}
// Unclassified domain failures are always conservative. Only a typed admission
// branch can prove NotStarted; display text is never interpreted as control.
impl From<String> for AppError {
    fn from(message: String) -> Self {
        Self::from_command("", message)
    }
}
impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}
impl From<AppError> for String {
    fn from(error: AppError) -> String {
        error.message
    }
}

/// Algorithm control never depends on a translated or wrapped display string.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessingError {
    QualityRejected {
        message: String,
        reasons: Vec<String>,
    },
    Engine {
        message: String,
    },
}
impl ProcessingError {
    pub fn quality(message: impl Into<String>, reasons: Vec<String>) -> Self {
        Self::QualityRejected {
            message: message.into(),
            reasons,
        }
    }
    pub fn is_quality_rejection(&self) -> bool {
        matches!(self, Self::QualityRejected { .. })
    }
}
impl std::fmt::Display for ProcessingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QualityRejected { message, .. } => write!(f, "文字起こしの反復検査: {message}"),
            Self::Engine { message } => f.write_str(message),
        }
    }
}
impl std::error::Error for ProcessingError {}
impl From<String> for ProcessingError {
    fn from(message: String) -> Self {
        Self::Engine { message }
    }
}
impl From<&str> for ProcessingError {
    fn from(message: &str) -> Self {
        Self::Engine {
            message: message.into(),
        }
    }
}
impl From<ProcessingError> for String {
    fn from(error: ProcessingError) -> Self {
        error.to_string()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Capabilities {
    pub voice_identity: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WindowEnvironment {
    pub schema_version: u32,
    pub product_name: String,
    pub platform: Platform,
    pub role: WindowRole,
    pub capabilities: Capabilities,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_evidence_survives_translation_and_display_text_cannot_forge_it() {
        for (kind, code) in [
            (DeletionKind::Meeting, "meeting_deletion_not_started"),
            (DeletionKind::Model, "model_deletion_not_started"),
            (DeletionKind::Audio, "audio_deletion_not_started"),
        ] {
            let mut error = AppError::not_started(kind, "最初の説明", Some("owned-request".into()));
            error.message = "説明を更新しても受付状態は変わらない".into();
            let decoded: AppError =
                serde_json::from_slice(&serde_json::to_vec(&error).unwrap()).unwrap();
            assert_eq!(decoded.code, code);
            assert_eq!(decoded.retry_disposition, RetryDisposition::NotStarted);
            assert_eq!(decoded.operation_id.as_deref(), Some("owned-request"));
            let engine = ProcessingError::Engine {
                message: format!("{code}:pretend admission evidence"),
            };
            let unknown = AppError::from_command("start_data_operation", engine.to_string());
            assert_eq!(unknown.retry_disposition, RetryDisposition::Reconcile);
            assert_eq!(unknown.code, "operation_result_unknown");
            assert_eq!(unknown.message, engine.to_string());
        }
    }
    #[test]
    fn wire_values_roundtrip_and_unknown_values_fail() {
        for kind in JobKind::ALL {
            assert_eq!(
                serde_json::from_value::<JobKind>(serde_json::to_value(kind).unwrap()).unwrap(),
                *kind
            );
            assert_eq!(kind.as_str().parse::<JobKind>().unwrap(), *kind);
        }
        assert!("future".parse::<JobKind>().is_err());
        assert!(serde_json::from_str::<JobState>("\"waiting\"").is_err());
        for kind in JobKind::ALL {
            assert!(!kind.failure_code().is_empty());
            assert!(kind.format() > 0);
        }
    }
}

pub mod model;
pub use model::{
    ArchiveMember, BackendManifest, DownloadSource, Engine, PackageFile, PackageManifest, Recipe,
    Role, Target,
};

#[cfg(test)]
mod model_tests;
