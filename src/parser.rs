//! Share-link parsing and generation (vmess://, vless://, trojan://, ss://) and
//! subscription body decoding.

use crate::model::{Protocol, Server};
use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use base64::Engine;
use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};
use std::collections::HashMap;
use url::Url;

pub fn b64_decode(input: &str) -> Result<Vec<u8>> {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let trimmed = cleaned.trim_end_matches('=');
    STANDARD_NO_PAD
        .decode(trimmed)
        .or_else(|_| URL_SAFE_NO_PAD.decode(trimmed))
        .map_err(|e| anyhow!("invalid base64: {e}"))
}

fn pct_decode(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

fn pct_encode(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

fn strip_brackets(host: &str) -> String {
    host.trim_start_matches('[').trim_end_matches(']').to_string()
}

fn truthy(s: &str) -> bool {
    matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

fn new_server(protocol: Protocol) -> Server {
    Server {
        id: uuid::Uuid::new_v4().to_string(),
        protocol,
        ..Server::default()
    }
}

pub fn parse_link(link: &str) -> Result<Server> {
    let link = link.trim();
    let (scheme, body) = link
        .split_once("://")
        .ok_or_else(|| anyhow!("not a share link"))?;
    match scheme.to_ascii_lowercase().as_str() {
        "vmess" => parse_vmess(body),
        "vless" => parse_url_style(link, Protocol::Vless),
        "trojan" => parse_url_style(link, Protocol::Trojan),
        "ss" => parse_ss(body),
        other => bail!("unsupported scheme: {other}"),
    }
}

fn parse_vmess(body: &str) -> Result<Server> {
    let payload = body.split('#').next().unwrap_or_default();
    let json: Value = serde_json::from_slice(&b64_decode(payload)?)
        .map_err(|e| anyhow!("invalid vmess payload: {e}"))?;
    let get = |key: &str| match json.get(key) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };

    let mut s = new_server(Protocol::Vmess);
    s.address = get("add");
    s.port = get("port").parse().map_err(|_| anyhow!("invalid port"))?;
    s.secret = get("id");
    if s.address.is_empty() || s.secret.is_empty() {
        bail!("vmess link is missing address or id");
    }
    s.alter_id = get("aid").parse().unwrap_or(0);
    s.method = match get("scy").as_str() {
        "" => "auto".into(),
        other => other.into(),
    };
    s.network = match get("net").as_str() {
        "" => "tcp".into(),
        "http" => "h2".into(),
        other => other.into(),
    };
    s.header_type = get("type");
    s.host = get("host");
    s.path = get("path");
    if s.network == "grpc" {
        s.service_name = std::mem::take(&mut s.path);
    }
    s.security = if get("tls") == "tls" { "tls".into() } else { "none".into() };
    s.sni = get("sni");
    s.alpn = get("alpn");
    s.fingerprint = get("fp");
    s.name = match get("ps").as_str() {
        "" => format!("{}:{}", s.address, s.port),
        ps => ps.to_string(),
    };
    Ok(s)
}

fn parse_url_style(link: &str, protocol: Protocol) -> Result<Server> {
    let url = Url::parse(link).map_err(|e| anyhow!("invalid link: {e}"))?;
    let q: HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let get = |key: &str| q.get(key).cloned().unwrap_or_default();

    let mut s = new_server(protocol);
    s.secret = pct_decode(url.username());
    s.address = strip_brackets(url.host_str().ok_or_else(|| anyhow!("missing host"))?);
    s.port = url.port().ok_or_else(|| anyhow!("missing port"))?;
    if s.secret.is_empty() || s.address.is_empty() {
        bail!("link is missing credentials or host");
    }

    s.network = match get("type").as_str() {
        "" => "tcp".into(),
        "http" => "h2".into(),
        other => other.into(),
    };
    s.header_type = get("headerType");
    s.security = match (get("security").as_str(), protocol) {
        ("", Protocol::Trojan) => "tls".into(),
        ("", _) => "none".into(),
        (other, _) => other.into(),
    };
    s.sni = match get("sni").as_str() {
        "" => get("peer"),
        sni => sni.to_string(),
    };
    s.host = get("host");
    s.path = get("path");
    s.service_name = get("serviceName");
    s.alpn = get("alpn");
    s.fingerprint = get("fp");
    s.public_key = get("pbk");
    s.short_id = get("sid");
    s.spider_x = get("spx");
    s.flow = get("flow");
    s.allow_insecure = truthy(&get("allowInsecure")) || truthy(&get("insecure"));
    s.name = match url.fragment().map(pct_decode).as_deref() {
        None | Some("") => format!("{}:{}", s.address, s.port),
        Some(name) => name.to_string(),
    };
    Ok(s)
}

fn parse_ss(body: &str) -> Result<Server> {
    let (rest, fragment) = match body.split_once('#') {
        Some((r, f)) => (r, Some(pct_decode(f))),
        None => (body, None),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((r, q)) => (r, q),
        None => (rest, ""),
    };
    if query.contains("plugin=") {
        bail!("shadowsocks plugins are not supported");
    }
    let rest = rest.trim_end_matches('/');

    let (userinfo, hostport) = match rest.rsplit_once('@') {
        Some((ui, hp)) => {
            let ui = pct_decode(ui);
            let ui = if ui.contains(':') {
                ui
            } else {
                String::from_utf8(b64_decode(&ui)?)?
            };
            (ui, hp.to_string())
        }
        None => {
            let decoded = String::from_utf8(b64_decode(rest)?)?;
            let (ui, hp) = decoded
                .rsplit_once('@')
                .ok_or_else(|| anyhow!("malformed ss link"))?;
            (ui.to_string(), hp.to_string())
        }
    };
    let (method, password) = userinfo
        .split_once(':')
        .ok_or_else(|| anyhow!("malformed ss credentials"))?;
    let (host, port) = hostport
        .trim_end_matches('/')
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("missing port"))?;

    let mut s = new_server(Protocol::Shadowsocks);
    s.method = method.to_string();
    s.secret = password.to_string();
    s.address = strip_brackets(host);
    s.port = port.parse().map_err(|_| anyhow!("invalid port"))?;
    s.name = match fragment.as_deref() {
        None | Some("") => format!("{}:{}", s.address, s.port),
        Some(name) => name.to_string(),
    };
    Ok(s)
}

/// Parse every share link found in `text`, one per line. Returns the servers and the
/// number of lines that could not be parsed.
pub fn parse_many(text: &str) -> (Vec<Server>, usize) {
    let mut servers = Vec::new();
    let mut failed = 0;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        match parse_link(line) {
            Ok(server) => servers.push(server),
            Err(_) => failed += 1,
        }
    }
    (servers, failed)
}

/// Subscription bodies are usually base64 of newline-separated links, but plain text
/// is accepted too.
pub fn decode_subscription(body: &str) -> (Vec<Server>, usize) {
    let body = body.trim();
    if body.contains("://") {
        return parse_many(body);
    }
    match b64_decode(body).ok().and_then(|b| String::from_utf8(b).ok()) {
        Some(text) => parse_many(&text),
        None => (Vec::new(), 1),
    }
}

pub fn to_link(s: &Server) -> String {
    match s.protocol {
        Protocol::Vmess => {
            let path = if s.network == "grpc" { &s.service_name } else { &s.path };
            let payload = json!({
                "v": "2",
                "ps": s.name,
                "add": s.address,
                "port": s.port.to_string(),
                "id": s.secret,
                "aid": s.alter_id.to_string(),
                "scy": if s.method.is_empty() { "auto" } else { &s.method },
                "net": s.network,
                "type": if s.header_type.is_empty() { "none" } else { &s.header_type },
                "host": s.host,
                "path": path,
                "tls": if s.security == "tls" { "tls" } else { "" },
                "sni": s.sni,
                "alpn": s.alpn,
                "fp": s.fingerprint,
            });
            format!("vmess://{}", STANDARD.encode(payload.to_string()))
        }
        Protocol::Vless | Protocol::Trojan => {
            let mut params: Vec<(&str, String)> = Vec::new();
            if s.protocol == Protocol::Vless {
                params.push(("encryption", "none".into()));
            }
            params.push(("security", s.security.clone()));
            params.push(("type", s.network.clone()));
            let optional = [
                ("headerType", &s.header_type),
                ("sni", &s.sni),
                ("host", &s.host),
                ("path", &s.path),
                ("serviceName", &s.service_name),
                ("alpn", &s.alpn),
                ("fp", &s.fingerprint),
                ("pbk", &s.public_key),
                ("sid", &s.short_id),
                ("spx", &s.spider_x),
                ("flow", &s.flow),
            ];
            for (key, value) in optional {
                if !value.is_empty() {
                    params.push((key, value.clone()));
                }
            }
            if s.allow_insecure {
                params.push(("allowInsecure", "1".into()));
            }
            let query = params
                .iter()
                .map(|(k, v)| format!("{k}={}", pct_encode(v)))
                .collect::<Vec<_>>()
                .join("&");
            format!(
                "{}://{}@{}:{}?{}#{}",
                s.protocol.scheme(),
                pct_encode(&s.secret),
                host_for_url(&s.address),
                s.port,
                query,
                pct_encode(&s.name)
            )
        }
        Protocol::Shadowsocks => {
            let userinfo = URL_SAFE_NO_PAD.encode(format!("{}:{}", s.method, s.secret));
            format!(
                "ss://{userinfo}@{}:{}#{}",
                host_for_url(&s.address),
                s.port,
                pct_encode(&s.name)
            )
        }
    }
}

fn host_for_url(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "11111111-2222-3333-4444-555555555555";

    fn without_id(mut s: Server) -> Server {
        s.id.clear();
        s
    }

    fn roundtrip(link: &str) -> Server {
        let first = parse_link(link).expect("first parse");
        let second = parse_link(&to_link(&first)).expect("second parse");
        assert_eq!(without_id(first.clone()), without_id(second));
        first
    }

    #[test]
    fn parses_vless_reality() {
        let link = format!(
            "vless://{UUID}@example.com:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.microsoft.com&fp=chrome&pbk=PUBKEY&sid=ab12&spx=%2F&type=tcp#My%20Node"
        );
        let s = roundtrip(&link);
        assert_eq!(s.protocol, Protocol::Vless);
        assert_eq!(s.address, "example.com");
        assert_eq!(s.port, 443);
        assert_eq!(s.secret, UUID);
        assert_eq!(s.security, "reality");
        assert_eq!(s.public_key, "PUBKEY");
        assert_eq!(s.short_id, "ab12");
        assert_eq!(s.spider_x, "/");
        assert_eq!(s.flow, "xtls-rprx-vision");
        assert_eq!(s.name, "My Node");
    }

    #[test]
    fn parses_vless_ws_tls_with_ipv6() {
        let link = format!(
            "vless://{UUID}@[2001:db8::1]:8443?security=tls&type=ws&host=cdn.example.com&path=%2Fws%3Fed%3D2048&sni=cdn.example.com&alpn=h2%2Chttp%2F1.1#v6"
        );
        let s = roundtrip(&link);
        assert_eq!(s.address, "2001:db8::1");
        assert_eq!(s.network, "ws");
        assert_eq!(s.path, "/ws?ed=2048");
        assert_eq!(s.alpn, "h2,http/1.1");
    }

    #[test]
    fn parses_vmess() {
        let payload = json!({
            "v": "2", "ps": "vm-node", "add": "vm.example.com", "port": 443, "id": UUID,
            "aid": "0", "scy": "auto", "net": "ws", "type": "none",
            "host": "vm.example.com", "path": "/vmws", "tls": "tls", "sni": "vm.example.com"
        });
        let link = format!("vmess://{}", STANDARD.encode(payload.to_string()));
        let s = roundtrip(&link);
        assert_eq!(s.protocol, Protocol::Vmess);
        assert_eq!(s.port, 443);
        assert_eq!(s.network, "ws");
        assert_eq!(s.security, "tls");
        assert_eq!(s.name, "vm-node");
    }

    #[test]
    fn vmess_grpc_path_is_service_name() {
        let payload = json!({
            "add": "g.example.com", "port": "443", "id": UUID, "net": "grpc",
            "path": "svc", "tls": "tls"
        });
        let link = format!("vmess://{}", STANDARD_NO_PAD.encode(payload.to_string()));
        let s = parse_link(&link).unwrap();
        assert_eq!(s.service_name, "svc");
        assert!(s.path.is_empty());
    }

    #[test]
    fn parses_trojan_defaults_to_tls() {
        let s = roundtrip("trojan://p%40ss@t.example.com:443?sni=t.example.com#trojan");
        assert_eq!(s.secret, "p@ss");
        assert_eq!(s.security, "tls");
    }

    #[test]
    fn parses_shadowsocks_sip002_and_legacy() {
        let userinfo = URL_SAFE_NO_PAD.encode("aes-256-gcm:secret");
        let s = roundtrip(&format!("ss://{userinfo}@1.2.3.4:8388#ss-node"));
        assert_eq!(s.method, "aes-256-gcm");
        assert_eq!(s.secret, "secret");
        assert_eq!(s.name, "ss-node");

        let legacy = STANDARD.encode("chacha20-ietf-poly1305:pw@5.6.7.8:9999");
        let s = parse_link(&format!("ss://{legacy}#old")).unwrap();
        assert_eq!(s.address, "5.6.7.8");
        assert_eq!(s.port, 9999);
        assert_eq!(s.method, "chacha20-ietf-poly1305");
    }

    #[test]
    fn rejects_unsupported_and_garbage() {
        assert!(parse_link("hysteria2://x@h:1").is_err());
        assert!(parse_link("not a link").is_err());
        assert!(parse_link("vless://@host:1").is_err());
        assert!(parse_link("ss://abc@h:1?plugin=obfs-local").is_err());
    }

    #[test]
    fn decodes_base64_subscription() {
        let lines = format!(
            "trojan://pw@a.example.com:443#a\nvless://{UUID}@b.example.com:443?security=tls#b\nbogus://x\n"
        );
        let body = STANDARD.encode(&lines);
        let (servers, failed) = decode_subscription(&body);
        assert_eq!(servers.len(), 2);
        assert_eq!(failed, 1);

        let (plain, _) = decode_subscription(&lines);
        assert_eq!(plain.len(), 2);
    }
}
