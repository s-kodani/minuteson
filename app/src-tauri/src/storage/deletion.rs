//! Meeting deletion is staged: consent, fixed file removal, then row removal.
//! User-facing deletion requires shared admission and explicit scope review.
#[cfg(any(unix, windows))]
pub(crate) mod audio;
#[cfg(any(unix, windows))]
mod audio_supersession;
#[cfg(any(unix, windows))]
pub(crate) mod automatic;
#[cfg(any(unix, windows))]
mod completion;
pub(in crate::storage) mod database;
#[cfg(any(unix, windows))]
mod directory_scope;

// Read-only native-image inspection; not a deletion or cutover capability.
#[cfg(any(target_os = "macos", windows))]
pub(in crate::storage) fn verify_known_history_schema(
    db: &rusqlite::Connection,
) -> Result<(), String> {
    database::trusted_history_schema(db).map(|_| ())
}
pub(in crate::storage) mod fence;
#[cfg(any(unix, windows))]
mod files;
#[cfg(unix)]
pub(in crate::storage) mod fs;
#[cfg(windows)]
#[path = "deletion/fs_windows.rs"]
pub(in crate::storage) mod fs;
#[cfg(any(unix, windows))]
mod intent;
#[cfg(any(unix, windows))]
pub(crate) mod models;
#[cfg(any(unix, windows))]
pub(crate) mod operations;
#[cfg(any(target_os = "macos", windows))]
pub(crate) mod relocation;
#[cfg(any(unix, windows))]
mod removal;
#[cfg(any(unix, windows))]
mod renewal;
#[cfg(any(unix, windows))]
pub(in crate::storage) mod startup;
#[cfg(any(unix, windows))]
pub(crate) mod verification;

#[cfg(all(test, unix))]
mod fence_tests;
#[cfg(all(test, unix))]
mod file_tests;
#[cfg(all(test, unix))]
mod intent_tests;
#[cfg(test)]
mod tests;

/// Reset workspaces use the same handle-bound, no-link traversal as deletion.
#[cfg(any(target_os = "macos", windows))]
pub(crate) fn remove_reset_workspace(
    path: &std::path::Path,
    expected: (u64, u64, u64),
) -> Result<(), String> {
    let leaf = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or("初期化作業領域の名前が不正です")?;
    if !leaf.starts_with(".mon-delete-reset-") {
        return Err("初期化作業領域ではありません".into());
    }
    let parent = fs::Directory::root(path.parent().ok_or("作業領域の親がありません")?)?;
    let child = parent.child(leaf)?.ok_or("初期化作業領域がありません")?;
    let identity = child.proof()?.identity;
    if identity.registered()? != expected {
        return Err("初期化作業領域が差し替えられています".into());
    }
    drop(child);
    parent.remove_reserved_tree(leaf, &identity, &|| false)
}
