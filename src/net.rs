//! Network helpers: subscriptions, latency tests and Xray-core download.

use crate::config;
use crate::model::{Server, Settings};
use crate::store::{core_dir, data_dir};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::process::Command;

const USER_AGENT: &str = concat!("MoxRay/", env!("CARGO_PKG_VERSION"));
const MAX_DOWNLOAD_BYTES: usize = 120 * 1024 * 1024;

fn client(proxy_socks_port: Option<u16>, timeout: Duration) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout);
    if let Some(port) = proxy_socks_port {
        builder = builder.proxy(reqwest::Proxy::all(format!("socks5h://127.0.0.1:{port}"))?);
    }
    Ok(builder.build()?)
}

fn require_http(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url).map_err(|e| anyhow!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        other => bail!("only http(s) URLs are allowed, got {other}"),
    }
}

/// Traffic quota and expiry reported by a provider in the `subscription-userinfo` header.
#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub struct Usage {
    pub used: Option<u64>,
    pub total: Option<u64>,
    pub expire: Option<u64>,
}

/// Parses `upload=1; download=2; total=3; expire=4`.
pub fn parse_userinfo(header: &str) -> Usage {
    let (mut upload, mut download) = (None, None);
    let mut usage = Usage::default();
    for pair in header.split(';') {
        let Some((key, value)) = pair.split_once('=') else { continue };
        let Ok(value) = value.trim().parse::<u64>() else { continue };
        match key.trim().to_ascii_lowercase().as_str() {
            "upload" => upload = Some(value),
            "download" => download = Some(value),
            "total" if value > 0 => usage.total = Some(value),
            "expire" if value > 0 => usage.expire = Some(value),
            _ => {}
        }
    }
    if upload.is_some() || download.is_some() {
        usage.used = Some(upload.unwrap_or(0) + download.unwrap_or(0));
    }
    usage
}

pub struct Fetched {
    pub body: String,
    pub usage: Usage,
}

async fn fetch_one(url: &str, proxy_socks_port: Option<u16>) -> Result<Fetched> {
    require_http(url)?;
    let response = client(proxy_socks_port, Duration::from_secs(25))?
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    let usage = response
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .map(parse_userinfo)
        .unwrap_or_default();
    Ok(Fetched { body: response.text().await?, usage })
}

/// Try each URL in turn (main link, then mirrors). When the local proxy is up each one is
/// tried through it first and directly second, so a blocked link has two chances.
pub async fn fetch_subscription(urls: &[String], proxy_socks_port: Option<u16>) -> Result<Fetched> {
    let routes: Vec<Option<u16>> = proxy_socks_port.into_iter().map(Some).chain([None]).collect();
    let mut last_error = anyhow!("no subscription URL");
    for url in urls.iter().filter(|u| !u.trim().is_empty()) {
        for route in &routes {
            match fetch_one(url.trim(), *route).await {
                Ok(fetched) => return Ok(fetched),
                Err(e) => last_error = e,
            }
        }
    }
    Err(last_error)
}

/// One request through the local SOCKS inbound; the delay in ms, or `None` if it failed.
pub async fn probe_via_proxy(socks_port: u16, test_url: &str) -> Option<u32> {
    require_http(test_url).ok()?;
    let http = client(Some(socks_port), Duration::from_secs(8)).ok()?;
    let started = Instant::now();
    http.get(test_url).send().await.ok()?.error_for_status().ok()?;
    Some(started.elapsed().as_millis() as u32)
}

/// What Cloudflare's trace endpoint says about the connection that reached it.
#[derive(Debug, Default, PartialEq, Clone, serde::Serialize)]
pub struct Trace {
    pub ip: String,
    pub country: String,
    pub colo: String,
}

pub fn parse_trace(text: &str) -> Trace {
    let mut trace = Trace::default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("ip", v)) => trace.ip = v.trim().into(),
            Some(("loc", v)) => trace.country = v.trim().into(),
            Some(("colo", v)) => trace.colo = v.trim().into(),
            _ => {}
        }
    }
    trace
}

/// The public address and country the world sees, directly or through the local proxy.
pub async fn trace(proxy_socks_port: Option<u16>) -> Result<Trace> {
    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(10));
    builder = match proxy_socks_port {
        Some(port) => builder.proxy(reqwest::Proxy::all(format!("socks5h://127.0.0.1:{port}"))?),
        None => builder.no_proxy(),
    };
    let text = builder
        .build()?
        .get("https://www.cloudflare.com/cdn-cgi/trace")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let trace = parse_trace(&text);
    if trace.ip.is_empty() {
        bail!("unexpected response");
    }
    Ok(trace)
}

/// TCP connect time to the server in milliseconds; `None` if unreachable.
pub async fn tcp_ping(server: &Server) -> Option<u32> {
    let started = Instant::now();
    let connect = TcpStream::connect((server.address.as_str(), server.port));
    match tokio::time::timeout(Duration::from_secs(4), connect).await {
        Ok(Ok(_)) => Some(started.elapsed().as_millis() as u32),
        _ => None,
    }
}

fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// Start a throwaway Xray instance for `server` and time one request through it.
pub async fn real_delay(binary: &Path, server: &Server, settings: &Settings) -> Result<u32> {
    let port = free_port()?;
    let config_path = data_dir().join(format!("probe-{}.json", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(data_dir())?;
    std::fs::write(&config_path, serde_json::to_vec(&config::generate_probe(server, settings, port)?)?)?;

    let result = probe(binary, &config_path, port, &settings.test_url).await;
    let _ = std::fs::remove_file(&config_path);
    result
}

async fn probe(binary: &Path, config_path: &Path, port: u16, test_url: &str) -> Result<u32> {
    require_http(test_url)?;
    let assets = binary
        .parent()
        .filter(|dir| dir.join("geoip.dat").is_file())
        .map(Path::to_path_buf)
        .unwrap_or_else(core_dir);
    let mut child = Command::new(binary)
        .arg("run")
        .arg("-c")
        .arg(config_path)
        .env("XRAY_LOCATION_ASSET", assets)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("starting probe core")?;

    let outcome = async {
        let mut ready = false;
        for _ in 0..40 {
            if child.try_wait()?.is_some() {
                bail!("probe core exited early");
            }
            if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !ready {
            bail!("probe core did not start");
        }
        let http = client(Some(port), Duration::from_secs(8))?;
        let started = Instant::now();
        http.get(test_url).send().await?.error_for_status()?;
        Ok(started.elapsed().as_millis() as u32)
    }
    .await;

    let _ = child.kill().await;
    outcome
}

fn release_asset_name() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("Xray-linux-64.zip"),
        ("linux", "aarch64") => Ok("Xray-linux-arm64-v8a.zip"),
        ("windows", "x86_64") => Ok("Xray-windows-64.zip"),
        ("macos", "x86_64") => Ok("Xray-macos-64.zip"),
        ("macos", "aarch64") => Ok("Xray-macos-arm64-v8a.zip"),
        (os, arch) => bail!("no Xray-core build known for {os}/{arch}"),
    }
}

fn parse_sha256(dgst: &str) -> Option<String> {
    dgst.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        let key = key.trim().to_ascii_uppercase();
        (key == "SHA2-256" || key == "SHA256").then(|| value.trim().to_ascii_lowercase())
    })
}

async fn download_bytes(http: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = http.get(url).send().await?.error_for_status()?;
    if response.content_length().unwrap_or(0) as usize > MAX_DOWNLOAD_BYTES {
        bail!("download is unexpectedly large");
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        bail!("download is unexpectedly large");
    }
    Ok(bytes.to_vec())
}

/// Download the latest official Xray-core release into the app's core directory.
/// The zip is only installed if it matches the SHA-256 digest published beside it.
pub async fn download_core(proxy_socks_port: Option<u16>) -> Result<String> {
    let asset = release_asset_name()?;
    let http = client(proxy_socks_port, Duration::from_secs(300))?;

    let release: Value = http
        .get("https://api.github.com/repos/XTLS/Xray-core/releases/latest")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let tag = release["tag_name"].as_str().unwrap_or("unknown").to_string();
    let url_of = |name: &str| {
        release["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["name"].as_str() == Some(name))
            .and_then(|a| a["browser_download_url"].as_str())
            .map(String::from)
    };
    let zip_url = url_of(asset).ok_or_else(|| anyhow!("release has no {asset}"))?;
    let dgst_url =
        url_of(&format!("{asset}.dgst")).ok_or_else(|| anyhow!("release has no digest for {asset}"))?;

    let zip_bytes = download_bytes(&http, &zip_url).await?;
    let dgst = String::from_utf8_lossy(&download_bytes(&http, &dgst_url).await?).into_owned();
    let expected = parse_sha256(&dgst).ok_or_else(|| anyhow!("digest file has no SHA-256"))?;
    let actual = hex(&Sha256::digest(&zip_bytes));
    if actual != expected {
        bail!("checksum mismatch: expected {expected}, got {actual}");
    }

    tokio::task::spawn_blocking(move || extract_core(&zip_bytes)).await??;
    Ok(tag)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn extract_core(zip_bytes: &[u8]) -> Result<()> {
    const WANTED: [&str; 4] = ["xray", "xray.exe", "geoip.dat", "geosite.dat"];
    let dir = core_dir();
    std::fs::create_dir_all(&dir)?;
    let mut archive = zip::ZipArchive::new(Cursor::new(zip_bytes))?;
    let mut found_binary = false;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        // Only extract known file names, flattened, so archive paths can't escape `dir`.
        let Some(name) = Path::new(entry.name()).file_name().and_then(|n| n.to_str()).map(String::from)
        else {
            continue;
        };
        if !WANTED.contains(&name.as_str()) {
            continue;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        let target = dir.join(&name);
        let tmp = dir.join(format!("{name}.part"));
        std::fs::write(&tmp, &data)?;
        #[cfg(unix)]
        if name == "xray" {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&tmp, &target)?;
        found_binary |= name.starts_with("xray");
    }
    if !found_binary {
        bail!("archive did not contain the xray binary");
    }
    Ok(())
}

/// `xray version` first line, e.g. "Xray 25.1.1 (Xray, Penetrates Everything.) ...".
pub async fn core_version(binary: &Path) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        Command::new(binary).arg("version").kill_on_drop(true).output(),
    )
    .await
    .ok()?
    .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(|l| l.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_sha256_from_dgst() {
        let dgst = "MD5= aaaa\nSHA1= bbbb\nSHA2-256= CCCC1234\nSHA2-512= dddd\n";
        assert_eq!(parse_sha256(dgst).as_deref(), Some("cccc1234"));
        assert_eq!(parse_sha256("MD5= x"), None);
    }

    #[test]
    fn parses_subscription_userinfo() {
        let usage = parse_userinfo("upload=100; download=900; total=10737418240; expire=1767225600");
        assert_eq!(usage.used, Some(1000));
        assert_eq!(usage.total, Some(10_737_418_240));
        assert_eq!(usage.expire, Some(1_767_225_600));
        // total=0 / expire=0 mean "unlimited" for most panels.
        let unlimited = parse_userinfo("upload=1;download=2;total=0;expire=0");
        assert_eq!((unlimited.total, unlimited.expire), (None, None));
        assert_eq!(parse_userinfo("garbage"), Usage::default());
    }

    #[test]
    fn parses_cloudflare_trace() {
        let trace = parse_trace("fl=1\nip=203.0.113.9\ncolo=AMS\nloc=NL\n");
        assert_eq!(trace, Trace { ip: "203.0.113.9".into(), country: "NL".into(), colo: "AMS".into() });
    }

    #[test]
    fn only_http_urls_are_fetchable() {
        assert!(require_http("https://example.com/sub").is_ok());
        assert!(require_http("file:///etc/passwd").is_err());
        assert!(require_http("ftp://example.com").is_err());
    }
}
