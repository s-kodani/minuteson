//! System curl and bounded pipe streaming. No PowerShell, curlrc, inherited
//! credentials, or unchecked Content-Length. Child ownership uses Job Objects.
use crate::local_engine::OwnedCommand;
use mon_native::transport::{configure_curl, Diagnostics, DEADLINE};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

pub(super) fn system_tool(name: &str) -> Result<std::path::PathBuf, String> {
    let root = std::env::var_os("SystemRoot").ok_or("Windowsのシステム領域がありません")?;
    Ok(std::path::PathBuf::from(root).join("System32").join(name))
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
    let mut launch = OwnedCommand::for_purpose(
        &system_tool("curl.exe")?,
        mon_native::process::Purpose::SystemTool,
    )?;
    configure_curl(&mut launch.command, url, "=https");
    stream(
        launch,
        expected_size,
        expected_hash,
        output,
        cancelled,
        progress,
        DEADLINE,
    )
}
fn stream(
    mut launch: OwnedCommand,
    expected_size: u64,
    expected_hash: &str,
    output: &mut File,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64),
    deadline: Duration,
) -> Result<(), String> {
    use std::process::Stdio;
    launch
        .command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = launch.spawn().map_err(|e| e.to_string())?;
    let mut stdout = child.child.stdout.take().ok_or("通信パイプがありません")?;
    let mut stderr = child.child.stderr.take().ok_or("診断パイプがありません")?;
    let (send, receive) = mpsc::sync_channel(4);
    let reader = std::thread::spawn(move || loop {
        let mut chunk = vec![0u8; 65536];
        match stdout.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                chunk.truncate(n);
                if send.send(Ok(chunk)).is_err() {
                    break;
                }
            }
            Err(e) => {
                let _ = send.send(Err(e.to_string()));
                break;
            }
        }
    });
    let diagnostics = Arc::new(Mutex::new(Diagnostics::default()));
    let errors = diagnostics.clone();
    let error_reader = std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(mut text) = errors.lock() {
                        text.push(&buffer[..n]);
                    }
                }
                Err(_) => {
                    if let Ok(mut text) = errors.lock() {
                        text.read_failed();
                    }
                    break;
                }
            }
        }
    });
    let start = Instant::now();
    let result: Result<(), String> = (|| {
        let mut count = 0u64;
        let mut hash = Sha256::new();
        loop {
            diagnostics
                .lock()
                .map_err(|_| "通信状態を確認できません")?
                .check()
                .map_err(str::to_owned)?;
            if cancelled() {
                return Err("モデルの取得・展開を中断しました。途中ファイルは保持します".into());
            }
            if start.elapsed() > deadline {
                return Err("モデルの取得・展開が時間上限を超えました".into());
            }
            match receive.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(chunk)) => {
                    count = count
                        .checked_add(chunk.len() as u64)
                        .filter(|n| *n <= expected_size)
                        .ok_or("取得ファイルが確認した容量を超えました")?;
                    output.write_all(&chunk).map_err(|e| e.to_string())?;
                    hash.update(&chunk);
                    progress(count);
                }
                Ok(Err(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
        }
        let status = loop {
            if cancelled() || start.elapsed() > deadline {
                return Err("取得・展開の終了待機を中断しました".into());
            }
            if let Some(status) = child.child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        if !status.success() {
            return Err(mon_native::transport::HTTP_FAILURE.into());
        }
        if count != expected_size || format!("{:x}", hash.finalize()) != expected_hash {
            return Err("取得ファイルの容量・SHA-256が確認内容と一致しません".into());
        }
        output.sync_all().map_err(|e| e.to_string())?;
        Ok(())
    })();
    drop(receive);
    drop(child);
    let _ = reader.join();
    let _ = error_reader.join();
    result?;
    let completion = diagnostics
        .lock()
        .map_err(|_| "通信状態を確認できません")?
        .complete_http(true)
        .map_err(str::to_owned);
    completion
}
