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
    pub(crate) fn max_bytes(self) -> u64 {
        match self {
            Self::TranscriptionResult | Self::DiarizationResult => 8 * 1024 * 1024,
            _ => 2 * 1024 * 1024,
        }
    }
    pub(crate) fn media(self) -> &'static str {
        match self {
            Self::TemporaryPcm => "audio/wav",
            Self::DiarizationLog | Self::GenerationSystem | Self::GenerationPrompt => "text/plain",
            Self::GenerationRaw => "application/vnd.mon.llama-completion",
            _ => "application/json",
        }
    }
}

// Construct only private restore artifacts; archive/SQL ownership and physical
// byte proofs remain the coordinator's responsibility. Implementations must
// never overwrite conflicting historical output or make models available.
pub(crate) trait RestoreAssets {
    fn read(&mut self, role: Role, id: &str, hash: &str) -> Result<Vec<u8>, String>;
    fn write(
        &mut self,
        role: Role,
        id: &str,
        hash: &str,
        bytes: &[u8],
    ) -> Result<(String, String), String>;
    fn map(&mut self, domain: &str, id: &str) -> Result<String, String>;
}
