//! Meeting deletion is staged: consent, fixed file removal, then row removal.
//! No IPC/Storage deletion entry point is exposed until the file phase exists.
mod database;
pub(in crate::storage) mod fence;
#[cfg(unix)]
mod files;
#[cfg(unix)]
pub(in crate::storage) mod fs;
#[cfg(unix)]
mod intent;
#[cfg(unix)]
mod removal;

#[cfg(all(test, unix))]
mod fence_tests;
#[cfg(all(test, unix))]
mod file_tests;
#[cfg(all(test, unix))]
mod intent_tests;
#[cfg(test)]
mod tests;
