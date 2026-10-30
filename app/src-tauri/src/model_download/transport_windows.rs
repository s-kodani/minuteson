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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom};
    use std::net::TcpListener;

    #[test]
    fn windows_system_curl_stream_accepts_native_http_line_endings() {
        const BODY: &[u8] = b"Windows curl verified response\n";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || -> Result<(), String> {
            let started = Instant::now();
            let mut connection = loop {
                match listener.accept() {
                    Ok((connection, _)) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if started.elapsed() > Duration::from_secs(10) {
                            return Err("curl did not connect".into());
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            connection
                .set_nonblocking(false)
                .map_err(|error| error.to_string())?;
            connection
                .set_read_timeout(Some(Duration::from_secs(2)))
                .map_err(|error| error.to_string())?;
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut chunk = [0u8; 1024];
                let count = connection.read(&mut chunk).map_err(|e| e.to_string())?;
                if count == 0 || headers.len() + count > 16384 {
                    return Err("incomplete or oversized request".into());
                }
                headers.extend_from_slice(&chunk[..count]);
            }
            write!(
                connection,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                BODY.len()
            )
            .map_err(|error| error.to_string())?;
            connection
                .write_all(BODY)
                .map_err(|error| error.to_string())
        });
        let mut launch = OwnedCommand::for_purpose(
            &system_tool("curl.exe").unwrap(),
            mon_native::process::Purpose::SystemTool,
        )
        .unwrap();
        // Only this loopback fixture permits HTTP; production fetch requires HTTPS.
        configure_curl(&mut launch.command, &url, "=http");
        let mut output = tempfile::tempfile().unwrap();
        let mut progress = Vec::new();
        let result = stream(
            launch,
            BODY.len() as u64,
            &format!("{:x}", Sha256::digest(BODY)),
            &mut output,
            &|| false,
            &mut |count| progress.push(count),
            Duration::from_secs(10),
        );
        server.join().unwrap().unwrap();
        result.unwrap();
        output.seek(SeekFrom::Start(0)).unwrap();
        let mut actual = Vec::new();
        output.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, BODY);
        assert_eq!(progress.last(), Some(&(BODY.len() as u64)));
    }

    #[test]
    #[ignore = "downloads the fixed LFM3 model and notices over HTTPS into owned temporary storage"]
    fn windows_lfm3_network_download_finishes_with_hash_bound_receipt() {
        use crate::model_download_recipe::RecipeExt;
        #[derive(serde::Deserialize)]
        struct Choice {
            id: String,
            recipe: crate::model_download_recipe::Recipe,
        }
        let bundle = crate::windows_runtime::resources().unwrap();
        let choices: Vec<Choice> =
            serde_json::from_slice(&crate::windows_runtime::catalog(&bundle).unwrap()).unwrap();
        let recipe = choices
            .into_iter()
            .find(|choice| choice.id == "windows-llama-b11960-lfm3-17b-q4km")
            .unwrap()
            .recipe;
        let recipe_hash = recipe.hash().unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("downloads");
        let operation = uuid::Uuid::new_v4().to_string();
        let mut latest = Instant::now();
        let receipt = crate::model_download::download_platform(
            &root,
            &bundle,
            None,
            &operation,
            &recipe,
            &recipe_hash,
            &|| false,
            &mut |done, total| {
                if latest.elapsed().as_secs() >= 5 || done == total {
                    eprintln!("LFM3 download: {done}/{total}");
                    latest = Instant::now();
                }
            },
        )
        .unwrap();
        assert_eq!(receipt.recipe_hash, recipe_hash);
        assert!(!receipt.installed && !receipt.ready);
        let readback =
            crate::model_download::readback(&root, &operation, &recipe_hash, &|| false).unwrap();
        assert_eq!(readback.state, "complete");
        assert!(readback.receipt.is_some());
    }
}
