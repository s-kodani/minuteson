//! Roles come from the worker's typed protocol, never from arbitrary JSON keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    TemporaryPcm,
    TranscriptionResult,
    DiarizationLog,
    DiarizationResult,
    GenerationSystem,
    GenerationPrompt,
    GenerationSchema,
    GenerationRaw,
    GenerationItems,
}

impl Role {
    pub(crate) fn media(self) -> &'static str {
        match self {
            Self::TemporaryPcm => "audio/wav",
            Self::DiarizationLog | Self::GenerationSystem | Self::GenerationPrompt => "text/plain",
            Self::GenerationRaw => "application/vnd.mon.llama-completion",
            _ => "application/json",
        }
    }
}
