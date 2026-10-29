//! The same curl completion and diagnostic policy on every desktop platform.
use std::process::Command;
use std::time::Duration;

pub const HTTP_MARKER: &str = "MON_HTTP_STATUS:";
pub const MAX_DIAGNOSTIC_BYTES: usize =
    crate::process::Purpose::SystemTool.policy().stderr_bytes as usize;
pub const DEADLINE: Duration = match crate::process::Purpose::SystemTool.policy().deadline {
    crate::process::Deadline::Wall(value) => value,
    _ => panic!("system tool deadline"),
};
pub const HTTP_FAILURE: &str =
    "配布元の通信・証明書・HTTP応答を確認できません。自動再試行はしません";
pub const DIAGNOSTIC_FAILURE: &str = "ダウンロード通信状態が上限を超えています";

/// Network stderr may contain bearer URLs, proxy credentials and server text.
/// It is bounded evidence for a terminal marker, never a user-visible message.
#[derive(Default)]
pub struct Diagnostics {
    bytes: Vec<u8>,
    failed: bool,
}
impl Diagnostics {
    pub fn push(&mut self, bytes: &[u8]) {
        if self.bytes.len().saturating_add(bytes.len()) > MAX_DIAGNOSTIC_BYTES {
            self.failed = true;
        } else if !self.failed {
            self.bytes.extend_from_slice(bytes);
        }
    }
    pub fn read_failed(&mut self) {
        self.failed = true;
    }
    pub fn check(&self) -> Result<(), &'static str> {
        if self.failed {
            Err(DIAGNOSTIC_FAILURE)
        } else {
            Ok(())
        }
    }
    pub fn complete_http(&self, process_success: bool) -> Result<(), &'static str> {
        self.check()?;
        if process_success && self.bytes.ends_with(b"MON_HTTP_STATUS:200\n") {
            Ok(())
        } else {
            Err(HTTP_FAILURE)
        }
    }
}

/// --disable remains first so curl never reads a user's curlrc. This does not
/// configure environment or process ownership, which the native builder owns.
pub fn configure_curl(command: &mut Command, url: &str, protocol: &str) {
    command.args([
        "--disable",
        "--globoff",
        "--no-buffer",
        "--silent",
        "--show-error",
        "--fail",
        "--proto",
        protocol,
        "--proto-redir",
        protocol,
        "--location",
        "--max-redirs",
        "5",
        "--connect-timeout",
        "15",
        "--max-time",
        &DEADLINE.as_secs().to_string(),
        "--speed-limit",
        "1024",
        "--speed-time",
        "30",
        "--write-out",
        "%{stderr}MON_HTTP_STATUS:%{http_code}\n",
        "--url",
        url,
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_successful_complete_200_has_the_same_contract_on_every_os() {
        for code in ["200", "206", "204", "301", "404", "000"] {
            let mut diagnostic = Diagnostics::default();
            diagnostic.push(format!("{HTTP_MARKER}{code}\n").as_bytes());
            assert_eq!(diagnostic.complete_http(true).is_ok(), code == "200");
            assert!(diagnostic.complete_http(false).is_err());
        }
        let mut diagnostic = Diagnostics::default();
        diagnostic.push(b"MON_HTTP_STATUS:200\nextra");
        assert!(diagnostic.complete_http(true).is_err());
    }
    #[test]
    fn tokens_are_never_in_errors_and_oversized_or_failed_diagnostics_fail_closed() {
        let mut diagnostic = Diagnostics::default();
        diagnostic.push(b"https://server/?token=secret\nMON_HTTP_STATUS:403\n");
        assert_eq!(diagnostic.complete_http(true), Err(HTTP_FAILURE));
        assert!(!diagnostic
            .complete_http(true)
            .unwrap_err()
            .contains("secret"));
        diagnostic.push(&vec![b'x'; MAX_DIAGNOSTIC_BYTES]);
        assert_eq!(diagnostic.complete_http(true), Err(DIAGNOSTIC_FAILURE));
        let mut diagnostic = Diagnostics::default();
        diagnostic.push(b"MON_HTTP_STATUS:200\n");
        diagnostic.read_failed();
        assert!(diagnostic.complete_http(true).is_err());
    }
    #[test]
    fn curl_policy_disables_configuration_before_any_other_option() {
        let mut command = Command::new("curl");
        configure_curl(&mut command, "https://example.test/model", "=https");
        let args = command.get_args().collect::<Vec<_>>();
        assert_eq!(args[0], "--disable");
        assert!(args.windows(2).any(|a| a == ["--proto", "=https"]));
        assert!(args.windows(2).any(|a| a == ["--proto-redir", "=https"]));
        assert!(args
            .iter()
            .any(|a| a == &"%{stderr}MON_HTTP_STATUS:%{http_code}\n"));
    }
}
