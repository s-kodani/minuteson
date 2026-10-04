//! Whole-track native diarization. Cluster IDs are local to one track/run;
//! they are never treated as voice identities or shared across meetings.
pub(crate) mod sherpa;
pub(crate) mod timeline;
