pub mod config;
pub mod core;
pub mod health;
pub mod model;
pub mod net;
pub mod parser;
pub mod scan;
pub mod store;
pub mod sysproxy;

use crate::core::{Core, LogSink};
use crate::health::Health;
use crate::model::{Server, Settings, Store, Subscription};
use futures::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};

pub struct AppState {
    store: Mutex<Store>,
    core: Core,
    /// True while we are the ones who switched the desktop proxy on.
    proxy_applied: AtomicBool,
    /// Set while a failover or "connect to best" is choosing a server.
    switching: AtomicBool,
    scan_cancel: AtomicBool,
}

type CmdResult<T> = Result<T, String>;

fn fail(err: impl std::fmt::Display) -> String {
    format!("{err:#}")
}

fn persist(state: &AppState) -> CmdResult<()> {
    store::save(&state.store.lock().unwrap()).map_err(fail)
}

fn notify(app: &AppHandle) {
    let _ = app.emit("state-changed", ());
}

fn log_sink(app: &AppHandle) -> LogSink {
    let app = app.clone();
    Arc::new(move |line: &str| {
        let _ = app.emit("core-log", line);
    })
}

fn log_line(app: &AppHandle, line: &str) {
    app.state::<AppState>().core.push_log_line(line);
    let _ = app.emit("core-log", line);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Record a probe result for a server and return its current failure streak.
fn record_health(state: &AppState, id: &str, ms: Option<u32>) -> u32 {
    let mut store = state.store.lock().unwrap();
    let health = store.health.entry(id.to_string()).or_default();
    health.record(ms, now_secs());
    health.streak_fail
}

/// SOCKS port to route app-initiated downloads through, if the proxy is up and allowed.
async fn download_proxy(state: &AppState) -> Option<u16> {
    let (allowed, port) = {
        let store = state.store.lock().unwrap();
        (store.settings.download_via_proxy, store.settings.socks_port)
    };
    (allowed && state.core.running().await).then_some(port)
}

fn binary_or_err(state: &AppState) -> CmdResult<std::path::PathBuf> {
    let settings = state.store.lock().unwrap().settings.clone();
    core::find_binary(&settings)
        .ok_or_else(|| "Xray-core was not found. Download it from the Core menu first.".to_string())
}

async fn do_connect(app: &AppHandle, id: Option<String>) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let (server, settings) = {
        let mut store = state.store.lock().unwrap();
        let id = id.or_else(|| store.selected.clone());
        let server = id
            .as_ref()
            .and_then(|id| store.servers.iter().find(|s| &s.id == id))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("select a server first"))?;
        store.selected = Some(server.id.clone());
        (server, store.settings.clone())
    };
    let binary = core::find_binary(&settings).ok_or_else(|| {
        anyhow::anyhow!("Xray-core was not found. Download it from the Core menu first.")
    })?;

    state.core.start(&binary, &server, &settings, log_sink(app)).await?;
    log_line(app, &format!("[moxray] connected to {}", server.name));

    if settings.set_system_proxy {
        match sysproxy::enable(settings.http_port, settings.socks_port) {
            Ok(()) => state.proxy_applied.store(true, Ordering::SeqCst),
            Err(e) => log_line(app, &format!("[moxray] could not set system proxy: {e:#}")),
        }
    }
    let _ = store::save(&state.store.lock().unwrap());
    notify(app);
    Ok(())
}

async fn do_disconnect(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.core.stop().await;
    if state.proxy_applied.swap(false, Ordering::SeqCst) {
        if let Err(e) = sysproxy::disable() {
            log_line(app, &format!("[moxray] could not clear system proxy: {e:#}"));
        }
    }
    log_line(app, "[moxray] disconnected");
    notify(app);
}

#[derive(Serialize)]
struct StateView {
    servers: Vec<Server>,
    subscriptions: Vec<Subscription>,
    settings: Settings,
    selected: Option<String>,
    running: bool,
    active: Option<String>,
    core_found: bool,
    health: HashMap<String, Health>,
}

#[tauri::command]
async fn get_state(state: State<'_, AppState>) -> CmdResult<StateView> {
    let store = state.store.lock().unwrap().clone();
    Ok(StateView {
        core_found: core::find_binary(&store.settings).is_some(),
        health: store.health,
        servers: store.servers,
        subscriptions: store.subscriptions,
        settings: store.settings,
        selected: store.selected,
        running: state.core.running().await,
        active: state.core.active_server(),
    })
}

#[derive(Serialize)]
struct ImportReport {
    added: usize,
    skipped: usize,
    failed: usize,
}

#[tauri::command]
async fn import_links(app: AppHandle, state: State<'_, AppState>, text: String) -> CmdResult<ImportReport> {
    let (servers, failed) = parser::decode_subscription(&text);
    let (added, skipped) = {
        let mut store = state.store.lock().unwrap();
        let result = store::add_unique(&mut store, servers);
        if store.selected.is_none() {
            store.selected = store.servers.first().map(|s| s.id.clone());
        }
        result
    };
    persist(&state)?;
    notify(&app);
    Ok(ImportReport { added, skipped, failed })
}

#[tauri::command]
async fn delete_servers(app: AppHandle, state: State<'_, AppState>, ids: Vec<String>) -> CmdResult<()> {
    let deleting_active = state.core.active_server().is_some_and(|a| ids.contains(&a));
    {
        let mut store = state.store.lock().unwrap();
        store.servers.retain(|s| !ids.contains(&s.id));
        store.health.retain(|id, _| !ids.contains(id));
        if store.selected.as_ref().is_some_and(|s| ids.contains(s)) {
            store.selected = store.servers.first().map(|s| s.id.clone());
        }
    }
    persist(&state)?;
    if deleting_active {
        do_disconnect(&app).await;
    }
    notify(&app);
    Ok(())
}

#[tauri::command]
async fn rename_server(app: AppHandle, state: State<'_, AppState>, id: String, name: String) -> CmdResult<()> {
    {
        let mut store = state.store.lock().unwrap();
        if let Some(server) = store.servers.iter_mut().find(|s| s.id == id) {
            server.name = name.trim().to_string();
        }
    }
    persist(&state)?;
    notify(&app);
    Ok(())
}

#[tauri::command]
async fn export_link(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    let store = state.store.lock().unwrap();
    store
        .servers
        .iter()
        .find(|s| s.id == id)
        .map(parser::to_link)
        .ok_or_else(|| "server not found".to_string())
}

#[tauri::command]
async fn select_server(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.store.lock().unwrap().selected = Some(id.clone());
    persist(&state)?;
    if state.core.running().await {
        do_connect(&app, Some(id)).await.map_err(fail)?;
    }
    notify(&app);
    Ok(())
}

#[tauri::command]
async fn connect(app: AppHandle, id: Option<String>) -> CmdResult<()> {
    do_connect(&app, id).await.map_err(fail)
}

#[tauri::command]
async fn disconnect(app: AppHandle) -> CmdResult<()> {
    do_disconnect(&app).await;
    Ok(())
}

#[tauri::command]
async fn save_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> CmdResult<()> {
    if settings.socks_port == settings.http_port {
        return Err("SOCKS and HTTP ports must differ".into());
    }
    if [settings.socks_port, settings.http_port].contains(&config::API_PORT) {
        return Err(format!("port {} is reserved for the stats API", config::API_PORT));
    }
    if settings.dns.is_empty() {
        return Err("add at least one DNS server".into());
    }
    if !(config::valid_range(&settings.fragment_length) && config::valid_range(&settings.fragment_interval)) {
        return Err("fragment length and interval must look like 100 or 100-200".into());
    }
    if settings.health_interval < 15 || settings.fail_threshold == 0 {
        return Err("health check interval must be at least 15 seconds and failures at least 1".into());
    }
    let (restart, proxy_toggled) = {
        let mut store = state.store.lock().unwrap();
        let restart = store.settings.affects_core(&settings);
        let proxy_toggled = store.settings.set_system_proxy != settings.set_system_proxy;
        store.settings = settings.clone();
        (restart, proxy_toggled)
    };
    persist(&state)?;
    if state.core.running().await {
        if restart {
            // Ports, routing, DNS or fragmenting changed: the core must be reconfigured.
            do_disconnect(&app).await;
            do_connect(&app, None).await.map_err(fail)?;
        } else if proxy_toggled {
            if settings.set_system_proxy {
                if sysproxy::enable(settings.http_port, settings.socks_port).is_ok() {
                    state.proxy_applied.store(true, Ordering::SeqCst);
                }
            } else if state.proxy_applied.swap(false, Ordering::SeqCst) {
                let _ = sysproxy::disable();
            }
        }
    }
    notify(&app);
    Ok(())
}

#[derive(Serialize)]
struct SubscriptionReport {
    added: usize,
    removed: usize,
    failed: usize,
}

async fn refresh_subscription(app: &AppHandle, sub_id: &str) -> anyhow::Result<SubscriptionReport> {
    let state = app.state::<AppState>();
    let urls = {
        let store = state.store.lock().unwrap();
        let sub = store
            .subscriptions
            .iter()
            .find(|s| s.id == sub_id)
            .ok_or_else(|| anyhow::anyhow!("subscription not found"))?;
        std::iter::once(sub.url.clone()).chain(sub.mirrors.iter().cloned()).collect::<Vec<_>>()
    };
    let fetched = net::fetch_subscription(&urls, download_proxy(&state).await).await?;
    let (servers, failed) = parser::decode_subscription(&fetched.body);
    if servers.is_empty() {
        anyhow::bail!("the subscription contained no supported servers");
    }

    let (added, removed) = {
        let mut store = state.store.lock().unwrap();
        let result = store::merge_subscription(&mut store, sub_id, servers);
        if let Some(sub) = store.subscriptions.iter_mut().find(|s| s.id == sub_id) {
            sub.updated_at = Some(now_secs());
            sub.used = fetched.usage.used;
            sub.total = fetched.usage.total;
            sub.expire = fetched.usage.expire;
        }
        if store.selected.is_none() {
            store.selected = store.servers.first().map(|s| s.id.clone());
        }
        result
    };
    store::save(&state.store.lock().unwrap())?;
    notify(app);
    Ok(SubscriptionReport { added, removed, failed })
}

#[tauri::command]
async fn add_subscription(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    url: String,
    mirrors: Vec<String>,
    auto_update_hours: u32,
) -> CmdResult<SubscriptionReport> {
    let id = uuid::Uuid::new_v4().to_string();
    let name = if name.trim().is_empty() { url.clone() } else { name.trim().to_string() };
    state.store.lock().unwrap().subscriptions.push(Subscription {
        id: id.clone(),
        name,
        url: url.trim().to_string(),
        mirrors: clean_urls(mirrors),
        auto_update_hours,
        ..Subscription::default()
    });
    match refresh_subscription(&app, &id).await {
        Ok(report) => Ok(report),
        Err(e) => {
            // Don't keep a subscription that never worked.
            state.store.lock().unwrap().subscriptions.retain(|s| s.id != id);
            Err(fail(e))
        }
    }
}

fn clean_urls(urls: Vec<String>) -> Vec<String> {
    urls.into_iter().map(|u| u.trim().to_string()).filter(|u| !u.is_empty()).collect()
}

#[tauri::command]
async fn edit_subscription(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: String,
    url: String,
    mirrors: Vec<String>,
    auto_update_hours: u32,
) -> CmdResult<()> {
    {
        let mut store = state.store.lock().unwrap();
        let sub = store
            .subscriptions
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| "subscription not found".to_string())?;
        sub.name = name.trim().to_string();
        sub.url = url.trim().to_string();
        sub.mirrors = clean_urls(mirrors);
        sub.auto_update_hours = auto_update_hours;
    }
    persist(&state)?;
    notify(&app);
    Ok(())
}

#[tauri::command]
async fn update_subscriptions(app: AppHandle, state: State<'_, AppState>, id: Option<String>) -> CmdResult<String> {
    let ids: Vec<String> = match id {
        Some(id) => vec![id],
        None => state.store.lock().unwrap().subscriptions.iter().map(|s| s.id.clone()).collect(),
    };
    let mut summary = Vec::new();
    for id in ids {
        let name = state
            .store
            .lock()
            .unwrap()
            .subscriptions
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        match refresh_subscription(&app, &id).await {
            Ok(r) => summary.push(format!("{name}: +{} -{}", r.added, r.removed)),
            Err(e) => summary.push(format!("{name}: {e:#}")),
        }
    }
    Ok(summary.join("\n"))
}

#[tauri::command]
async fn remove_subscription(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let removes_active = {
        let mut store = state.store.lock().unwrap();
        let active = state.core.active_server();
        let owned: Vec<String> = store
            .servers
            .iter()
            .filter(|s| s.subscription.as_deref() == Some(id.as_str()))
            .map(|s| s.id.clone())
            .collect();
        store.subscriptions.retain(|s| s.id != id);
        store.servers.retain(|s| !owned.contains(&s.id));
        if store.selected.as_ref().is_some_and(|s| owned.contains(s)) {
            store.selected = store.servers.first().map(|s| s.id.clone());
        }
        active.is_some_and(|a| owned.contains(&a))
    };
    persist(&state)?;
    if removes_active {
        do_disconnect(&app).await;
    }
    notify(&app);
    Ok(())
}

#[derive(Serialize, Clone)]
struct LatencyEvent {
    id: String,
    kind: &'static str,
    ms: Option<u32>,
    error: Option<String>,
}

fn servers_by_id(state: &AppState, ids: &[String]) -> Vec<Server> {
    let store = state.store.lock().unwrap();
    store.servers.iter().filter(|s| ids.contains(&s.id)).cloned().collect()
}

#[tauri::command]
async fn test_tcp(app: AppHandle, state: State<'_, AppState>, ids: Vec<String>) -> CmdResult<()> {
    futures::stream::iter(servers_by_id(&state, &ids))
        .for_each_concurrent(24, |server| {
            let app = app.clone();
            async move {
                let ms = net::tcp_ping(&server).await;
                let _ = app.emit("latency", LatencyEvent { id: server.id, kind: "tcp", ms, error: None });
            }
        })
        .await;
    Ok(())
}

#[tauri::command]
async fn test_real(app: AppHandle, state: State<'_, AppState>, ids: Vec<String>) -> CmdResult<()> {
    let binary = binary_or_err(&state)?;
    let settings = state.store.lock().unwrap().settings.clone();
    futures::stream::iter(servers_by_id(&state, &ids))
        .for_each_concurrent(4, |server| {
            let (app, binary, settings) = (app.clone(), binary.clone(), settings.clone());
            async move {
                let result = net::real_delay(&binary, &server, &settings).await;
                let (ms, error) = match result {
                    Ok(ms) => (Some(ms), None),
                    Err(e) => (None, Some(format!("{e:#}"))),
                };
                record_health(&app.state::<AppState>(), &server.id, ms);
                let _ = app.emit("latency", LatencyEvent { id: server.id, kind: "real", ms, error });
            }
        })
        .await;
    Ok(())
}

#[derive(Serialize)]
struct CoreInfo {
    path: Option<String>,
    version: Option<String>,
}

#[tauri::command]
async fn core_info(state: State<'_, AppState>) -> CmdResult<CoreInfo> {
    let settings = state.store.lock().unwrap().settings.clone();
    let Some(path) = core::find_binary(&settings) else {
        return Ok(CoreInfo { path: None, version: None });
    };
    Ok(CoreInfo {
        version: net::core_version(&path).await,
        path: Some(path.display().to_string()),
    })
}

#[tauri::command]
async fn download_core(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    let proxy = download_proxy(&state).await;
    log_line(&app, "[moxray] downloading Xray-core…");
    let tag = net::download_core(proxy).await.map_err(fail)?;
    log_line(&app, &format!("[moxray] installed Xray-core {tag}"));
    notify(&app);
    Ok(tag)
}

/// Probe the most promising servers (best health first) with throwaway cores and return
/// the fastest one that answered. Results are recorded in each server's health.
async fn pick_best(app: &AppHandle, exclude: Option<&str>, limit: usize) -> anyhow::Result<Option<Server>> {
    let state = app.state::<AppState>();
    let (candidates, settings) = {
        let store = state.store.lock().unwrap();
        let ids = health::rank(&store.servers, &store.health, exclude);
        let servers: Vec<Server> = ids
            .iter()
            .take(limit)
            .filter_map(|id| store.servers.iter().find(|s| &s.id == id).cloned())
            .collect();
        (servers, store.settings.clone())
    };
    if candidates.is_empty() {
        return Ok(None);
    }
    let binary = core::find_binary(&settings)
        .ok_or_else(|| anyhow::anyhow!("Xray-core was not found. Download it from the Core menu first."))?;

    let results: Mutex<Vec<(u32, Server)>> = Mutex::new(Vec::new());
    futures::stream::iter(candidates)
        .for_each_concurrent(4, |server| {
            let (binary, settings, results) = (&binary, &settings, &results);
            async move {
                let ms = net::real_delay(binary, &server, settings).await.ok();
                record_health(&app.state::<AppState>(), &server.id, ms);
                let _ = app.emit(
                    "latency",
                    LatencyEvent { id: server.id.clone(), kind: "real", ms, error: None },
                );
                if let Some(ms) = ms {
                    results.lock().unwrap().push((ms, server));
                }
            }
        })
        .await;
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(ms, _)| *ms);
    Ok(results.into_iter().next().map(|(_, server)| server))
}

#[tauri::command]
async fn connect_best(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    if state.switching.swap(true, Ordering::SeqCst) {
        return Err("already choosing a server".into());
    }
    let outcome = async {
        let best = pick_best(&app, None, 8)
            .await?
            .ok_or_else(|| anyhow::anyhow!("none of the tried servers answered"))?;
        do_connect(&app, Some(best.id.clone())).await?;
        Ok::<_, anyhow::Error>(best.name)
    }
    .await;
    state.switching.store(false, Ordering::SeqCst);
    notify(&app);
    outcome.map_err(fail)
}

#[tauri::command]
async fn save_server(app: AppHandle, state: State<'_, AppState>, mut server: Server) -> CmdResult<String> {
    server.name = server.name.trim().to_string();
    if server.name.is_empty() {
        server.name = format!("{}:{}", server.address.trim(), server.port);
    }
    server.address = server.address.trim().to_string();
    server.secret = server.secret.trim().to_string();
    config::validate_server(&server).map_err(fail)?;
    // A hand-edited server no longer matches what its subscription publishes.
    server.subscription = None;

    let id = {
        let mut store = state.store.lock().unwrap();
        match store.servers.iter_mut().find(|s| s.id == server.id && !server.id.is_empty()) {
            Some(existing) => *existing = server.clone(),
            None => {
                server.id = uuid::Uuid::new_v4().to_string();
                store.servers.push(server.clone());
            }
        }
        if store.selected.is_none() {
            store.selected = Some(server.id.clone());
        }
        server.id.clone()
    };
    persist(&state)?;
    if state.core.active_server().as_deref() == Some(id.as_str()) && state.core.running().await {
        do_connect(&app, Some(id.clone())).await.map_err(fail)?;
    }
    notify(&app);
    Ok(id)
}

#[tauri::command]
async fn duplicate_server(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<String> {
    let copy_id = {
        let mut store = state.store.lock().unwrap();
        let position = store
            .servers
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| "server not found".to_string())?;
        let mut copy = store.servers[position].clone();
        copy.id = uuid::Uuid::new_v4().to_string();
        copy.name = format!("{} (copy)", copy.name);
        copy.subscription = None;
        let copy_id = copy.id.clone();
        store.servers.insert(position + 1, copy);
        copy_id
    };
    persist(&state)?;
    notify(&app);
    Ok(copy_id)
}

#[tauri::command]
async fn check_config(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let binary = binary_or_err(&state)?;
    let (server, settings) = {
        let store = state.store.lock().unwrap();
        let server = store
            .servers
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .ok_or_else(|| "server not found".to_string())?;
        (server, store.settings.clone())
    };
    core::test_config(&binary, &server, &settings).await.map_err(fail)
}

#[tauri::command]
async fn link_qr(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    let link = {
        let store = state.store.lock().unwrap();
        store
            .servers
            .iter()
            .find(|s| s.id == id)
            .map(parser::to_link)
            .ok_or_else(|| "server not found".to_string())?
    };
    let code = qrcode::QrCode::new(link.as_bytes()).map_err(fail)?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(260, 260)
        .quiet_zone(true)
        .build())
}

#[derive(Serialize)]
struct TraceResult {
    trace: Option<net::Trace>,
    error: Option<String>,
}

impl From<anyhow::Result<net::Trace>> for TraceResult {
    fn from(result: anyhow::Result<net::Trace>) -> Self {
        match result {
            Ok(trace) => TraceResult { trace: Some(trace), error: None },
            Err(e) => TraceResult { trace: None, error: Some(format!("{e:#}")) },
        }
    }
}

#[derive(Serialize)]
struct IpReport {
    direct: TraceResult,
    proxied: Option<TraceResult>,
}

#[tauri::command]
async fn check_ip(state: State<'_, AppState>) -> CmdResult<IpReport> {
    let socks = state.store.lock().unwrap().settings.socks_port;
    let proxy_port = state.core.running().await.then_some(socks);
    let (direct, proxied) = tokio::join!(net::trace(None), async {
        match proxy_port {
            Some(port) => Some(net::trace(Some(port)).await),
            None => None,
        }
    });
    Ok(IpReport { direct: direct.into(), proxied: proxied.map(Into::into) })
}

#[derive(Serialize, Clone)]
struct ScanProgress {
    done: usize,
    total: usize,
}

#[tauri::command]
async fn scan_clean_ips(
    app: AppHandle,
    state: State<'_, AppState>,
    host: String,
    samples: usize,
    ranges: Option<Vec<String>>,
) -> CmdResult<Vec<scan::ScanHit>> {
    let host = host.trim().to_ascii_lowercase();
    let valid_host = !host.is_empty()
        && host.len() < 254
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && host.parse::<std::net::IpAddr>().is_err();
    if !valid_host {
        return Err("enter the server's domain name (not an IP address)".into());
    }
    let ranges = match ranges.map(clean_urls).filter(|r| !r.is_empty()) {
        Some(r) => r,
        None => scan::CLOUDFLARE_V4.iter().map(|s| s.to_string()).collect(),
    };
    let ips = scan::sample_ips(&ranges, samples.max(1)).map_err(fail)?;

    state.scan_cancel.store(false, Ordering::SeqCst);
    let total = ips.len();
    let done = AtomicUsize::new(0);
    let hits: Mutex<Vec<scan::ScanHit>> = Mutex::new(Vec::new());
    futures::stream::iter(ips)
        .for_each_concurrent(24, |ip| {
            let (app, host, cancel, hits, done) = (&app, &host, &state.scan_cancel, &hits, &done);
            async move {
                if cancel.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(hit) = scan::probe(host, ip).await {
                    let _ = app.emit("scan-hit", hit.clone());
                    hits.lock().unwrap().push(hit);
                }
                let finished = done.fetch_add(1, Ordering::SeqCst) + 1;
                if finished % 5 == 0 || finished == total {
                    let _ = app.emit("scan-progress", ScanProgress { done: finished, total });
                }
            }
        })
        .await;

    let mut hits = hits.into_inner().unwrap();
    hits.sort_by_key(|h| h.ms);
    hits.truncate(30);
    Ok(hits)
}

#[tauri::command]
fn cancel_scan(state: State<'_, AppState>) {
    state.scan_cancel.store(true, Ordering::SeqCst);
}

/// Copy a server with its address swapped for a scanned IP, keeping the original host name
/// for TLS (SNI) and the HTTP Host header so Cloudflare still routes it to the right site.
#[tauri::command]
async fn apply_clean_ip(app: AppHandle, state: State<'_, AppState>, id: String, ip: String) -> CmdResult<String> {
    ip.parse::<std::net::Ipv4Addr>().map_err(|_| "not a valid IPv4 address".to_string())?;
    let new_id = {
        let mut store = state.store.lock().unwrap();
        let position = store
            .servers
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| "server not found".to_string())?;
        let original = store.servers[position].clone();
        let mut copy = original.clone();
        copy.id = uuid::Uuid::new_v4().to_string();
        copy.name = format!("{} · {ip}", original.name);
        copy.subscription = None;
        if copy.sni.is_empty() {
            copy.sni = if original.host.is_empty() { original.address.clone() } else { original.host.clone() };
        }
        if copy.host.is_empty() {
            copy.host = original.address.clone();
        }
        copy.address = ip;
        let new_id = copy.id.clone();
        store.servers.insert(position + 1, copy);
        new_id
    };
    persist(&state)?;
    notify(&app);
    Ok(new_id)
}

#[derive(Serialize, Clone)]
struct HealthEvent {
    id: String,
    ms: Option<u32>,
}

/// One probe through the live proxy. After enough consecutive failures, and if allowed,
/// move to the best working alternative.
async fn health_check(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(active) = state.core.active_server() else { return };
    let (port, url, auto_switch, threshold) = {
        let store = state.store.lock().unwrap();
        let s = &store.settings;
        (s.socks_port, s.test_url.clone(), s.auto_switch, s.fail_threshold)
    };
    let ms = net::probe_via_proxy(port, &url).await;
    let streak = record_health(&state, &active, ms);
    let _ = app.emit("health", HealthEvent { id: active.clone(), ms });
    let _ = store::save(&state.store.lock().unwrap());

    if ms.is_none() && auto_switch && streak >= threshold {
        log_line(app, &format!("[moxray] {streak} failed checks in a row, looking for a better server…"));
        if state.switching.swap(true, Ordering::SeqCst) {
            return;
        }
        let outcome = pick_best(app, Some(&active), 6).await;
        match outcome {
            Ok(Some(best)) => match do_connect(app, Some(best.id.clone())).await {
                Ok(()) => log_line(app, &format!("[moxray] switched to {}", best.name)),
                Err(e) => log_line(app, &format!("[moxray] switch failed: {e:#}")),
            },
            Ok(None) => log_line(app, "[moxray] no other server answered; staying on the current one"),
            Err(e) => log_line(app, &format!("[moxray] could not look for another server: {e:#}")),
        }
        state.switching.store(false, Ordering::SeqCst);
        notify(app);
    }
}

/// Background work: periodic health checks and scheduled subscription refreshes.
fn spawn_monitor(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_probe = Instant::now();
        let mut last_attempt: HashMap<String, Instant> = HashMap::new();
        let mut ticker = tokio::time::interval(Duration::from_secs(10));
        loop {
            ticker.tick().await;
            let state = app.state::<AppState>();

            let due: Vec<String> = {
                let store = state.store.lock().unwrap();
                store
                    .subscriptions
                    .iter()
                    .filter(|s| s.auto_update_hours > 0)
                    .filter(|s| {
                        let age = now_secs().saturating_sub(s.updated_at.unwrap_or(0));
                        age >= u64::from(s.auto_update_hours) * 3600
                    })
                    .filter(|s| last_attempt.get(&s.id).is_none_or(|t| t.elapsed() > Duration::from_secs(1800)))
                    .map(|s| s.id.clone())
                    .collect()
            };
            for id in due {
                last_attempt.insert(id.clone(), Instant::now());
                match refresh_subscription(&app, &id).await {
                    Ok(r) => log_line(&app, &format!("[moxray] subscription updated (+{} -{})", r.added, r.removed)),
                    Err(e) => log_line(&app, &format!("[moxray] subscription update failed: {e:#}")),
                }
            }

            let interval = state.store.lock().unwrap().settings.health_interval.max(15);
            if last_probe.elapsed() >= Duration::from_secs(u64::from(interval)) && state.core.running().await {
                last_probe = Instant::now();
                health_check(&app).await;
            }
        }
    });
}

#[tauri::command]
fn get_logs(state: State<'_, AppState>) -> Vec<String> {
    state.core.logs()
}

#[derive(Serialize)]
struct Traffic {
    up: u64,
    down: u64,
}

#[tauri::command]
async fn get_traffic(state: State<'_, AppState>) -> CmdResult<Traffic> {
    if !state.core.running().await {
        return Ok(Traffic { up: 0, down: 0 });
    }
    let binary = binary_or_err(&state)?;
    let (up, down) = core::traffic(&binary).await.map_err(fail)?;
    Ok(Traffic { up, down })
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show MoxRay", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Connect / Disconnect", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &toggle, &quit])?;

    let mut tray = TrayIconBuilder::new().tooltip("MoxRay").menu(&menu);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id.as_ref() {
        "show" => show_main_window(app),
        "toggle" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if app.state::<AppState>().core.running().await {
                    do_disconnect(&app).await;
                } else if let Err(e) = do_connect(&app, None).await {
                    log_line(&app, &format!("[moxray] connect failed: {e:#}"));
                }
            });
        }
        "quit" => app.exit(0),
        _ => {}
    })
    .build(app)?;
    Ok(())
}

/// A lingering system proxy pointing at a dead port breaks the user's internet, so
/// terminate signals get the same cleanup as a normal quit.
#[cfg(unix)]
fn handle_signals(app: AppHandle) {
    use tokio::signal::unix::{signal, SignalKind};
    tauri::async_runtime::spawn(async move {
        let (Ok(mut term), Ok(mut int)) =
            (signal(SignalKind::terminate()), signal(SignalKind::interrupt()))
        else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
        app.exit(0);
    });
}

pub fn run() {
    let state = AppState {
        store: Mutex::new(store::load()),
        core: Core::new(),
        proxy_applied: AtomicBool::new(false),
        switching: AtomicBool::new(false),
        scan_cancel: AtomicBool::new(false),
    };

    let app = tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_state,
            import_links,
            delete_servers,
            rename_server,
            export_link,
            select_server,
            connect,
            disconnect,
            save_settings,
            add_subscription,
            update_subscriptions,
            remove_subscription,
            test_tcp,
            test_real,
            core_info,
            download_core,
            get_logs,
            get_traffic,
            connect_best,
            save_server,
            duplicate_server,
            check_config,
            link_qr,
            check_ip,
            scan_clean_ips,
            cancel_scan,
            apply_clean_ip,
            edit_subscription,
        ])
        .on_window_event(|window, event| {
            // Closing the window keeps the app alive in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            build_tray(app)?;
            #[cfg(unix)]
            handle_signals(app.handle().clone());
            spawn_monitor(app.handle().clone());

            let auto_connect = {
                let store = app.state::<AppState>().store.lock().unwrap().clone();
                store.settings.auto_connect && store.selected.is_some()
            };
            if auto_connect {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = do_connect(&handle, None).await {
                        log_line(&handle, &format!("[moxray] auto-connect failed: {e:#}"));
                    }
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build MoxRay");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            tauri::async_runtime::block_on(do_disconnect(handle));
        }
    });
}
