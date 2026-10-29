//! Purpose-bound environments, default IO and the budgets consumed by native
//! launch/monitor adapters. OS process ownership remains with their guardian,
//! Job Object or parent-PID protocol. File-backed output is supplied explicitly.
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Capture,
    CaptureTest,
    CaptureMetadata,
    Import,
    Inference,
    Diarization,
    Embedding,
    Generation,
    Tokenize,
    Resident,
    SystemTool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deadline {
    /// Live recording/resident lifetime is owned, never a wall-clock cutoff.
    Owned,
    Wall(Duration),
    /// Only result growth resets this clock; stderr activity does not.
    Idle(Duration),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Closed,
    ProtocolPipe,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub input: Input,
    pub protocol_stdout: bool,
    pub deadline: Deadline,
    /// Per file/report/protocol message, or per line for capture diagnostics.
    /// Zero means discarded by default. A caller that opens a data stream (for
    /// example a hash-bound download) supplies that stream's domain budget.
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub result_bytes: u64,
}
impl Policy {
    pub const fn timeout(self) -> Option<Duration> {
        match self.deadline {
            Deadline::Owned => None,
            Deadline::Wall(value) | Deadline::Idle(value) => Some(value),
        }
    }
    /// Unit/probe callers retain their explicitly selected timeout. The caller
    /// supplies wall elapsed or idle elapsed according to `deadline`.
    pub fn expired(self, elapsed: Duration, requested: Option<Duration>) -> bool {
        match self.deadline {
            Deadline::Owned => false,
            Deadline::Wall(default) | Deadline::Idle(default) => {
                elapsed > requested.unwrap_or(default)
            }
        }
    }
}
impl Purpose {
    pub const ALL: &'static [Self] = &[
        Self::Capture,
        Self::CaptureTest,
        Self::CaptureMetadata,
        Self::Import,
        Self::Inference,
        Self::Diarization,
        Self::Embedding,
        Self::Generation,
        Self::Tokenize,
        Self::Resident,
        Self::SystemTool,
    ];
    pub const fn policy(self) -> Policy {
        let (deadline, stdout_bytes, stderr_bytes, result_bytes) = match self {
            Self::Capture => (Deadline::Owned, 0, 4096, 0),
            Self::CaptureTest => (Deadline::Owned, 0, 8192, 0),
            Self::CaptureMetadata => (Deadline::Wall(Duration::from_secs(5)), 256 * 1024, 0, 0),
            Self::Import => (Deadline::Wall(Duration::from_secs(3600)), 16 * 1024, 0, 0),
            Self::Inference => (
                Deadline::Wall(Duration::from_secs(1800)),
                4 * 1024 * 1024,
                4 * 1024 * 1024,
                8 * 1024 * 1024,
            ),
            Self::Diarization => (
                Deadline::Wall(Duration::from_secs(24 * 3600)),
                16 * 1024 * 1024,
                4 * 1024 * 1024,
                16 * 1024 * 1024,
            ),
            Self::Embedding => (
                Deadline::Wall(Duration::from_secs(3600)),
                64 * 1024,
                1024 * 1024,
                64 * 1024,
            ),
            Self::Generation => (
                Deadline::Idle(Duration::from_secs(1800)),
                2 * 1024 * 1024,
                4 * 1024 * 1024,
                2 * 1024 * 1024,
            ),
            Self::Tokenize => (
                Deadline::Wall(Duration::from_secs(5)),
                4096,
                1024 * 1024,
                4096,
            ),
            Self::Resident => (Deadline::Owned, 65536, 4 * 1024 * 1024, 0),
            Self::SystemTool => (Deadline::Wall(Duration::from_secs(3600)), 0, 8192, 0),
        };
        Policy {
            input: if matches!(self, Self::Resident) {
                Input::ProtocolPipe
            } else {
                Input::Closed
            },
            protocol_stdout: matches!(self, Self::Resident),
            deadline,
            stdout_bytes,
            stderr_bytes,
            result_bytes,
        }
    }
}
fn default_directory() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("System32")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/")
    }
}
pub fn configure(command: &mut Command, purpose: Purpose) {
    let policy = purpose.policy();
    command
        .env_clear()
        .env("LC_ALL", "C")
        .current_dir(default_directory())
        .stdin(match policy.input {
            Input::Closed => Stdio::null(),
            Input::ProtocolPipe => Stdio::piped(),
        })
        .stdout(if policy.protocol_stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null());
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command
            .env("SystemRoot", &root)
            .env("PATH", PathBuf::from(root).join("System32"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn purpose_budgets_keep_live_capture_import_and_engine_contracts() {
        let capture = Purpose::Capture.policy();
        assert_eq!(capture.timeout(), None);
        assert!(!capture.expired(
            Duration::from_secs(7 * 24 * 3600),
            Some(Duration::from_secs(1))
        ));
        let import = Purpose::Import.policy();
        assert_eq!(import.stdout_bytes, 16384);
        assert!(!import.expired(Duration::from_secs(3600), None));
        assert!(import.expired(Duration::from_secs(3601), None));
        assert!(Purpose::CaptureMetadata
            .policy()
            .expired(Duration::from_millis(101), Some(Duration::from_millis(100))));
        assert_eq!(Purpose::Inference.policy().result_bytes, 8 * 1024 * 1024);
        assert_eq!(Purpose::Diarization.policy().stdout_bytes, 16 * 1024 * 1024);
        assert_eq!(
            Purpose::Generation.policy().deadline,
            Deadline::Idle(Duration::from_secs(1800))
        );
    }
    #[test]
    fn every_purpose_removes_inherited_credentials_and_loader_overrides() {
        for &purpose in Purpose::ALL {
            let mut command = Command::new("/unused");
            command
                .env("TOKEN", "secret")
                .env("DYLD_INSERT_LIBRARIES", "bad");
            configure(&mut command, purpose);
            let vars = command.get_envs().collect::<Vec<_>>();
            assert!(!vars
                .iter()
                .any(|(key, _)| key == &"TOKEN" || key == &"DYLD_INSERT_LIBRARIES"));
            assert!(vars
                .iter()
                .any(|(key, value)| key == &"LC_ALL" && *value == Some(std::ffi::OsStr::new("C"))));
            assert_eq!(
                command.get_current_dir(),
                Some(default_directory().as_path())
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn spawned_child_cannot_read_parent_secrets_and_defaults_do_not_inherit_io() {
        let mut command = Command::new("/usr/bin/env");
        command.env("TOKEN", "secret");
        configure(&mut command, Purpose::SystemTool);
        command.stdout(Stdio::piped());
        let output = command.output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("secret"));
        assert!(text.contains("LC_ALL=C"));
        let mut quiet = Command::new("/bin/sh");
        configure(&mut quiet, Purpose::Capture);
        let output = quiet
            .args([
                "-c",
                "test \"$PWD\" = / && ! read value; echo hidden; echo hidden >&2",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        let mut protocol = Command::new("/bin/cat");
        configure(&mut protocol, Purpose::Resident);
        let mut child = protocol.spawn().unwrap();
        assert!(child.stdin.take().is_some() && child.stdout.take().is_some());
        child.wait().unwrap();
    }
}
