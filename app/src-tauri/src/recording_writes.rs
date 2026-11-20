//! Live evidence of verified, durably advertised recording segments. Final
//! recording completion still requires producer closure and WAV publication.
use crate::recording_segments::{live::LivePrefix, Lane};
use serde::Serialize;
use std::fs::File;
use std::path::Path;
use std::time::{Instant, SystemTime};

const SAFE: u64 = 9_007_199_254_740_991;
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Track {
    state: &'static str,
    frames: Option<u64>,
    age_ms: Option<u64>,
}
impl Track {
    fn unknown(state: &'static str) -> Self {
        Self {
            state,
            frames: None,
            age_ms: None,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    schema_version: u8,
    owner_id: String,
    revision: u64,
    pub(crate) microphone: Track,
    pub(crate) system: Track,
}
fn milliseconds(duration: std::time::Duration) -> Result<u64, String> {
    u64::try_from(duration.as_millis().saturating_add(1))
        .ok()
        .filter(|v| *v <= SAFE)
        .ok_or_else(|| "書込み確認の時刻が範囲外です".into())
}
#[derive(Default)]
struct TrackReader {
    prefix: LivePrefix,
    previous: Option<(u64, u64, Instant)>,
}
impl TrackReader {
    fn read(&mut self, directory: &File, owner: &str, lane: Lane) -> Track {
        let result = (|| {
            let Some((frames, modified)) = self.prefix.read(directory, owner, lane)? else {
                return Ok(Track::unknown("waiting"));
            };
            let mut age = milliseconds(
                SystemTime::now()
                    .duration_since(modified)
                    .map_err(|_| "書込み確認の時計が逆行しました")?,
            )?;
            if let Some((old_frames, old_age, read_at)) = &self.previous {
                if frames == *old_frames {
                    age = age.max(old_age.saturating_add(milliseconds(read_at.elapsed())?));
                }
            }
            self.previous = Some((frames, age, Instant::now()));
            Ok(Track {
                state: "written",
                frames: Some(frames),
                age_ms: Some(age),
            })
        })();
        result.unwrap_or_else(|_: String| Track::unknown("unavailable"))
    }
}
pub(crate) struct WriteReader {
    directory: File,
    owner: Option<String>,
    revision: u64,
    microphone: TrackReader,
    system: TrackReader,
    wav: crate::storage::recordings::wav::Cursor,
}
impl WriteReader {
    pub(crate) fn pin(directory: &Path) -> Result<Self, String> {
        Ok(Self {
            directory: crate::input_levels::open_capture_directory(directory)?,
            owner: None,
            revision: 0,
            microphone: Default::default(),
            system: Default::default(),
            wav: Default::default(),
        })
    }
    pub(crate) fn read(&mut self, directory: &Path, owner: &str) -> Result<Snapshot, String> {
        if !crate::input_levels::valid_owner(owner)
            || self.owner.as_ref().is_some_and(|v| v != owner)
            || directory.canonicalize().map_err(|e| e.to_string())? != directory
            || self.revision >= SAFE
        {
            return Err("書込み確認する録音の対象が変更されました".into());
        }
        self.check_directory(directory)?;
        self.owner = Some(owner.into());
        self.revision += 1;
        let wav = self.wav.read(directory, owner, &|| false);
        let shared = if let Ok(Some(_)) = &wav {
            Some(std::array::from_fn::<_, 2, _>(|index| {
                let frames = self.wav.coverage[index];
                if frames == 0 {
                    return Track::unknown("waiting");
                }
                let age = self.wav.coverage_modified[index]
                    .and_then(|at| SystemTime::now().duration_since(at).ok())
                    .and_then(|d| milliseconds(d).ok());
                match age {
                    Some(age) => Track {
                        state: "written",
                        frames: Some(frames),
                        age_ms: Some(age),
                    },
                    None => Track::unknown("unavailable"),
                }
            }))
        } else {
            None
        };
        let result = Snapshot {
            schema_version: 1,
            owner_id: owner.into(),
            revision: self.revision,
            microphone: shared.as_ref().map(|t| t[0].clone()).unwrap_or_else(|| {
                if wav.is_err() {
                    Track::unknown("unavailable")
                } else {
                    self.microphone
                        .read(&self.directory, owner, Lane::Microphone)
                }
            }),
            system: shared.as_ref().map(|t| t[1].clone()).unwrap_or_else(|| {
                if wav.is_err() {
                    Track::unknown("unavailable")
                } else {
                    self.system.read(&self.directory, owner, Lane::System)
                }
            }),
        };
        self.check_directory(directory)?;
        Ok(result)
    }
    fn check_directory(&self, path: &Path) -> Result<(), String> {
        let held = crate::native_fs::metadata(&self.directory).map_err(|e| e.to_string())?;
        let current =
            crate::native_fs::metadata(&crate::input_levels::open_capture_directory(path)?)
                .map_err(|e| e.to_string())?;
        #[cfg(any(unix, windows))]
        {
            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt;
            if held.dev() != current.dev()
                || held.ino() != current.ino()
                || held.created().map_err(|e| e.to_string())?
                    != current.created().map_err(|e| e.to_string())?
            {
                return Err("書込み確認先の実体が変更されました".into());
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
