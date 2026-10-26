//! Bounded reading of owned native result files.
use std::path::Path;

pub(crate) fn read_local(
    path: &Path,
    limit: u64,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let metadata = crate::native_fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file()
        || metadata.len() > limit
        || std::fs::canonicalize(path).map_err(|e| e.to_string())? != path
    {
        return Err("処理出力の実体・容量が不正です".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(not(windows))]
    let file = options.open(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let file = crate::windows_files::read(path, false)?;
    let before = crate::native_fs::metadata(&file).map_err(|e| e.to_string())?;
    let same =
        |a: &crate::native_fs::Metadata, b: &crate::native_fs::Metadata| -> Result<bool, String> {
            if !b.file_type().is_file()
                || a.len() != b.len()
                || a.modified().map_err(|e| e.to_string())?
                    != b.modified().map_err(|e| e.to_string())?
            {
                return Ok(false);
            }
            #[cfg(any(unix, windows))]
            {
                #[cfg(unix)]
                use std::os::unix::fs::MetadataExt;
                if a.dev() != b.dev() || a.ino() != b.ino() {
                    return Ok(false);
                }
            }
            Ok(true)
        };
    if !same(&metadata, &before)? {
        return Err("処理出力が差し替えられました".into());
    }
    let mut bytes = Vec::new();
    let mut reader = (&file).take(limit.checked_add(1).ok_or("出力の上限が不正です")?);
    let mut block = [0u8; 65_536];
    loop {
        if cancelled() {
            return Err("処理出力の読込を取り消しました".into());
        }
        let count = reader.read(&mut block).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&block[..count]);
    }
    if bytes.len() as u64 != before.len()
        || bytes.len() as u64 > limit
        || !same(
            &before,
            &crate::native_fs::metadata(&file).map_err(|e| e.to_string())?,
        )?
        || !same(
            &before,
            &crate::native_fs::symlink_metadata(path).map_err(|e| e.to_string())?,
        )?
    {
        return Err("処理出力が読込中に変更されています".into());
    }
    Ok(bytes)
}
pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_rebinding_local_output_read_is_bounded_cancellable_and_writes_never_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap().join("result.json");
        write_new(&path, b"{\"ok\":true}").unwrap();
        assert_eq!(
            read_local(&path, 64, &crate::test_support::never_cancelled).unwrap(),
            b"{\"ok\":true}"
        );
        assert!(read_local(&path, 2, &crate::test_support::never_cancelled).is_err());
        assert!(read_local(&path, 64, &|| true).is_err());
        assert!(write_new(&path, b"changed").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"ok\":true}");
        #[cfg(unix)]
        {
            let link = path.parent().unwrap().join("link.json");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(read_local(&link, 64, &crate::test_support::never_cancelled).is_err());
        }
    }
}
