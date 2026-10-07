//! Meeting deletion is staged: consent, fixed file removal, then row removal.
//! User-facing deletion requires shared admission and explicit scope review.
#[cfg(unix)]
pub(crate) mod audio;
#[cfg(unix)]
mod audio_supersession;
#[cfg(unix)]
mod completion;
mod database;

// Read-only native-image inspection; not a deletion or cutover capability.
#[cfg(target_os = "macos")]
pub(in crate::storage) fn verify_known_history_schema(
    db: &rusqlite::Connection,
) -> Result<(), String> {
    database::trusted_history_schema(db).map(|_| ())
}
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
#[cfg(target_os = "macos")]
pub(crate) mod relocation;
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
