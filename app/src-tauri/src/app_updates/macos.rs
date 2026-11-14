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

impl super::metadata::Response for reqwest::Response {
    fn status(&self) -> u16 {
        self.status().as_u16()
    }
    fn content_length(&self) -> Option<u64> {
        self.content_length()
    }
    async fn next(&mut self) -> Result<Option<Vec<u8>>, String> {
        self.chunk()
            .await
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
            .map_err(|e| e.to_string())
    }
}

async fn metadata(client: &reqwest::Client, endpoint: &str) -> Result<Vec<u8>, String> {
    super::metadata::read(async {
        client
            .get(endpoint)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "MinutesOn-update-check")
            .send()
            .await
            .map_err(|e| e.to_string())
    })
    .await
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
