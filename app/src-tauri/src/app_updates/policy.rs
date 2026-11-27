//! Pure channel, cadence and release validation. No storage or meeting locks.
use mon_contracts::{AppUpdateChannel as Channel, AppUpdateState as Phase, AppUpdateStatus};
use std::time::{Duration, Instant};

pub(super) const CACHE: Duration = Duration::from_secs(30 * 60);
pub(super) const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
pub(super) const DEADLINE: Duration = Duration::from_secs(20);
pub(super) const STORE_PAGE: &str = "ms-windows-store://downloadsandupdates";
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
                downloaded_bytes: 0,
                total_bytes: None,
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
            || matches!(
                self.status.state,
                Phase::Checking | Phase::Downloading | Phase::Ready | Phase::Installing
            )
            || self.status.state != Phase::Failed
                && self
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
    pub fn transfer(
        &mut self,
        phase: Phase,
        bytes: u64,
        total: Option<u64>,
        error: Option<String>,
    ) {
        self.status.revision += 1;
        self.status.state = phase;
        self.status.downloaded_bytes = bytes;
        self.status.total_bytes = total;
        self.status.error_message = error;
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

/// Store installs a deferred update only after the app exits; quit through the normal path.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn require_store_quit(channel: Channel) -> Result<(), String> {
    if channel == Channel::WindowsStore {
        Ok(())
    } else {
        Err("この配布版はアプリ内更新の対象外です".into())
    }
}

fn stable(value: &str) -> Result<semver::Version, String> {
    let parsed = semver::Version::parse(value).map_err(|_| "公開版のバージョンを確認できません")?;
    if !parsed.pre.is_empty() || !parsed.build.is_empty() {
        return Err("安定版のバージョンではありません".into());
    }
    Ok(parsed)
}
pub(super) fn validate_download(version: &str, url: &str, arch: &str) -> Result<(), String> {
    stable(version)?;
    let expected = format!("https://github.com/s-kodani/minuteson/releases/download/v{version}/MinutesOn_{version}_aarch64.app.tar.gz");
    if arch != "aarch64" || url != expected {
        return Err("このMacに対応する更新の配布元を確認できません".into());
    }
    Ok(())
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
    #[test]
    fn store_quit_is_limited_to_the_store_channel() {
        assert!(require_store_quit(Channel::WindowsStore).is_ok());
        for channel in [Channel::MacosRelease, Channel::Unsupported] {
            assert!(require_store_quit(channel).is_err());
        }
    }
    #[test]
    fn downloads_require_stable_version_fixed_origin_and_supported_cpu() {
        let url = "https://github.com/s-kodani/minuteson/releases/download/v1.0.4/MinutesOn_1.0.4_aarch64.app.tar.gz";
        assert!(validate_download("1.0.4", url, "aarch64").is_ok());
        for (version, uri, arch) in [
            ("1.0.4-beta.1", url, "aarch64"),
            ("1.0.4", "https://example.test/app", "aarch64"),
            ("1.0.4", url, "x86_64"),
            ("1.0.5", url, "aarch64"),
        ] {
            assert!(validate_download(version, uri, arch).is_err());
        }
    }
    #[test]
    fn transfer_coalesces_checks_and_failed_downloads_allow_immediate_retry() {
        let now = Instant::now();
        let mut m = Model::new(Channel::MacosRelease, "1.0.3".into());
        assert!(m.begin(now));
        m.finish(Ok((true, Some("1.0.4".into()))), now, 1);
        for phase in [Phase::Downloading, Phase::Ready, Phase::Installing] {
            m.transfer(phase, 100, Some(200), None);
            assert!(!m.begin(now + INTERVAL));
            assert_eq!(m.status.downloaded_bytes, 100);
        }
        m.transfer(Phase::Failed, 0, None, Some("offline".into()));
        assert!(m.status.update_available);
        assert!(m.begin(now));
    }
}
