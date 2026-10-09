//! The macOS system curl is a transport, never a shell or model executable.
//! Parent-side streaming limits do not depend on HTTP Content-Length or curl's
//! version-dependent --max-filesize behavior. Only this owned child is killed.
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const HTTP_MARKER: &str = "MON_HTTP_STATUS:";
const MAX_ERROR: usize = 8192;
const DEADLINE: Duration = Duration::from_secs(3600);

struct OwnedChild {
    child: Child,
    reaped: bool,
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
fn nonblocking(fd: RawFd) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("ダウンロード通信パイプを準備できません".into());
    }
    Ok(())
}

fn command(url: &str, protocol: &str) -> Command {
    let mut command = Command::new("/usr/bin/curl");
    // --disable MUST be the first option: never read ~/.curlrc. Do not inherit
    // certificate overrides, proxy credentials, DYLD variables or cookies.
    // HTTPS-only redirects allow model hosts' CDN transfers, not downgrades.
    command.env_clear().env("LC_ALL", "C").args([
        "--disable",
        "--globoff",
        // stdout is a pipe; deliver small partial bodies before completion so
        // progress and mid-stream cancellation do not wait for stdio buffering.
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
        "3600",
        "--speed-limit",
        "1024",
        "--speed-time",
        "30",
        "--write-out",
        "%{stderr}MON_HTTP_STATUS:%{http_code}\n",
        "--url",
        url,
    ]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

pub(super) fn fetch(
    url: &str,
    expected_size: u64,
    expected_hash: &str,
    output: &mut File,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64),
) -> Result<(), String> {
    super::validate_url(url)?;
    transfer(
        command(url, "=https"),
        expected_size,
        expected_hash,
        output,
        cancelled,
        progress,
        DEADLINE,
    )
}

fn transfer(
    mut command: Command,
    expected_size: u64,
    expected_hash: &str,
    output: &mut File,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64),
    deadline: Duration,
) -> Result<(), String> {
    if cancelled() {
        return Err("モデルダウンロードを中断しました".into());
    }
    let mut owned = OwnedChild {
        child: command
            .spawn()
            .map_err(|_| "システムのダウンロード通信を開始できません")?,
        reaped: false,
    };
    let mut body = owned
        .child
        .stdout
        .take()
        .ok_or("通信本文パイプがありません")?;
    let mut error = owned
        .child
        .stderr
        .take()
        .ok_or("通信状態パイプがありません")?;
    nonblocking(body.as_raw_fd())?;
    nonblocking(error.as_raw_fd())?;
    let started = Instant::now();
    let mut body_eof = false;
    let mut error_eof = false;
    let mut status = None;
    let mut count = 0_u64;
    let mut digest = Sha256::new();
    let mut error_bytes = Vec::new();
    let mut buffer = [0_u8; 65_536];
    loop {
        if cancelled() {
            return Err("モデルダウンロードを中断しました。途中ファイルは保全します".into());
        }
        if started.elapsed() >= deadline {
            return Err("モデルダウンロードの通信時間が上限を超えました".into());
        }
        if !body_eof {
            match body.read(&mut buffer) {
                Ok(0) => body_eof = true,
                Ok(n) => {
                    count = count
                        .checked_add(n as u64)
                        .filter(|n| *n <= expected_size)
                        .ok_or("配布元の本文が確認済み容量を超えています")?;
                    output
                        .write_all(&buffer[..n])
                        .map_err(|_| "ダウンロード本文を保存できません")?;
                    digest.update(&buffer[..n]);
                    progress(count);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err("ダウンロード本文を読み取れません".into()),
            }
        }
        if !error_eof {
            match error.read(&mut buffer) {
                Ok(0) => error_eof = true,
                Ok(n) => {
                    if error_bytes.len() + n > MAX_ERROR {
                        return Err("ダウンロード通信状態が上限を超えています".into());
                    }
                    error_bytes.extend_from_slice(&buffer[..n]);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err("ダウンロード通信状態を読み取れません".into()),
            }
        }
        if status.is_none() {
            status = owned
                .child
                .try_wait()
                .map_err(|_| "ダウンロード通信の停止を確認できません")?;
            owned.reaped = status.is_some();
        }
        if body_eof && error_eof && status.is_some() {
            break;
        }
        let mut fds = [
            libc::pollfd {
                fd: if body_eof { -1 } else { body.as_raw_fd() },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if error_eof { -1 } else { error.as_raw_fd() },
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, 50) } < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
        {
            return Err("ダウンロード通信を待機できません".into());
        }
    }
    // Never return curl's stderr: it can include URL/CDN tokens, proxy or server
    // data. Only a fixed terminal marker and successful full HTTP 200 are used.
    if !status.unwrap().success()
        || !String::from_utf8_lossy(&error_bytes).ends_with(&format!("{HTTP_MARKER}200\n"))
    {
        return Err("配布元の通信・証明書・HTTP応答を確認できません。自動再試行はしません".into());
    }
    if count != expected_size || format!("{:x}", digest.finalize()) != expected_hash {
        return Err("ダウンロード本文の容量・SHA-256が一致しません".into());
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn fixture_fetch(
    url: &str,
    expected_size: u64,
    expected_hash: &str,
    output: &mut File,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64),
    deadline: Duration,
) -> Result<(), String> {
    let parsed = tauri::Url::parse(url).unwrap();
    assert_eq!(parsed.scheme(), "http");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    transfer(
        command(url, "=http"),
        expected_size,
        expected_hash,
        output,
        cancelled,
        progress,
        deadline,
    )
}

#[cfg(test)]
pub(super) fn transport_arguments(url: &str) -> Vec<String> {
    command(url, "=https")
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect()
}
