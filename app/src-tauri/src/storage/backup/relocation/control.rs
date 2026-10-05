//! Sealed local recovery history, tied to a held native source snapshot. This is
//! an input to the destination handoff, not permission to activate or clean up.
use super::*;
use std::sync::Arc;

#[derive(Serialize)]
struct Envelope {
    version: u32,
    native_manifest_hash: String,
    history: scratch::transfer::Bundle,
}

#[allow(dead_code)] // Destination handoff and cutover coordinator follow.
pub(crate) struct ControlHandoff<'a, 'storage> {
    snapshot: &'a NativeSnapshot<'storage>,
    registry: Arc<scratch::Registry>,
    path: PathBuf,
    proof: Proof,
    bytes: Vec<u8>,
    hash: String,
}

#[allow(dead_code)]
impl<'storage> NativeSnapshot<'storage> {
    pub(crate) fn prepare_control_handoff<'a>(
        &'a self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<ControlHandoff<'a, 'storage>, String> {
        self.verify(cancelled)?;
        self.verify_source(cancelled)?;
        let registry = self
            .maintenance
            .storage
            .scratch
            .as_ref()
            .ok_or("端末内台帳がありません")?
            .clone();
        let history = registry.handoff(
            self.stage.path(),
            &read_only(&self.image.join("meetings.sqlite3"))?,
            cancelled,
        )?;
        let bytes = serde_json::to_vec(&Envelope {
            version: 1,
            native_manifest_hash: self.manifest_hash.clone(),
            history,
        })
        .map_err(|e| e.to_string())?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("移行用制御記録が安全容量を超えています".into());
        }
        let path = self.stage.path().join("relocation-control.json");
        let mut file = private_file(&path)?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        let proof = Proof::read(&file.metadata().map_err(|e| e.to_string())?)?;
        File::open(self.stage.path())
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        let result = ControlHandoff {
            snapshot: self,
            registry,
            path,
            proof,
            hash: digest(&bytes),
            bytes,
        };
        result.verify(cancelled)?;
        Ok(result)
    }
}
#[allow(dead_code)]
impl ControlHandoff<'_, '_> {
    pub(crate) fn hash(&self) -> &str {
        &self.hash
    }
    pub(crate) fn verify(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.snapshot.verify(cancelled)?;
        self.snapshot.verify_source(cancelled)?;
        if Proof::read(&std::fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?)?
            != self.proof
            || digest_file(&self.path, cancelled)? != (self.bytes.len() as u64, self.hash.clone())
        {
            return Err("移行用制御記録が変更されています".into());
        }
        let history = self.registry.handoff(
            self.snapshot.stage.path(),
            &read_only(&self.snapshot.image.join("meetings.sqlite3"))?,
            cancelled,
        )?;
        let actual = serde_json::to_vec(&Envelope {
            version: 1,
            native_manifest_hash: self.snapshot.manifest_hash.clone(),
            history,
        })
        .map_err(|e| e.to_string())?;
        if actual != self.bytes {
            return Err("元の所有台帳・保持記録が変更されています。移行確認は失効しました".into());
        }
        self.snapshot.verify_source(cancelled)?;
        if Proof::read(&std::fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?)?
            != self.proof
        {
            return Err("元の台帳を再照合する間に移行用制御記録が変更されています".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
