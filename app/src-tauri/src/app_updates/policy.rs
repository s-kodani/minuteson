//! Pure channel, cadence and release validation. No storage or meeting locks.
use mon_contracts::{AppUpdateChannel as Channel, AppUpdateState as Phase, AppUpdateStatus};
use serde::Deserialize;
use std::time::{Duration, Instant};

pub(super) const CACHE: Duration = Duration::from_secs(30 * 60);
pub(super) const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
pub(super) const DEADLINE: Duration = Duration::from_secs(20);
pub(super) const MAX_BODY: usize = 256 * 1024;
pub(super) const STORE_PAGE: &str = "ms-windows-store://downloadsandupdates";
pub(super) const RELEASE_API: &str =
    "https://api.github.com/repos/s-kodani/minuteson/releases/latest";
const RELEASES: &str = "https://github.com/s-kodani/minuteson/releases";

pub(super) fn channel(os: &str, identifier: &str, debug: bool, store_package: bool) -> Channel {
    match (os, identifier, debug, store_package) {
        ("macos", "jp.mon.desktop", false, _) => Channel::MacosRelease,
        ("windows", "jp.mon.desktop.store", false, true) => Channel::WindowsStore,
        _ => Channel::Unsupported,
    }
}

pub(super) struct Model {
    pub status: AppUpdateStatus,
    attempted: Option<Instant>,
    succeeded: Option<Instant>,
}
impl Model {
    pub fn new(channel: Channel, current_version: String) -> Self {
        Self {
            status: AppUpdateStatus {
                revision: 0,
                channel,
                state: if channel == Channel::Unsupported {
                    Phase::Unsupported
                } else {
                    Phase::NotChecked
                },
                current_version,
                update_available: false,
                available_version: None,
                checked_at_ms: None,
                error_message: None,
            },
            attempted: None,
            succeeded: None,
        }
    }
    pub fn due(&self, now: Instant) -> bool {
        self.attempted
            .is_none_or(|last| now.saturating_duration_since(last) >= INTERVAL)
    }
    pub fn begin(&mut self, now: Instant) -> bool {
        if self.status.channel == Channel::Unsupported
            || self.status.state == Phase::Checking
            || self
                .succeeded
                .is_some_and(|last| now.saturating_duration_since(last) < CACHE)
        {
            return false;
        }
        self.attempted = Some(now);
        self.status.revision += 1;
        self.status.state = Phase::Checking;
        self.status.error_message = None;
        true
    }
    pub fn finish(
        &mut self,
        result: Result<(bool, Option<String>), String>,
        now: Instant,
        wall_ms: u64,
    ) {
        self.status.revision += 1;
        match result {
            Ok((available, version)) => {
                self.succeeded = Some(now);
                self.status.state = if available {
                    Phase::Available
                } else {
                    Phase::NoUpdate
                };
                self.status.update_available = available;
                self.status.available_version = version;
                self.status.checked_at_ms = Some(wall_ms);
                self.status.error_message = None;
            }
            Err(_) => {
                // Preserve a previously observed update, labeled stale by the UI.
                self.status.state = Phase::Unavailable;
                self.status.error_message = Some(
                    "更新を確認できませんでした。接続を確認して、もう一度お試しください。".into(),
                );
            }
        }
    }
    pub fn destination(&self) -> Result<String, String> {
        if !self.status.update_available {
            return Err("利用可能な更新を確認してから配布元を開いてください".into());
        }
        match self.status.channel {
            Channel::WindowsStore => Ok(STORE_PAGE.into()),
            Channel::MacosRelease => {
                let version = self
                    .status
                    .available_version
                    .as_deref()
                    .ok_or("更新版を確認できません")?;
                stable(version)?;
                Ok(format!("{RELEASES}/tag/v{version}"))
            }
            Channel::Unsupported => Err("この配布版は更新確認の対象外です".into()),
        }
    }
}

fn stable(value: &str) -> Result<semver::Version, String> {
    let parsed = semver::Version::parse(value).map_err(|_| "公開版のバージョンを確認できません")?;
    if !parsed.pre.is_empty() || !parsed.build.is_empty() {
        return Err("安定版のバージョンではありません".into());
    }
    Ok(parsed)
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    html_url: String,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
    browser_download_url: String,
}

pub(super) fn macos_release(body: &[u8], current: &str) -> Result<(bool, Option<String>), String> {
    if body.len() > MAX_BODY {
        return Err("更新情報の容量が上限を超えています".into());
    }
    let release: Release =
        serde_json::from_slice(body).map_err(|_| "公開版情報の形式を確認できません")?;
    if release.draft || release.prerelease {
        return Err("公開された安定版ではありません".into());
    }
    let version = release
        .tag_name
        .strip_prefix('v')
        .ok_or("公開版のタグを確認できません")?;
    let target = stable(version)?;
    let page = format!("{RELEASES}/tag/v{version}");
    if release.html_url != page {
        return Err("公開版の配布元が一致しません".into());
    }
    if target <= stable(current)? {
        return Ok((false, None));
    }
    let name = format!("MinutesOn_{version}_aarch64.dmg");
    let url = format!("{RELEASES}/download/v{version}/{name}");
    if !release
        .assets
        .iter()
        .any(|a| a.name == name && a.state == "uploaded" && a.browser_download_url == url)
    {
        return Err("このMacに対応する公開版のDMGを確認できません".into());
    }
    Ok((true, Some(version.into())))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distribution_identity_and_runtime_gate_checks() {
        assert_eq!(
            channel("windows", "jp.mon.desktop.store", false, true),
            Channel::WindowsStore
        );
        assert_eq!(
            channel("macos", "jp.mon.desktop", false, false),
            Channel::MacosRelease
        );
        for (os, id, debug, packaged) in [
            ("windows", "jp.mon.desktop", false, false),
            ("windows", "jp.mon.desktop.store", false, false),
            ("windows", "jp.mon.desktop.store", true, true),
            ("macos", "jp.mon.desktop.qa", false, false),
            ("macos", "jp.mon.desktop", true, false),
            ("linux", "jp.mon.desktop", false, false),
        ] {
            assert_eq!(channel(os, id, debug, packaged), Channel::Unsupported);
        }
    }
    #[test]
    fn cadence_coalesces_checks_preserves_failures_and_success_cache() {
        let t = Instant::now();
        let mut m = Model::new(Channel::WindowsStore, "1.0.1".into());
        assert!(m.due(t));
        assert!(m.begin(t));
        assert!(!m.begin(t));
        m.finish(Ok((true, None)), t, 10);
        assert!(!m.begin(t + CACHE - Duration::from_secs(1)));
        assert!(m.begin(t + CACHE));
        m.finish(Err("offline".into()), t + CACHE, 20);
        assert_eq!(m.status.state, Phase::Unavailable);
        assert!(m.status.update_available);
        assert_eq!(m.status.checked_at_ms, Some(10));
        assert_eq!(m.destination().unwrap(), STORE_PAGE);
        assert!(!m.due(t + CACHE + Duration::from_secs(1)));
        assert!(m.due(t + CACHE + INTERVAL));
        m.finish(Ok((false, None)), t + INTERVAL, 30);
        assert!(m.destination().is_err());
        let mut unsupported = Model::new(Channel::Unsupported, "1.0.1".into());
        assert!(!unsupported.begin(t));
    }
    fn release(version: &str) -> serde_json::Value {
        serde_json::json!({"tag_name":format!("v{version}"),"draft":false,"prerelease":false,
            "html_url":format!("{RELEASES}/tag/v{version}"),"assets":[{"name":format!("MinutesOn_{version}_aarch64.dmg"),
            "state":"uploaded","browser_download_url":format!("{RELEASES}/download/v{version}/MinutesOn_{version}_aarch64.dmg")}]})
    }
    #[test]
    fn releases_require_a_new_stable_matching_platform_asset_and_fixed_origin() {
        let check = |r: &serde_json::Value| macos_release(&serde_json::to_vec(r).unwrap(), "1.0.9");
        assert_eq!(
            check(&release("1.0.10")).unwrap(),
            (true, Some("1.0.10".into()))
        );
        for version in ["1.0.9", "1.0.8"] {
            assert_eq!(check(&release(version)).unwrap(), (false, None));
        }
        for version in ["1.1.0-beta.1", "1.0.10+build.1", "01.2.3", "../../bad"] {
            assert!(check(&release(version)).is_err());
        }
        for field in ["draft", "prerelease"] {
            let mut r = release("1.0.10");
            r[field] = true.into();
            assert!(check(&r).is_err());
        }
        for field in ["name", "state", "browser_download_url"] {
            let mut r = release("1.0.10");
            r["assets"][0][field] = "wrong".into();
            assert!(check(&r).is_err());
        }
        let mut r = release("1.0.10");
        r["html_url"] = "https://example.test/".into();
        assert!(check(&r).is_err());
        assert!(macos_release(b"invalid", "1.0.1").is_err());
        assert!(macos_release(&vec![b' '; MAX_BODY + 1], "1.0.1").is_err());
    }
}
