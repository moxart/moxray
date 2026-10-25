//! JSON persistence for servers, subscriptions and settings.

use crate::model::{Server, Store};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("moxray")
}

pub fn core_dir() -> PathBuf {
    data_dir().join("core")
}

fn store_path() -> PathBuf {
    data_dir().join("data.json")
}

pub fn load() -> Store {
    std::fs::read_to_string(store_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Server credentials live in this file, so it is readable by the owner only.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
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

/// Write via a temp file so a crash mid-write cannot corrupt the existing data.
pub fn save(store: &Store) -> Result<()> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).context("creating data directory")?;
    let tmp = dir.join("data.json.tmp");
    write_private(&tmp, &serde_json::to_vec_pretty(store)?)?;
    std::fs::rename(&tmp, store_path())?;
    Ok(())
}

/// Add servers that are not already present. Returns (added, skipped_duplicates).
pub fn add_unique(store: &mut Store, incoming: Vec<Server>) -> (usize, usize) {
    let mut known: HashSet<String> = store.servers.iter().map(Server::fingerprint_key).collect();
    let (mut added, mut skipped) = (0, 0);
    for server in incoming {
        if known.insert(server.fingerprint_key()) {
            store.servers.push(server);
            added += 1;
        } else {
            skipped += 1;
        }
    }
    (added, skipped)
}

/// Replace a subscription's servers with a freshly fetched set. Servers that are still
/// present keep their id (so the selection survives a refresh). Returns (added, removed).
pub fn merge_subscription(store: &mut Store, sub_id: &str, incoming: Vec<Server>) -> (usize, usize) {
    let mut seen = HashSet::new();
    let incoming: Vec<Server> = incoming
        .into_iter()
        .filter(|s| seen.insert(s.fingerprint_key()))
        .collect();

    let previous: HashMap<String, String> = store
        .servers
        .iter()
        .filter(|s| s.subscription.as_deref() == Some(sub_id))
        .map(|s| (s.fingerprint_key(), s.id.clone()))
        .collect();
    let removed = previous.keys().filter(|key| !seen.contains(*key)).count();

    store.servers.retain(|s| s.subscription.as_deref() != Some(sub_id));
    let mut added = 0;
    for mut server in incoming {
        match previous.get(&server.fingerprint_key()) {
            Some(id) => server.id = id.clone(),
            None => added += 1,
        }
        server.subscription = Some(sub_id.to_string());
        store.servers.push(server);
    }

    if let Some(selected) = &store.selected {
        if !store.servers.iter().any(|s| &s.id == selected) {
            store.selected = None;
        }
    }
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_link;

    fn server(host: &str) -> Server {
        parse_link(&format!("trojan://pw@{host}:443#{host}")).unwrap()
    }

    #[test]
    fn add_unique_skips_duplicates() {
        let mut store = Store::default();
        let (added, skipped) = add_unique(&mut store, vec![server("a.com"), server("b.com"), server("a.com")]);
        assert_eq!((added, skipped), (2, 1));
        let (added, skipped) = add_unique(&mut store, vec![server("a.com")]);
        assert_eq!((added, skipped), (0, 1));
        assert_eq!(store.servers.len(), 2);
    }

    #[test]
    fn merge_keeps_ids_and_prunes_removed() {
        let mut store = Store::default();
        let manual = server("manual.com");
        store.servers.push(manual.clone());
        merge_subscription(&mut store, "sub", vec![server("a.com"), server("b.com")]);
        let kept_id = store.servers.iter().find(|s| s.address == "a.com").unwrap().id.clone();
        let dropped_id = store.servers.iter().find(|s| s.address == "b.com").unwrap().id.clone();
        store.selected = Some(dropped_id);

        let (added, removed) = merge_subscription(&mut store, "sub", vec![server("a.com"), server("c.com")]);
        assert_eq!((added, removed), (1, 1));
        assert_eq!(store.servers.iter().find(|s| s.address == "a.com").unwrap().id, kept_id);
        assert!(store.servers.iter().any(|s| s.id == manual.id), "manual servers are untouched");
        assert!(!store.servers.iter().any(|s| s.address == "b.com"));
        assert_eq!(store.selected, None, "selection of a removed server is cleared");
    }
}
