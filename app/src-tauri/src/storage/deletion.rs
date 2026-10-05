//! Meeting deletion is staged: consent, fixed file removal, then row removal.
//! No IPC/Storage deletion entry point is exposed until the file phase exists.
mod database;
#[cfg(unix)]
mod files;
#[cfg(unix)]
pub(in crate::storage) mod fs;

#[cfg(all(test, unix))]
mod file_tests;
#[cfg(test)]
mod tests;
