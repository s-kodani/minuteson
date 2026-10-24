//! Bounded, read-only helper metadata. This never starts a capture or asks TCC.
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const MAX_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct InputDevice {
    pub(crate) uid: String,
    pub(crate) name: String,
    pub(crate) is_default: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct InputDevices {
    pub(crate) schema_version: u32,
    pub(crate) microphone_permission: String,
    pub(crate) devices: Vec<InputDevice>,
    #[serde(default, rename = "selectedMicrophoneUID")]
    pub(crate) selected_microphone_uid: Option<String>,
}
impl InputDevices {
    pub(crate) fn require_automatic_microphone(&self) -> Result<(), String> {
        if matches!(self.microphone_permission.as_str(), "denied" | "restricted") {
            return Err("マイクの利用を許可してから再確認してください".into());
        }
        if !self.devices.iter().any(|device| device.is_default) {
            return Err("OSの既定入力を確認できません。マイクを接続して再確認してください".into());
        }
        Ok(())
    }

    pub(crate) fn require_microphone(&self, uid: &str) -> Result<(), String> {
        if !crate::storage::valid_microphone_uid(uid) || !self.devices.iter().any(|d| d.uid == uid)
        {
            return Err("選択したマイクが見つかりません。一覧を再確認して明示選択してください。別機器への切替はしていません".into());
        }
        if matches!(self.microphone_permission.as_str(), "denied" | "restricted") {
            return Err("マイクの利用を許可してから再確認してください".into());
        }
        Ok(())
    }
}
fn parse(bytes: &[u8]) -> Result<InputDevices, String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err("マイク一覧の応答が不正です".into());
    }
    let result: InputDevices = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if result.schema_version != 1
        || result.devices.len() > 128
        || !matches!(
            result.microphone_permission.as_str(),
            "authorized" | "denied" | "restricted" | "notDetermined" | "unknown"
        )
    {
        return Err("マイク一覧の形式が不正です".into());
    }
    let mut ids = std::collections::HashSet::new();
    if result.devices.iter().any(|d| {
        !crate::storage::valid_microphone_uid(&d.uid)
            || d.name.trim().is_empty()
            || d.name.len() > 1024
            || !ids.insert(&d.uid)
    }) || result.devices.iter().filter(|d| d.is_default).count() > 1
        || result.selected_microphone_uid.is_some()
    {
        return Err("マイク一覧の識別情報が不正です".into());
    }
    Ok(result)
}

struct Probe(Child);
impl Drop for Probe {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
pub(crate) fn read(binary: &Path) -> Result<InputDevices, String> {
    read_with_timeout(binary, Duration::from_secs(5))
}
fn read_with_timeout(binary: &Path, timeout: Duration) -> Result<InputDevices, String> {
    parse(&read_report(binary, "--list-input-devices", timeout)?)
}
pub(crate) fn read_metadata(binary: &Path, flag: &str) -> Result<Vec<u8>, String> {
    if !matches!(flag, "--list-input-devices" | "--list-capture-applications") {
        return Err("一覧取得のコマンドが不正です".into());
    }
    read_report(binary, flag, Duration::from_secs(5))
}
fn read_report(binary: &Path, flag: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let mut report = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut command = Command::new(binary);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = Probe(
        command
            .arg(flag)
            .stdin(Stdio::null())
            .stdout(Stdio::from(report.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?,
    );
    let start = Instant::now();
    let status = loop {
        if report.metadata().map_err(|e| e.to_string())?.len() > MAX_BYTES {
            return Err("取得対象一覧の応答が大きすぎます".into());
        }
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if start.elapsed() > timeout {
            return Err("取得対象一覧の取得が時間上限を超えました。再確認してください".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !status.success() {
        return Err(
            "取得対象一覧を取得できません。録音ヘルパーと機器の状態を確認してください".into(),
        );
    }
    report.rewind().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    report
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("取得対象一覧の応答が大きすぎます".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report() -> serde_json::Value {
        serde_json::json!({"schemaVersion":1,"microphonePermission":"notDetermined","devices":[{"uid":"usb-123","name":"同じ名前","isDefault":false},{"uid":"built-in","name":"同じ名前","isDefault":true}]})
    }
    #[test]
    fn input_catalog_uses_stable_ids_without_name_or_default_fallback() {
        let result = parse(&serde_json::to_vec(&report()).unwrap()).unwrap();
        assert!(result.require_microphone("usb-123").is_ok());
        assert!(result.require_automatic_microphone().is_ok());
        assert!(result.require_microphone("同じ名前").is_err());
        assert!(result.require_microphone("missing").is_err());
        let mut result = result;
        result.microphone_permission = "denied".into();
        assert!(result.require_microphone("usb-123").is_err());
        assert!(result.require_automatic_microphone().is_err());
        result.microphone_permission = "unknown".into();
        assert!(result.require_microphone("usb-123").is_ok());
        result.devices.retain(|device| !device.is_default);
        assert!(result.require_automatic_microphone().is_err());
    }
    #[test]
    fn input_catalog_rejects_oversized_ambiguous_and_untrusted_metadata() {
        assert!(parse(&vec![b' '; MAX_BYTES as usize + 1]).is_err());
        for patch in [
            serde_json::json!({"schemaVersion":2}),
            serde_json::json!({"microphonePermission":"allowed"}),
            serde_json::json!({"selectedMicrophoneUID":"usb-123"}),
            serde_json::json!({"unexpected":true}),
        ] {
            let mut v = report();
            v.as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(parse(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        for key in ["uid", "name"] {
            let mut v = report();
            v["devices"][0][key] = serde_json::json!("");
            assert!(parse(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        let mut v = report();
        v["devices"][0]["uid"] = v["devices"][1]["uid"].clone();
        assert!(parse(&serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = report();
        v["devices"][0]["isDefault"] = serde_json::json!(true);
        assert!(parse(&serde_json::to_vec(&v).unwrap()).is_err());
    }

    #[cfg(unix)]
    fn fake_helper(directory: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("metadata-helper");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nset -eu\n[ \"$#\" -eq 1 ]\n[ \"$1\" = --list-input-devices ]\n{body}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn metadata_helper_only_receives_read_only_flag_and_nonzero_is_not_success() {
        let temp = tempfile::tempdir().unwrap();
        let json = report().to_string();
        let helper = fake_helper(temp.path(), &format!("printf '%s' '{json}'"));
        assert_eq!(read(&helper).unwrap().devices[0].uid, "usb-123");
        let helper = fake_helper(temp.path(), &format!("printf '%s' '{json}'\nexit 7"));
        assert!(read(&helper).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn metadata_helper_timeout_is_bounded_even_before_helper_startup() {
        let temp = tempfile::tempdir().unwrap();
        let helper = fake_helper(temp.path(), "while :; do :; done");
        let start = Instant::now();
        assert!(read_with_timeout(&helper, Duration::from_millis(100))
            .unwrap_err()
            .contains("時間上限"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn metadata_helper_guard_kills_and_reaps_the_owned_pid_without_startup_assumptions() {
        let temp = tempfile::tempdir().unwrap();
        let helper = fake_helper(temp.path(), "while :; do :; done");
        let child = Command::new(helper)
            .arg("--list-input-devices")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        drop(Probe(child));
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[cfg(unix)]
    #[test]
    fn metadata_helper_oversized_output_is_rejected_and_reaped() {
        let temp = tempfile::tempdir().unwrap();
        let helper = fake_helper(
            temp.path(),
            "printf '%s' \"$$\" > \"$0.pid\"\nwhile :; do printf '%01024d' 0; done",
        );
        assert!(read(&helper).unwrap_err().contains("大きすぎ"));
        let pid: i32 = std::fs::read_to_string(temp.path().join("metadata-helper.pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
}
