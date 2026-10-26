//! Small, disposable range descriptors; never another copy of meeting PCM.
use crate::local_engine::Fingerprint;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

const NAME: &str = "input-source.json";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Input {
    pub proof: Fingerprint,
    pub first_frame: u64,
    pub end_frame: u64,
}
impl Input {
    pub fn new(
        path: &Path,
        first_frame: u64,
        end_frame: u64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let input = Self {
            proof: Fingerprint::capture_retained(path, cancelled)?,
            first_frame,
            end_frame,
        };
        input.verify(cancelled)?;
        Ok(input)
    }
    pub fn load(directory: &Path, cancelled: &impl Fn() -> bool) -> Result<Self, String> {
        let path = std::fs::canonicalize(directory)
            .map_err(|e| e.to_string())?
            .join(NAME);
        let input = if path.exists() {
            let bytes = crate::restored_processing::read_local(&path, 16384, cancelled)?;
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?
        } else {
            // Standalone synthetic runtime probes also use this reader.
            let proof = Fingerprint::capture_retained(&directory.join("input.wav"), cancelled)?;
            let mut file = std::fs::File::open(&proof.path).map_err(|e| e.to_string())?;
            let layout = crate::audio::wave::inspect(&mut file, proof.size)?;
            Self {
                proof,
                first_frame: 0,
                end_frame: layout.frames,
            }
        };
        input.verify(cancelled)?;
        Ok(input)
    }
    pub fn verify(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.proof.verify(cancelled)?;
        let mut file = std::fs::File::open(&self.proof.path).map_err(|e| e.to_string())?;
        let layout = crate::audio::wave::inspect(&mut file, self.proof.size)?;
        if self.end_frame <= self.first_frame || self.end_frame > layout.frames {
            return Err("解析入力のフレーム範囲が不正です".into());
        }
        Ok(())
    }
    pub fn frames(&self) -> u64 {
        self.end_frame - self.first_frame
    }
    pub fn write(&self, directory: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(NAME))
            .map_err(|e| e.to_string())?;
        f.write_all(&bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())
    }
    pub fn slice(&self, first: u64, end: u64) -> Result<Self, String> {
        if end <= first || end > self.frames() {
            return Err("再認識区間が入力範囲外です".into());
        }
        Ok(Self {
            proof: self.proof.clone(),
            first_frame: self.first_frame + first,
            end_frame: self.first_frame + end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("meeting.wav");
        let mut bytes = crate::audio::wave::header(1600).unwrap();
        bytes.extend(vec![0; 3200]);
        std::fs::write(&path, bytes).unwrap();
        (temp, path)
    }
    #[test]
    fn descriptors_and_subranges_reuse_the_same_pcm_without_creating_audio() {
        let (temp, path) = fixture();
        let input = Input::new(&path, 100, 1500, &|| false).unwrap();
        let unit = temp.path().join("unit");
        std::fs::create_dir(&unit).unwrap();
        input.write(&unit).unwrap();
        assert_eq!(Input::load(&unit, &|| false).unwrap(), input);
        let child = input.slice(200, 400).unwrap();
        assert_eq!((child.first_frame, child.end_frame), (300, 500));
        assert_eq!(child.proof, input.proof);
        assert!(input.slice(0, 1401).is_err());
        assert_eq!(std::fs::read_dir(&unit).unwrap().count(), 1);
        assert!(unit.join(NAME).is_file());
        assert!(!unit.join("input.wav").exists());
    }
    #[test]
    fn out_of_bounds_ranges_and_changed_pcm_are_refused() {
        let (_temp, path) = fixture();
        assert!(Input::new(&path, 0, 1601, &|| false).is_err());
        let input = Input::new(&path, 0, 1600, &|| false).unwrap();
        crate::local_engine::verification::release(&input.proof.path).unwrap();
        std::fs::write(&path, b"changed").unwrap();
        assert!(input.verify(&|| false).is_err());
    }
}
