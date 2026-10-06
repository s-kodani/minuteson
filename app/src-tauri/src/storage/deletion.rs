//! Meeting deletion is staged: consent, fixed file removal, then row removal.
//! User-facing deletion requires shared admission and explicit scope review.
#[cfg(unix)]
pub(crate) mod audio;
#[cfg(unix)]
mod audio_supersession;
#[cfg(unix)]
mod completion;
mod database;
pub(in crate::storage) mod fence;
#[cfg(unix)]
mod files;
#[cfg(unix)]
pub(in crate::storage) mod fs;
#[cfg(unix)]
mod intent;
#[cfg(unix)]
pub(crate) mod models;
#[cfg(unix)]
pub(crate) mod operations;
#[cfg(unix)]
mod removal;
#[cfg(unix)]
mod renewal;
#[cfg(unix)]
pub(in crate::storage) mod startup;

#[cfg(all(test, unix))]
mod fence_tests;
#[cfg(all(test, unix))]
mod file_tests;
#[cfg(all(test, unix))]
mod intent_tests;
#[cfg(test)]
mod tests;
