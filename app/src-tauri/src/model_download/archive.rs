//! Shared bounded TAR/gzip/bzip2 reader. Only this adapter opens pinned,
//! create-new destination handles; archive metadata never grants filesystem IO.
use super::{check, Recipe, Source};
use crate::model_placement::fs::Directory;
use mon_native::archive::{Member, Policy, Target};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

struct Destination {
    file: File,
    parent: Directory,
}
impl Write for Destination {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.parent.verify().map_err(std::io::Error::other)?;
        self.file.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
impl Target for Destination {
    fn verify(&self) -> Result<(), String> {
        self.parent.verify()
    }
    fn finish(self: Box<Self>) -> Result<(), String> {
        self.file
            .sync_all()
            .map_err(|_| "展開したファイルを確定できません")?;
        self.parent.sync()
    }
}

pub(super) fn extract(
    directory: &Directory,
    relative: &str,
    payload: &Directory,
    recipe: &Recipe,
    indices: &[usize],
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    check(cancelled)?;
    let started = std::time::Instant::now();
    let mut policy = Policy::default();
    let first = &recipe.sources[*indices.first().ok_or("展開対象がありません")?];
    let archive = first.archive.as_ref().ok_or("アーカイブ指定がありません")?;
    let mut source = Source::open(directory.path(), relative)?;
    let mut digest = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 65_536];
    loop {
        check(cancelled)?;
        if started.elapsed() >= policy.deadline {
            return Err("アーカイブ読取の時間上限を超えました".into());
        }
        let n = source
            .file
            .read(&mut buffer)
            .map_err(|_| "アーカイブを再照合できません")?;
        if n == 0 {
            break;
        }
        size = size
            .checked_add(n as u64)
            .filter(|n| *n <= archive.expected_size)
            .ok_or("アーカイブ容量が一致しません")?;
        digest.update(&buffer[..n]);
    }
    if size != archive.expected_size || format!("{:x}", digest.finalize()) != archive.expected_hash
    {
        return Err("アーカイブの容量・SHAが一致しません".into());
    }
    source.verify()?;
    source
        .file
        .seek(SeekFrom::Start(0))
        .map_err(|_| "アーカイブを読み取れません")?;
    let input = source
        .file
        .try_clone()
        .map_err(|_| "アーカイブの読取ハンドルを保持できません")?;
    let members = indices
        .iter()
        .map(|index| {
            let file = &recipe.manifest.files[*index];
            Ok(Member {
                path: &recipe.sources[*index]
                    .archive
                    .as_ref()
                    .ok_or("アーカイブ指定がありません")?
                    .member_path,
                size: file.expected_size.ok_or("展開容量がありません")?,
                sha256: file.expected_hash.as_deref().ok_or("展開SHAがありません")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    policy.deadline = policy.deadline.saturating_sub(started.elapsed());
    mon_native::archive::extract(
        input,
        &members,
        cancelled,
        &mut || {
            source.verify()?;
            directory.verify()?;
            payload.verify()
        },
        &mut |index| {
            let (file, parent) =
                payload.create(&recipe.manifest.files[indices[index]].relative_path)?;
            Ok(Box::new(Destination { file, parent }))
        },
        policy,
    )?;
    source.verify()?;
    directory.verify()?;
    payload.sync()?;
    Ok(())
}
