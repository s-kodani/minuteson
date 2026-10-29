//! File contents and directory namespaces are distinct guarantees. A verified
//! directory is not a flushed directory. Windows may provide only restart
//! recovery when directory flushing is unsupported by the filesystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamespaceDurability {
    Flushed,
    RestartRecoverable,
}

pub fn directory_sync_result(result: std::io::Result<()>) -> std::io::Result<NamespaceDurability> {
    result.map(|()| NamespaceDurability::Flushed)
}

/// These Win32 errors mean a pinned directory cannot be flushed. Permission
/// denial is accepted only after the caller has independently opened/verified
/// the directory and confirmed that a metadata-capable handle is owned.
pub fn windows_directory_sync_result(
    result: std::io::Result<()>,
) -> std::io::Result<NamespaceDurability> {
    match result {
        Ok(()) => Ok(NamespaceDurability::Flushed),
        Err(error) if matches!(error.raw_os_error(), Some(1 | 5 | 6 | 50)) => {
            Ok(NamespaceDurability::RestartRecoverable)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_namespace_sync_is_never_reported_as_flushed() {
        assert_eq!(
            directory_sync_result(Ok(())).unwrap(),
            NamespaceDurability::Flushed
        );
        assert_eq!(
            windows_directory_sync_result(Err(std::io::Error::from_raw_os_error(5))).unwrap(),
            NamespaceDurability::RestartRecoverable
        );
        assert!(
            windows_directory_sync_result(Err(std::io::Error::from_raw_os_error(112))).is_err()
        );
    }
}
