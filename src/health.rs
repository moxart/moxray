//! Per-server reliability tracking and ranking, used for auto-switching and "connect to best".

use crate::model::Server;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Beyond this many samples, old results are scaled down so recent behaviour dominates.
const SAMPLE_CAP: u32 = 200;
/// Assumed delay for a server we have never measured.
const UNKNOWN_DELAY_MS: f64 = 600.0;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Health {
    pub ok: u32,
    pub fail: u32,
    pub streak_fail: u32,
    pub last_ms: Option<u32>,
    /// Exponential moving average of successful delays.
    pub ema_ms: Option<f64>,
    pub checked_at: u64,
}

impl Health {
    pub fn record(&mut self, ms: Option<u32>, now: u64) {
        self.checked_at = now;
        match ms {
            Some(ms) => {
                self.ok += 1;
                self.streak_fail = 0;
                self.last_ms = Some(ms);
                self.ema_ms = Some(match self.ema_ms {
                    Some(prev) => prev * 0.7 + f64::from(ms) * 0.3,
                    None => f64::from(ms),
                });
            }
            None => {
                self.fail += 1;
                self.streak_fail += 1;
                self.last_ms = None;
            }
        }
        if self.ok + self.fail > SAMPLE_CAP {
            self.ok = self.ok * 3 / 4;
            self.fail = self.fail * 3 / 4;
        }
    }

    /// Share of probes that worked, with a mild prior so one sample does not decide.
    pub fn reliability(&self) -> f64 {
        f64::from(self.ok + 1) / f64::from(self.ok + self.fail + 2)
    }

    /// Higher is better: reliable and fast. A server failing right now is heavily demoted.
    pub fn score(&self) -> f64 {
        let delay = self.ema_ms.unwrap_or(UNKNOWN_DELAY_MS);
        let base = self.reliability() * 1000.0 / (delay + 100.0);
        if self.streak_fail >= 2 {
            base * 0.1
        } else {
            base
        }
    }
}

/// Server ids from most to least promising, leaving out `exclude` (usually the active one).
pub fn rank(servers: &[Server], health: &HashMap<String, Health>, exclude: Option<&str>) -> Vec<String> {
    let mut scored: Vec<(f64, &str)> = servers
        .iter()
        .filter(|s| Some(s.id.as_str()) != exclude)
        .map(|s| (health.get(&s.id).cloned().unwrap_or_default().score(), s.id.as_str()))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().map(|(_, id)| id.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(id: &str) -> Server {
        Server { id: id.into(), ..Server::default() }
    }

    fn health(samples: &[Option<u32>]) -> Health {
        let mut h = Health::default();
        for (i, s) in samples.iter().enumerate() {
            h.record(*s, i as u64);
        }
        h
    }

    #[test]
    fn record_tracks_streaks_and_average() {
        let h = health(&[Some(100), Some(200), None, None]);
        assert_eq!((h.ok, h.fail, h.streak_fail), (2, 2, 2));
        assert_eq!(h.last_ms, None);
        assert!((h.ema_ms.unwrap() - 130.0).abs() < 1e-6);
        let recovered = health(&[None, None, Some(90)]);
        assert_eq!(recovered.streak_fail, 0);
    }

    #[test]
    fn ranks_fast_reliable_first_and_demotes_failing() {
        let servers = [server("fast"), server("slow"), server("unknown"), server("down"), server("active")];
        let mut map = HashMap::new();
        map.insert("fast".to_string(), health(&[Some(120); 10]));
        map.insert("slow".to_string(), health(&[Some(900); 10]));
        map.insert("down".to_string(), health(&[Some(100), Some(100), None, None]));
        map.insert("active".to_string(), health(&[Some(50); 10]));

        let order = rank(&servers, &map, Some("active"));
        assert_eq!(order, ["fast", "slow", "unknown", "down"]);
    }

    #[test]
    fn old_samples_decay() {
        let mut h = Health::default();
        for i in 0..400 {
            h.record(Some(100), i);
        }
        assert!(h.ok <= SAMPLE_CAP);
    }
}
