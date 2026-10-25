//! Lifecycle of the Xray-core child process.

use crate::config::{self, API_PORT};
use crate::model::{Server, Settings};
use crate::store::{core_dir, data_dir};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

const MAX_LOG_LINES: usize = 500;

pub type LogSink = Arc<dyn Fn(&str) + Send + Sync>;

#[cfg(windows)]
const BINARY_NAME: &str = "xray.exe";
#[cfg(not(windows))]
const BINARY_NAME: &str = "xray";

/// Locate the Xray binary: explicit setting, then our own core directory, then PATH.
pub fn find_binary(settings: &Settings) -> Option<PathBuf> {
    let explicit = PathBuf::from(&settings.core_path);
    if !settings.core_path.is_empty() && explicit.is_file() {
        return Some(explicit);
    }
    let bundled = core_dir().join(BINARY_NAME);
    if bundled.is_file() {
        return Some(bundled);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(BINARY_NAME))
            .find(|candidate| candidate.is_file())
    })
}

/// geoip.dat/geosite.dat live next to our downloaded core; fall back to that directory
/// when the binary comes from somewhere without them.
fn asset_dir(binary: &Path) -> PathBuf {
    match binary.parent() {
        Some(dir) if dir.join("geoip.dat").is_file() => dir.to_path_buf(),
        _ => core_dir(),
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)?;
        Ok(())
    }
}

pub struct Core {
    child: Mutex<Option<Child>>,
    active: StdMutex<Option<String>>,
    logs: Arc<StdMutex<VecDeque<String>>>,
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

impl Core {
    pub fn new() -> Self {
        Core {
            child: Mutex::new(None),
            active: StdMutex::new(None),
            logs: Arc::new(StdMutex::new(VecDeque::new())),
        }
    }

    pub fn logs(&self) -> Vec<String> {
        self.logs.lock().unwrap().iter().cloned().collect()
    }

    pub fn push_log_line(&self, line: &str) {
        Core::push_log(&self.logs, line.to_string());
    }

    pub fn active_server(&self) -> Option<String> {
        self.active.lock().unwrap().clone()
    }

    fn push_log(logs: &StdMutex<VecDeque<String>>, line: String) {
        let mut logs = logs.lock().unwrap();
        if logs.len() == MAX_LOG_LINES {
            logs.pop_front();
        }
        logs.push_back(line);
    }

    fn pipe_lines<R: AsyncRead + Unpin + Send + 'static>(&self, stream: R, sink: LogSink) {
        let logs = Arc::clone(&self.logs);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                sink(&line);
                Core::push_log(&logs, line);
            }
        });
    }

    pub async fn start(
        &self,
        binary: &Path,
        server: &Server,
        settings: &Settings,
        sink: LogSink,
    ) -> Result<()> {
        self.stop().await;

        let config = config::generate(server, settings)?;
        std::fs::create_dir_all(data_dir())?;
        let config_path = data_dir().join("config.json");
        write_private(&config_path, &serde_json::to_vec_pretty(&config)?)?;

        let mut child = Command::new(binary)
            .arg("run")
            .arg("-c")
            .arg(&config_path)
            .env("XRAY_LOCATION_ASSET", asset_dir(binary))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {}", binary.display()))?;

        if let Some(out) = child.stdout.take() {
            self.pipe_lines(out, Arc::clone(&sink));
        }
        if let Some(err) = child.stderr.take() {
            self.pipe_lines(err, sink);
        }

        // A bad config or a busy port makes Xray exit almost immediately.
        tokio::time::sleep(Duration::from_millis(700)).await;
        if let Some(status) = child.try_wait()? {
            let tail: Vec<String> = self.logs().into_iter().rev().take(5).rev().collect();
            bail!("core exited right after start ({status})\n{}", tail.join("\n"));
        }

        *self.child.lock().await = Some(child);
        *self.active.lock().unwrap() = Some(server.id.clone());
        Ok(())
    }

    pub async fn stop(&self) {
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
        }
        *self.active.lock().unwrap() = None;
    }

    pub async fn running(&self) -> bool {
        match self.child.lock().await.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }
}

/// Ask Xray to validate the config for `server` without starting it (`xray run -test`).
pub async fn test_config(binary: &Path, server: &Server, settings: &Settings) -> Result<()> {
    let config = config::generate(server, settings)?;
    std::fs::create_dir_all(data_dir())?;
    let path = data_dir().join(format!("check-{}.json", uuid::Uuid::new_v4()));
    write_private(&path, &serde_json::to_vec(&config)?)?;
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new(binary)
            .args(["run", "-test", "-c"])
            .arg(&path)
            .env("XRAY_LOCATION_ASSET", asset_dir(binary))
            .kill_on_drop(true)
            .output(),
    )
    .await;
    let _ = std::fs::remove_file(&path);
    let output = output.context("config test timed out")??;
    if output.status.success() {
        return Ok(());
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let detail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).rev().take(4).collect();
    bail!("{}", detail.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

/// Cumulative (uplink, downlink) bytes through the proxy outbound, from Xray's stats API.
pub async fn traffic(binary: &Path) -> Result<(u64, u64)> {
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        Command::new(binary)
            .args([
                "api",
                "statsquery",
                &format!("-server=127.0.0.1:{API_PORT}"),
                "-pattern",
                "outbound>>>proxy>>>traffic",
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("stats query timed out")??;
    if !output.status.success() {
        bail!("stats query failed");
    }
    Ok(parse_traffic(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_traffic(json: &str) -> (u64, u64) {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return (0, 0);
    };
    let (mut up, mut down) = (0, 0);
    for stat in value["stat"].as_array().into_iter().flatten() {
        let name = stat["name"].as_str().unwrap_or_default();
        // Counters arrive as strings from some builds and numbers from others.
        let amount = match &stat["value"] {
            Value::String(s) => s.parse().unwrap_or(0),
            Value::Number(n) => n.as_u64().unwrap_or(0),
            _ => 0,
        };
        if name.ends_with(">>>uplink") {
            up = amount;
        } else if name.ends_with(">>>downlink") {
            down = amount;
        }
    }
    (up, down)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_traffic_counters() {
        let json = r#"{"stat":[
            {"name":"outbound>>>proxy>>>traffic>>>uplink","value":"1024"},
            {"name":"outbound>>>proxy>>>traffic>>>downlink","value":4096}
        ]}"#;
        assert_eq!(parse_traffic(json), (1024, 4096));
    }

    #[test]
    fn missing_counters_are_zero() {
        assert_eq!(parse_traffic("{}"), (0, 0));
        assert_eq!(parse_traffic("not json"), (0, 0));
        let only_down = r#"{"stat":[{"name":"outbound>>>proxy>>>traffic>>>downlink","value":"7"}]}"#;
        assert_eq!(parse_traffic(only_down), (0, 7));
    }
}
