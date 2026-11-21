//! Exact, app-owned files copied by partial salvage. Reuse the same guarded
//! reservation and removal operations as ordinary file deletion.
use super::fs::{Directory, Identity, NativeProof as _, Proof};
use crate::native_fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub(crate) relative: String,
    parent: Identity,
    pub(crate) proof: Proof,
}

pub(crate) struct Parent {
    directory: Directory,
    path: PathBuf,
}

fn directory<'a>(root: &Path, relative: &'a str) -> Result<(Parent, &'a str), String> {
    if !crate::model_inspection::relative(relative) {
        return Err("元ファイルの相対パスが不正です".into());
    }
    let mut directory = Directory::root(root)?;
    let mut path = root.to_owned();
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return Ok((Parent { directory, path }, part));
        }
        directory = directory.child(part)?.ok_or("元ファイルの親がありません")?;
        path.push(part);
    }
    Err("元ファイル名が不正です".into())
}

pub(crate) fn managed_audio(relative: &str) -> bool {
    let uuid = |text: &str| uuid::Uuid::parse_str(text).is_ok_and(|id| id.to_string() == text);
    let wav = |text: &str| text.strip_suffix(".wav").is_some_and(uuid);
    match relative.split('/').collect::<Vec<_>>().as_slice() {
        ["meetings", meeting, "audio" | "imports", operation, "recording.wav"] => {
            uuid(meeting) && uuid(operation)
        }
        ["meetings", meeting, "audio", "meeting-wave", key, asset] => {
            uuid(meeting)
                && key.len() == 64
                && key
                    .bytes()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
                && wav(asset)
        }
        ["meetings", meeting, "restored", asset] => uuid(meeting) && uuid(asset),
        ["voices", person, sample] => uuid(person) && wav(sample),
        _ => false,
    }
}

impl Target {
    pub(crate) fn capture(root: &Path, relative: &str) -> Result<Option<Self>, String> {
        let (parent, leaf) = directory(root, relative)?;
        Ok(parent.observe(leaf)?.map(|proof| Self {
            relative: relative.into(),
            parent: parent.directory.identity.clone(),
            proof,
        }))
    }

    pub(crate) fn validate(&self, device: u64) -> Result<(), String> {
        self.parent.validate(true)?;
        self.proof.validate(false)?;
        if !crate::model_inspection::relative(&self.relative)
            || self.parent.device != device
            || self.proof.identity.device != device
        {
            return Err("元ファイルの所有記録が不正です".into());
        }
        Ok(())
    }

    pub(crate) fn parent<'a>(&'a self, root: &Path) -> Result<(Parent, &'a str), String> {
        let (parent, leaf) = directory(root, &self.relative)?;
        parent.directory.verify_identity(&self.parent)?;
        Ok((parent, leaf))
    }
}

impl Parent {
    pub(crate) fn observe(&self, leaf: &str) -> Result<Option<Proof>, String> {
        self.directory.verify()?;
        let proof = match native_fs::symlink_metadata(self.path.join(leaf)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
            Ok(meta) => {
                if !meta.is_file()
                    || meta.dev() != self.directory.identity.device
                    || meta.uid() != native_fs::current_owner()?
                {
                    return Err("元ファイルの実体・所有を確認できません".into());
                }
                let proof = Proof::read(&meta)?;
                proof.validate(false)?;
                Some(proof)
            }
        };
        self.directory.verify()?;
        Ok(proof)
    }

    pub(crate) fn reserve(&self, leaf: &str, staged: &str) -> Result<(), String> {
        self.directory.rename_exclusive(leaf, staged)
    }

    pub(crate) fn remove(&self, staged: &str, proof: &Proof) -> Result<(), String> {
        self.directory.unlink_staged(staged, false, proof)
    }

    pub(crate) fn sync(&self) -> Result<(), String> {
        self.directory.sync()
    }
}
