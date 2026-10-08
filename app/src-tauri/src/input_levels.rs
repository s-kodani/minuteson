//! Read-only live telemetry. Neither connection nor telemetry proves saved audio.
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::{Instant, SystemTime};

const LIMIT: u64 = 16 * 1024;
const SAFE: u64 = 9_007_199_254_740_991;

#[cfg(unix)]
pub(crate) fn open_capture_directory(directory: &Path) -> Result<File, String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Component;
    if !directory.is_absolute() {
        return Err("入力確認先は絶対パスが必要です".into());
    }
    let mut parent = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")
        .map_err(|e| e.to_string())?;
    for component in directory.components() {
        let Component::Normal(name) = component else {
            if component == Component::RootDir {
                continue;
            }
            return Err("入力確認先に不正なパス要素があります".into());
        };
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|e| e.to_string())?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        parent = unsafe { File::from_raw_fd(fd) };
    }
    Ok(parent)
}
#[cfg(unix)]
pub(crate) fn open_capture_file_at(parent: &File, name: &str) -> Result<File, String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    if !matches!(
        name,
        "input-levels.json" | "input-status.json" | "microphone.caf" | "system.caf"
    ) {
        return Err("録音確認のファイル名が不正です".into());
    }
    let name = std::ffi::CString::new(name).unwrap();
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
#[cfg(not(unix))]
pub(crate) fn open_capture_directory(_directory: &Path) -> Result<File, String> {
    Err("このOSの入力レベル確認は未対応です".into())
}
#[cfg(not(unix))]
pub(crate) fn open_capture_file_at(_parent: &File, _name: &str) -> Result<File, String> {
    Err("このOSの入力レベル確認は未対応です".into())
}
fn open_capture_file(directory: &Path, name: &str) -> Result<File, String> {
    open_capture_file_at(&open_capture_directory(directory)?, name)
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Track {
    state: String,
    window_frames: Option<u32>,
    age_ms: Option<u64>,
    rms: Option<f64>,
    peak: Option<f64>,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Publication {
    schema_version: u32,
    owner_id: String,
    revision: u64,
    microphone: Track,
    system: Track,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InputLevels {
    schema_version: u32,
    owner_id: String,
    revision: u64,
    report_age_ms: u64,
    microphone: Track,
    system: Track,
}

pub(crate) fn valid_owner(owner: &str) -> bool {
    uuid::Uuid::parse_str(owner).is_ok_and(|id| {
        id.get_version_num() == 4
            && id.get_variant() == uuid::Variant::RFC4122
            && id.to_string() == owner
    })
}
fn parse(bytes: &[u8], owner: &str) -> Result<Publication, String> {
    if bytes.is_empty() || bytes.len() as u64 > LIMIT || !valid_owner(owner) {
        return Err("入力レベルの応答または対象が不正です".into());
    }
    let fields: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    for name in ["microphone", "system"] {
        let keys = ["state", "windowFrames", "ageMs", "rms", "peak"];
        let track = fields[name]
            .as_object()
            .ok_or("入力レベルの形式が不正です")?;
        if track.len() != keys.len() || keys.iter().any(|key| !track.contains_key(*key)) {
            return Err("入力レベルに必須項目がありません".into());
        }
    }
    let report: Publication = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if report.schema_version != 1
        || report.owner_id != owner
        || !(1..=SAFE).contains(&report.revision)
    {
        return Err("入力レベルの対象または版が一致しません".into());
    }
    for track in [&report.microphone, &report.system] {
        let valid = match track.state.as_str() {
            "waiting" | "unavailable" => {
                track.window_frames.is_none()
                    && track.age_ms.is_none()
                    && track.rms.is_none()
                    && track.peak.is_none()
            }
            "measured" => {
                matches!(track.window_frames, Some(1..=1_000_000))
                    && track.age_ms.is_some_and(|v| v <= SAFE)
                    && matches!((track.rms, track.peak), (Some(rms), Some(peak))
                        if rms.is_finite() && peak.is_finite() && rms >= 0.0
                        && peak >= rms && peak <= 16.0)
            }
            _ => false,
        };
        if !valid {
            return Err("入力レベルの測定値が不正です".into());
        }
    }
    Ok(report)
}

#[derive(Default)]
pub(crate) struct LevelReader {
    previous: Option<(Publication, u64, Instant)>,
}
impl LevelReader {
    pub(crate) fn read(&mut self, directory: &Path, owner: &str) -> Result<InputLevels, String> {
        // directory is the exact canonical output pinned during child admission.
        if directory.canonicalize().map_err(|e| e.to_string())? != directory {
            return Err("録音の入力確認先が変更されました".into());
        }
        // Traverse through directory descriptors, not path-following opens: a
        // parent swapped to a symlink between checks cannot redirect this read.
        let file = open_capture_file(directory, "input-levels.json")?;
        self.read_file(file, owner)
    }
    // A private capture test already owns a pinned directory descriptor. Use
    // its descriptor-relative open instead of resolving that path a second time.
    pub(crate) fn read_file(&mut self, file: File, owner: &str) -> Result<InputLevels, String> {
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > LIMIT {
            return Err("入力レベルのファイル形式または容量が不正です".into());
        }
        let modified = metadata.modified().map_err(|e| e.to_string())?;
        let started = Instant::now();
        let age = SystemTime::now()
            .duration_since(modified)
            .map_err(|_| "入力レベルの記録時刻が逆行しました")?;
        let mut bytes = Vec::new();
        (&file)
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        if metadata.len() != after.len()
            || modified != after.modified().map_err(|e| e.to_string())?
        {
            return Err("入力レベルが読込み中に変更されました".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1
                || metadata.dev() != after.dev()
                || metadata.ino() != after.ino()
            {
                return Err("入力レベルの実体が不正です".into());
            }
        }
        let publication = parse(&bytes, owner)?;
        let millis = |duration: std::time::Duration| -> Result<u64, String> {
            u64::try_from(duration.as_millis().saturating_add(1))
                .ok()
                .filter(|v| *v <= SAFE)
                .ok_or_else(|| "入力レベルの時刻が範囲外です".into())
        };
        let mut report_age_ms = millis(age)?.saturating_add(millis(started.elapsed())?);
        if let Some((previous, first_age, first_read)) = &self.previous {
            if publication.owner_id != previous.owner_id || publication.revision < previous.revision
            {
                return Err("入力レベルの対象または版が逆行しました".into());
            }
            if publication.revision == previous.revision {
                if &publication != previous {
                    return Err("同じ入力レベルの版に異なる測定値があります".into());
                }
                report_age_ms =
                    report_age_ms.max(first_age.saturating_add(millis(first_read.elapsed())?));
            } else {
                self.previous = Some((publication.clone(), report_age_ms, Instant::now()));
            }
        } else {
            self.previous = Some((publication.clone(), report_age_ms, Instant::now()));
        }
        if report_age_ms > SAFE {
            return Err("入力レベルの時刻が範囲外です".into());
        }
        Ok(InputLevels {
            schema_version: 1,
            owner_id: publication.owner_id,
            revision: publication.revision,
            report_age_ms,
            microphone: publication.microphone,
            system: publication.system,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::fs;
    use std::fs::OpenOptions;
    const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
    fn report() -> Value {
        json!({"schemaVersion":1,"ownerId":OWNER,"revision":1,
            "microphone":{"state":"measured","windowFrames":1024,"ageMs":10,"rms":0.5,"peak":2},
            "system":{"state":"waiting","windowFrames":null,"ageMs":null,"rms":null,"peak":null}})
    }
    fn directory() -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        (temp, path)
    }
    fn write(path: &Path, value: &Value) {
        fs::write(
            path.join("input-levels.json"),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn both_tracks_keep_exact_received_amplitude_and_unknown_nulls() {
        let r = parse(&serde_json::to_vec(&report()).unwrap(), OWNER).unwrap();
        assert_eq!(r.microphone.rms, Some(0.5));
        assert_eq!(r.microphone.peak, Some(2.0));
        assert_eq!(r.system.state, "waiting");
        assert_eq!(r.system.rms, None);
        let mut silence = report();
        silence["microphone"]["rms"] = json!(0);
        silence["microphone"]["peak"] = json!(0);
        assert_eq!(
            parse(&serde_json::to_vec(&silence).unwrap(), OWNER)
                .unwrap()
                .microphone
                .state,
            "measured"
        );
    }
    #[test]
    fn strict_owner_version_and_measurement_shape_reject_invented_values() {
        let mut invalid = Vec::new();
        for (key, value) in [
            ("schemaVersion", json!(2)),
            ("ownerId", json!(OWNER.to_uppercase())),
            ("revision", json!(0)),
            ("revision", json!(SAFE + 1)),
            ("unknown", json!(1)),
        ] {
            let mut r = report();
            r[key] = value;
            invalid.push(r);
        }
        for (key, value) in [
            ("windowFrames", json!(0)),
            ("windowFrames", json!(1_000_001)),
            ("ageMs", json!(-1)),
            ("ageMs", json!(SAFE + 1)),
            ("ageMs", json!(0.1)),
            ("rms", json!(-1)),
            ("rms", json!(3)),
            ("peak", json!(17)),
            ("peak", Value::Null),
            ("unknown", json!(1)),
        ] {
            let mut r = report();
            r["microphone"][key] = value;
            invalid.push(r);
        }
        let mut missing = report();
        missing["system"].as_object_mut().unwrap().remove("rms");
        invalid.push(missing);
        let mut fabricated = report();
        fabricated["system"]["rms"] = json!(0);
        invalid.push(fabricated);
        for r in invalid {
            assert!(
                parse(&serde_json::to_vec(&r).unwrap(), OWNER).is_err(),
                "{r}"
            );
        }
        assert!(parse(b"", OWNER).is_err());
        assert!(parse(&vec![b' '; LIMIT as usize + 1], OWNER).is_err());
        assert!(parse(
            &serde_json::to_vec(&report()).unwrap(),
            "735027d4-a03e-43d3-830a-21e767dfaf89"
        )
        .is_err());
    }
    #[test]
    fn polling_same_publication_and_rewriting_its_mtime_never_refreshes_it() {
        let (_temp, path) = directory();
        write(&path, &report());
        let mut reader = LevelReader::default();
        let first = reader.read(&path, OWNER).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        write(&path, &report());
        let second = reader.read(&path, OWNER).unwrap();
        assert_eq!(first.revision, second.revision);
        assert!(second.report_age_ms >= first.report_age_ms + 25);
        let mut next = report();
        next["revision"] = json!(2);
        write(&path, &next);
        assert_eq!(reader.read(&path, OWNER).unwrap().revision, 2);
        write(&path, &report());
        assert!(reader.read(&path, OWNER).is_err());
    }
    #[test]
    fn contradictory_same_revision_and_future_publication_time_are_rejected() {
        let (_temp, path) = directory();
        write(&path, &report());
        let mut reader = LevelReader::default();
        reader.read(&path, OWNER).unwrap();
        let mut changed = report();
        changed["microphone"]["peak"] = json!(1);
        write(&path, &changed);
        assert!(reader.read(&path, OWNER).is_err());
        write(&path, &report());
        let file = OpenOptions::new()
            .write(true)
            .open(path.join("input-levels.json"))
            .unwrap();
        file.set_times(
            std::fs::FileTimes::new()
                .set_modified(SystemTime::now() + std::time::Duration::from_secs(3600)),
        )
        .unwrap();
        assert!(reader.read(&path, OWNER).is_err());
    }
    #[test]
    fn missing_directory_and_oversized_or_in_place_changed_payload_fail_closed() {
        let (_temp, path) = directory();
        let mut reader = LevelReader::default();
        assert!(reader.read(&path, OWNER).is_err());
        fs::create_dir(path.join("input-levels.json")).unwrap();
        assert!(reader.read(&path, OWNER).is_err());
        fs::remove_dir(path.join("input-levels.json")).unwrap();
        fs::write(
            path.join("input-levels.json"),
            vec![b' '; LIMIT as usize + 1],
        )
        .unwrap();
        assert!(reader.read(&path, OWNER).is_err());
        fs::write(path.join("input-levels.json"), b"{invalid}").unwrap();
        assert!(reader.read(&path, OWNER).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_file_parent_and_hard_link_never_supply_telemetry() {
        use std::os::unix::fs::symlink;
        let (temp, path) = directory();
        let real = path.join("real");
        fs::create_dir(&real).unwrap();
        write(&real, &report());
        let link = path.join("link");
        symlink(&real, &link).unwrap();
        let mut reader = LevelReader::default();
        assert!(reader.read(&link, OWNER).is_err());
        assert!(open_capture_file(&link, "input-levels.json").is_err());
        symlink(
            real.join("input-levels.json"),
            path.join("input-levels.json"),
        )
        .unwrap();
        assert!(reader.read(&path, OWNER).is_err());
        fs::remove_file(path.join("input-levels.json")).unwrap();
        fs::hard_link(
            real.join("input-levels.json"),
            path.join("input-levels.json"),
        )
        .unwrap();
        assert!(reader.read(&path, OWNER).is_err());
        drop(temp);
    }
    #[cfg(unix)]
    #[test]
    fn fifo_is_rejected_without_blocking_for_a_writer() {
        use std::os::unix::ffi::OsStrExt;
        let (_temp, path) = directory();
        let fifo = path.join("input-levels.json");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(LevelReader::default().read(&path, OWNER).is_err());
    }
    #[test]
    #[ignore = "requires Swift-generated PCM telemetry through make check-input-levels"]
    fn swift_generated_input_levels_read_independent_pcm_measurements() {
        let directory =
            std::path::PathBuf::from(std::env::var("MON_INPUT_LEVEL_FIXTURE_DIR").unwrap())
                .canonicalize()
                .unwrap();
        let r = LevelReader::default().read(&directory, OWNER).unwrap();
        assert_eq!(r.microphone.state, "measured");
        assert_eq!(r.microphone.window_frames, Some(2));
        assert_eq!(r.microphone.age_ms, Some(10));
        assert_eq!(r.microphone.peak, Some(0.5));
        assert!((r.microphone.rms.unwrap() - (0.15625_f64).sqrt()).abs() < 1e-12);
        assert_eq!(r.system.state, "measured");
        assert_eq!(r.system.rms, Some(0.0));
        assert_eq!(r.system.peak, Some(0.0));
        assert!(!directory.join("microphone.caf").exists());
    }
}
