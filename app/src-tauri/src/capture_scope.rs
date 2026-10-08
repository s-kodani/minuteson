//! Explicit application scope; never infer a target from a name or reused PID.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ApplicationIdentity {
    pub pid: i32,
    pub bundle_identifier: String,
    pub launch_time_us: i64,
}
impl ApplicationIdentity {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.pid <= 0
            || !crate::storage::valid_microphone_uid(&self.bundle_identifier)
            || !(1..=9_007_199_254_740_991).contains(&self.launch_time_us)
        {
            return Err("録音対象アプリの識別情報が不正です".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Application {
    pub(crate) pid: i32,
    pub(crate) bundle_identifier: String,
    pub(crate) launch_time_us: i64,
    pub(crate) name: String,
}
impl Application {
    fn identity(&self) -> ApplicationIdentity {
        ApplicationIdentity {
            pid: self.pid,
            bundle_identifier: self.bundle_identifier.clone(),
            launch_time_us: self.launch_time_us,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Applications {
    pub(crate) schema_version: u8,
    pub(crate) system_permission: String,
    pub(crate) applications: Vec<Application>,
}
impl Applications {
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() > 256 * 1024 {
            return Err("アプリ一覧の応答が不正です".into());
        }
        let result: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if result.schema_version != 1
            || result.applications.len() > 128
            || !matches!(
                result.system_permission.as_str(),
                "authorized" | "permissionRequired" | "unknown"
            )
        {
            return Err("アプリ一覧の形式が不正です".into());
        }
        let mut ids = std::collections::HashSet::new();
        for app in &result.applications {
            app.identity().validate()?;
            if app.name.trim().is_empty() || app.name.len() > 1024 || !ids.insert(app.pid) {
                return Err("アプリ一覧の識別情報が不正です".into());
            }
        }
        Ok(result)
    }
    pub(crate) fn require(&self, identity: &ApplicationIdentity) -> Result<(), String> {
        identity.validate()?;
        if self
            .applications
            .iter()
            .filter(|a| a.identity() == *identity)
            .count()
            != 1
        {
            return Err("確認したアプリが終了または再起動しました。明示選択してください。全体取得へは切り替えていません".into());
        }
        Ok(())
    }
}
pub(crate) fn read(binary: &Path) -> Result<Applications, String> {
    Applications::parse(&crate::capture_inputs::read_metadata(
        binary,
        "--list-capture-applications",
    )?)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct CaptureScope {
    pub(crate) mode: String,
    pub(crate) application: Option<ApplicationIdentity>,
}
impl CaptureScope {
    pub(crate) fn validate(&self) -> Result<(), String> {
        match (self.mode.as_str(), self.application.as_ref()) {
            ("system", None) => Ok(()),
            ("application", Some(identity)) => identity.validate(),
            _ => Err("PC音声の取得範囲が不正です".into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn catalog() -> serde_json::Value {
        serde_json::json!({"schemaVersion":1,"systemPermission":"unknown","applications":[{"pid":123,"bundleIdentifier":"com.example.browser","launchTimeUs":1791100000000001_i64,"name":"同じ名前"},{"pid":456,"bundleIdentifier":"com.example.other","launchTimeUs":1791100000000002_i64,"name":"同じ名前"}]})
    }
    #[test]
    fn application_scope_never_adopts_a_reused_pid_name_or_global_fallback() {
        let apps = Applications::parse(&serde_json::to_vec(&catalog()).unwrap()).unwrap();
        let identity = apps.applications[0].identity();
        assert!(apps.require(&identity).is_ok());
        for changed in [
            ApplicationIdentity {
                launch_time_us: identity.launch_time_us + 1,
                ..identity.clone()
            },
            ApplicationIdentity {
                bundle_identifier: "other".into(),
                ..identity.clone()
            },
            ApplicationIdentity {
                pid: 456,
                ..identity.clone()
            },
        ] {
            assert!(apps.require(&changed).is_err());
        }
        assert!(CaptureScope {
            mode: "system".into(),
            application: None
        }
        .validate()
        .is_ok());
        assert!(CaptureScope {
            mode: "system".into(),
            application: Some(identity.clone())
        }
        .validate()
        .is_err());
        assert!(CaptureScope {
            mode: "application".into(),
            application: None
        }
        .validate()
        .is_err());
        assert!(CaptureScope {
            mode: "application".into(),
            application: Some(identity)
        }
        .validate()
        .is_ok());
    }
    #[test]
    fn application_metadata_rejects_ambiguous_oversized_or_untrusted_catalogs() {
        for patch in [
            serde_json::json!({"schemaVersion":2}),
            serde_json::json!({"systemPermission":"granted"}),
            serde_json::json!({"unexpected":true}),
        ] {
            let mut v = catalog();
            v.as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(Applications::parse(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        let mut v = catalog();
        v["applications"][1]["pid"] = serde_json::json!(123);
        assert!(Applications::parse(&serde_json::to_vec(&v).unwrap()).is_err());
        for patch in [
            serde_json::json!({"pid":0}),
            serde_json::json!({"launchTimeUs":0}),
            serde_json::json!({"launchTimeUs":9007199254740992_i64}),
            serde_json::json!({"bundleIdentifier":"a\nb"}),
            serde_json::json!({"name":""}),
            serde_json::json!({"pid":2147483648_i64}),
        ] {
            let mut v = catalog();
            v["applications"][0]
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(Applications::parse(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        assert!(Applications::parse(&vec![b' '; 256 * 1024 + 1]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn application_catalog_invokes_only_the_read_only_command_and_rejects_nonzero_exit() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("catalog-helper");
        let body = format!("#!/bin/sh\nset -eu\n[ \"$#\" -eq 1 ]\n[ \"$1\" = --list-capture-applications ]\nprintf '%s' '{}'\n", catalog());
        std::fs::write(&path, &body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(read(&path).unwrap().applications.len(), 2);
        assert!(crate::capture_inputs::read_metadata(&path, "--output").is_err());
        std::fs::write(&path, format!("{body}exit 7\n")).unwrap();
        assert!(read(&path).is_err());
    }
}
