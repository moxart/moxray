# MoxRay

A desktop client for [Xray-core](https://github.com/XTLS/Xray-core) in the spirit of v2rayN, built with Tauri 2 (Rust backend, plain HTML/JS frontend, no Node build step). The look is the pomegranate theme of mox desktop.

## What it does

**Servers and subscriptions**
- Import `vmess://`, `vless://` (REALITY, flow), `trojan://` and `ss://` links, or a base64 subscription body; a clipboard offer appears when you copy links
- Full server editor with a "save and check" that asks Xray itself (`xray run -test`) to validate the config
- QR codes and share links for every server
- Subscriptions with mirror URLs (tried in order, through the proxy and directly), scheduled auto-update, and quota/expiry read from the provider's `subscription-userinfo` header

**Staying connected**
- Health checks through the live proxy; each server keeps a stability score across restarts
- Auto-switch: after N failed checks it tests the best candidates with throwaway cores and moves to the fastest one that works
- "Best server" button that does the same on demand
- TCP ping and real-delay tests, live traffic graph from Xray's stats API

**Getting past blocks**
- TLS hello fragmentation (Xray `fragment` through a dialer outbound)
- DNS presets including DNS-over-HTTPS
- Clean-IP scanner for Cloudflare-fronted servers: samples addresses, makes one pinned HTTPS request each, and copies your server with the winning IP while keeping SNI/Host
- Routing: global, bypass LAN, or bypass LAN plus your country (geoip/geosite)

**Desktop**
- System proxy on GNOME and KDE, always cleared on disconnect, quit and SIGINT/SIGTERM
- Tray icon, close-to-tray, English and Persian (RTL), dark and light theme
- Downloads Xray-core from the official release and verifies the published SHA-256 before installing
- Server data is stored with owner-only permissions

## Build and run

Needs Rust and the Tauri Linux dependencies (webkit2gtk-4.1, libappindicator).

```bash
cargo run              # dev
cargo test             # parser, config generator, health ranking, scanner, subscriptions
node scripts/check-i18n.mjs   # English/Persian strings stay in sync
cargo build --release
```

Data lives in `~/.config/moxray/` (`data.json`, downloaded core in `core/`).

## Not done yet

TUN mode and a kill switch, sing-box (hysteria2/tuic), Windows and macOS system proxy, encrypting stored credentials with the system keyring.
