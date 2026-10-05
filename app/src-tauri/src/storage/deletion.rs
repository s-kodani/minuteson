//! Meeting deletion is staged: consent, fixed file removal, then row removal.
//! No IPC/Storage deletion entry point is exposed until the file phase exists.
mod database;

#[cfg(test)]
mod tests;
