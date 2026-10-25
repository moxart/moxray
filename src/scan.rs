//! "Clean IP" scanner for servers behind Cloudflare: samples addresses from Cloudflare's
//! published ranges and keeps the ones that answer a real TLS request for the server's host
//! quickly. The server's own address can then be swapped for one of them.

use anyhow::{anyhow, bail, Result};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

/// Cloudflare's IPv4 ranges (https://www.cloudflare.com/ips-v4). They change rarely;
/// callers can pass their own list.
pub const CLOUDFLARE_V4: &[&str] = &[
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
];

pub const MAX_SAMPLES: usize = 500;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ScanHit {
    pub ip: String,
    pub ms: u32,
    pub colo: String,
}

/// A random address inside `cidr` (IPv4), avoiding the network and broadcast addresses of
/// anything larger than a /31. `rng` is any 128-bit random number.
pub fn random_ip_in(cidr: &str, rng: u128) -> Result<Ipv4Addr> {
    let (base, len) = cidr.trim().split_once('/').ok_or_else(|| anyhow!("expected a.b.c.d/len"))?;
    let base: Ipv4Addr = base.parse().map_err(|_| anyhow!("invalid address in {cidr}"))?;
    let len: u32 = len.parse().map_err(|_| anyhow!("invalid prefix in {cidr}"))?;
    if len > 32 {
        bail!("prefix must be 0-32");
    }
    let size = 1u64 << (32 - len);
    let network = u32::from(base) & u32::MAX.checked_shl(32 - len).unwrap_or(0);
    let offset = if size > 2 { 1 + (rng % u128::from(size - 2)) as u64 } else { (rng % u128::from(size)) as u64 };
    Ok(Ipv4Addr::from(network.wrapping_add(offset as u32)))
}

/// Pick `count` addresses spread over the ranges.
pub fn sample_ips(ranges: &[String], count: usize) -> Result<Vec<Ipv4Addr>> {
    if ranges.is_empty() {
        bail!("no ranges to scan");
    }
    let count = count.min(MAX_SAMPLES);
    let mut ips = Vec::with_capacity(count);
    for i in 0..count {
        let rng = uuid::Uuid::new_v4().as_u128();
        ips.push(random_ip_in(&ranges[i % ranges.len()], rng)?);
    }
    ips.sort();
    ips.dedup();
    Ok(ips)
}

/// One full HTTPS request for `host`, pinned to `ip`. Only Cloudflare answers `/cdn-cgi/trace`
/// with a `colo=` line, so a hit means this address really serves the zone.
pub async fn probe(host: &str, ip: Ipv4Addr) -> Option<ScanHit> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .resolve(host, SocketAddr::new(IpAddr::V4(ip), 443))
        .timeout(Duration::from_secs(4))
        .user_agent(concat!("MoxRay/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let started = Instant::now();
    let response = client.get(format!("https://{host}/cdn-cgi/trace")).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    let ms = started.elapsed().as_millis() as u32;
    let colo = text.lines().find_map(|l| l.strip_prefix("colo=")).map(str::trim)?;
    Some(ScanHit { ip: ip.to_string(), ms, colo: colo.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_ip_stays_inside_range() {
        for seed in [0u128, 1, 12345, u128::MAX, 987_654_321_987_654_321] {
            let ip = random_ip_in("104.16.0.0/13", seed).unwrap();
            let n = u32::from(ip);
            assert!(n > u32::from(Ipv4Addr::new(104, 16, 0, 0)));
            assert!(n < u32::from(Ipv4Addr::new(104, 23, 255, 255)));
        }
        let ip = random_ip_in("103.21.244.77/22", 5).unwrap();
        assert!(u32::from(ip) >= u32::from(Ipv4Addr::new(103, 21, 244, 1)));
        assert!(u32::from(ip) <= u32::from(Ipv4Addr::new(103, 21, 247, 254)));
    }

    #[test]
    fn handles_tiny_and_invalid_ranges() {
        assert_eq!(random_ip_in("1.2.3.4/32", 99).unwrap(), Ipv4Addr::new(1, 2, 3, 4));
        assert!(random_ip_in("1.2.3.4/33", 1).is_err());
        assert!(random_ip_in("nonsense", 1).is_err());
        assert!(random_ip_in("1.2.3/24", 1).is_err());
    }

    #[test]
    fn built_in_ranges_parse_and_sampling_is_capped() {
        let ranges: Vec<String> = CLOUDFLARE_V4.iter().map(|s| s.to_string()).collect();
        let ips = sample_ips(&ranges, 10_000).unwrap();
        assert!(!ips.is_empty() && ips.len() <= MAX_SAMPLES);
        assert!(sample_ips(&[], 5).is_err());
    }
}
