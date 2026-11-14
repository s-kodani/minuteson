use super::policy;
use std::{
    process::Command,
    time::{Duration, Instant},
};

pub(super) async fn check(current: &str) -> Result<(bool, Option<String>), String> {
    // No authentication, release-note HTML, downloads or arbitrary redirect targets.
    let client = reqwest::Client::builder()
        .timeout(policy::DEADLINE)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    policy::macos_release(&metadata(&client, policy::RELEASE_API).await?, current)
}

async fn metadata(client: &reqwest::Client, endpoint: &str) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(endpoint)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "MinutesOn-update-check")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(format!("release HTTP {}", response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|n| n > policy::MAX_BODY as u64)
    {
        return Err("release response too large".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if body.len().saturating_add(chunk.len()) > policy::MAX_BODY {
            return Err("release response too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    fn request(reply: Vec<u8>, delay: Duration, timeout: Duration) -> Result<Vec<u8>, String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/release", listener.local_addr().unwrap());
        let peer = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut received = [0u8; 2048];
            let _ = socket.read(&mut received);
            std::thread::sleep(delay);
            let _ = socket.write_all(&reply);
        });
        let result = tauri::async_runtime::block_on(async {
            let client = reqwest::Client::builder()
                .timeout(timeout)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            metadata(&client, &url).await
        });
        peer.join().unwrap();
        result
    }
    #[test]
    fn native_http_transport_bounds_bodies_rejects_failures_and_expires_stalls() {
        let immediate = Duration::ZERO;
        let budget = Duration::from_secs(2);
        assert_eq!(
            request(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
                immediate,
                budget
            )
            .unwrap(),
            b"{}"
        );
        for status in ["503 Unavailable", "302 Found"] {
            assert!(request(format!("HTTP/1.1 {status}\r\nLocation: http://127.0.0.1:1/\r\nContent-Length: 0\r\n\r\n").into_bytes(),immediate,budget).unwrap_err().contains("HTTP"));
        }
        assert!(request(
            b"HTTP/1.1 200 OK\r\nContent-Length: 999\r\n\r\nx".to_vec(),
            immediate,
            budget
        )
        .is_err());
        let mut oversized = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
            policy::MAX_BODY + 1
        )
        .into_bytes();
        oversized.extend(vec![b'x'; policy::MAX_BODY + 1]);
        oversized.extend(b"\r\n0\r\n\r\n");
        assert!(request(oversized, immediate, budget)
            .unwrap_err()
            .contains("too large"));
        assert!(request(
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
            Duration::from_millis(150),
            Duration::from_millis(25)
        )
        .is_err());
    }
}
pub(super) fn open(destination: &str) -> Result<(), String> {
    let mut command = Command::new("/usr/bin/open");
    mon_native::process::configure(&mut command, mon_native::process::Purpose::SystemTool);
    let mut child = command
        .arg(destination)
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err("open failed".into())
                }
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            other => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("open did not complete: {other:?}"));
            }
        }
    }
}
