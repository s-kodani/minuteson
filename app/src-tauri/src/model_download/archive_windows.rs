//! Stream ONLY named members to create-new handles. System bsdtar never writes
//! paths, permissions or links to disk. Both archive and each member are pinned.
use super::{Recipe, Source};
use crate::{local_engine::OwnedCommand, model_placement::fs::Directory};
use sha2::{Digest, Sha256};
use std::io::Read;
pub(super) fn extract(
    directory: &Directory,
    relative: &str,
    payload: &Directory,
    recipe: &Recipe,
    indices: &[usize],
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let first = &recipe.sources[*indices.first().ok_or("展開対象がありません")?];
    let archive = first.archive.as_ref().ok_or("アーカイブ指定がありません")?;
    let mut source = Source::open(directory.path(), relative)?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        super::check(cancelled)?;
        let n = source.file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size = size
            .checked_add(n as u64)
            .filter(|n| *n <= archive.expected_size)
            .ok_or("アーカイブ容量が範囲外です")?;
        hash.update(&buffer[..n]);
    }
    if size != archive.expected_size || format!("{:x}", hash.finalize()) != archive.expected_hash {
        return Err("アーカイブの容量・SHAが一致しません".into());
    }
    for &index in indices {
        super::check(cancelled)?;
        source.verify()?;
        payload.verify()?;
        let file = &recipe.manifest.files[index];
        let member = &recipe.sources[index].archive.as_ref().unwrap().member_path;
        if !crate::model_inspection::relative(member) {
            return Err("アーカイブ内のパスが不正です".into());
        }
        let mut launch = OwnedCommand::new(&super::transport::system_tool("tar.exe")?)?;
        launch
            .command
            .env_clear()
            .env(
                "SystemRoot",
                std::env::var_os("SystemRoot").unwrap_or_default(),
            )
            .arg("-xOf")
            .arg(directory.path().join(relative))
            .arg("--")
            .arg(member);
        let (mut output, parent) = payload.create(&file.relative_path)?;
        super::transport::stream(
            launch,
            file.expected_size.ok_or("展開容量がありません")?,
            file.expected_hash.as_deref().ok_or("展開SHAがありません")?,
            &mut output,
            cancelled,
            &mut |_| {},
            std::time::Duration::from_secs(600),
        )?;
        parent.sync()?;
        source.verify()?;
    }
    Ok(())
}
