//! Xray-core configuration generation.

use crate::model::{Protocol, RoutingMode, Server, Settings};
use anyhow::{bail, Result};
use serde_json::{json, Map, Value};

/// Local gRPC port Xray's stats API listens on (loopback only).
pub const API_PORT: u16 = 10085;

fn non_empty(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}

fn csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

fn proxy_outbound(s: &Server) -> Result<Value> {
    let (name, settings) = match s.protocol {
        Protocol::Vmess => (
            "vmess",
            json!({"vnext": [{
                "address": s.address,
                "port": s.port,
                "users": [{
                    "id": s.secret,
                    "alterId": s.alter_id,
                    "security": non_empty(&s.method).unwrap_or("auto"),
                }],
            }]}),
        ),
        Protocol::Vless => {
            let mut user = json!({"id": s.secret, "encryption": "none"});
            if let Some(flow) = non_empty(&s.flow) {
                user["flow"] = json!(flow);
            }
            (
                "vless",
                json!({"vnext": [{"address": s.address, "port": s.port, "users": [user]}]}),
            )
        }
        Protocol::Trojan => (
            "trojan",
            json!({"servers": [{"address": s.address, "port": s.port, "password": s.secret}]}),
        ),
        Protocol::Shadowsocks => (
            "shadowsocks",
            json!({"servers": [{
                "address": s.address,
                "port": s.port,
                "method": s.method,
                "password": s.secret,
            }]}),
        ),
    };
    Ok(json!({
        "tag": "proxy",
        "protocol": name,
        "settings": settings,
        "streamSettings": stream_settings(s)?,
    }))
}

fn stream_settings(s: &Server) -> Result<Value> {
    let network = match s.network.as_str() {
        "" | "tcp" | "raw" => "tcp",
        "ws" => "ws",
        "grpc" => "grpc",
        "httpupgrade" => "httpupgrade",
        "xhttp" | "splithttp" => "xhttp",
        "h2" => "h2",
        other => bail!("transport '{other}' is not supported"),
    };
    let mut out = Map::new();
    out.insert("network".into(), json!(network));

    match s.security.as_str() {
        "" | "none" => {
            out.insert("security".into(), json!("none"));
        }
        "tls" => {
            let mut tls = Map::new();
            if let Some(name) = non_empty(&s.sni).or(non_empty(&s.host)) {
                tls.insert("serverName".into(), json!(name));
            }
            if s.allow_insecure {
                tls.insert("allowInsecure".into(), json!(true));
            }
            if !csv(&s.alpn).is_empty() {
                tls.insert("alpn".into(), json!(csv(&s.alpn)));
            }
            if let Some(fp) = non_empty(&s.fingerprint) {
                tls.insert("fingerprint".into(), json!(fp));
            }
            out.insert("security".into(), json!("tls"));
            out.insert("tlsSettings".into(), Value::Object(tls));
        }
        "reality" => {
            if s.public_key.is_empty() {
                bail!("reality server is missing its public key");
            }
            out.insert("security".into(), json!("reality"));
            out.insert(
                "realitySettings".into(),
                json!({
                    "serverName": s.sni,
                    "fingerprint": non_empty(&s.fingerprint).unwrap_or("chrome"),
                    "publicKey": s.public_key,
                    "shortId": s.short_id,
                    "spiderX": s.spider_x,
                }),
            );
        }
        other => bail!("security '{other}' is not supported"),
    }

    let path = non_empty(&s.path).unwrap_or("/");
    match network {
        "ws" => {
            let mut ws = json!({"path": path});
            if let Some(host) = non_empty(&s.host) {
                ws["headers"] = json!({"Host": host});
            }
            out.insert("wsSettings".into(), ws);
        }
        "grpc" => {
            out.insert("grpcSettings".into(), json!({"serviceName": s.service_name}));
        }
        "httpupgrade" => {
            out.insert(
                "httpupgradeSettings".into(),
                json!({"path": path, "host": s.host}),
            );
        }
        "xhttp" => {
            out.insert("xhttpSettings".into(), json!({"path": path, "host": s.host}));
        }
        "h2" => {
            out.insert(
                "httpSettings".into(),
                json!({"path": path, "host": csv(&s.host)}),
            );
        }
        "tcp" if s.header_type == "http" => {
            out.insert(
                "tcpSettings".into(),
                json!({"header": {
                    "type": "http",
                    "request": {"path": csv(path), "headers": {"Host": csv(&s.host)}},
                }}),
            );
        }
        _ => {}
    }
    Ok(Value::Object(out))
}

fn region_rules(region: &str) -> Vec<Value> {
    let region = region.trim().to_ascii_lowercase();
    if region.is_empty() {
        return Vec::new();
    }
    let mut rules = vec![json!({
        "type": "field",
        "ip": [format!("geoip:{region}")],
        "outboundTag": "direct",
    })];
    let geosite = match region.as_str() {
        "cn" => Some("geosite:cn".to_string()),
        "ir" => Some("geosite:category-ir".to_string()),
        _ => None,
    };
    if let Some(site) = geosite {
        rules.push(json!({"type": "field", "domain": [site], "outboundTag": "direct"}));
    }
    rules
}

fn sniffing() -> Value {
    json!({"enabled": true, "destOverride": ["http", "tls", "quic"], "routeOnly": true})
}

/// The proxy outbound, followed by the fragmenting dialer it routes through when enabled.
fn proxy_chain(server: &Server, settings: &Settings) -> Result<Vec<Value>> {
    let mut proxy = proxy_outbound(server)?;
    let mut chain = Vec::new();
    if settings.fragment && matches!(server.security.as_str(), "tls" | "reality") {
        proxy["streamSettings"]["sockopt"] = json!({"dialerProxy": "fragment"});
        chain.push(json!({
            "tag": "fragment",
            "protocol": "freedom",
            "settings": {"fragment": {
                "packets": settings.fragment_packets,
                "length": settings.fragment_length,
                "interval": settings.fragment_interval,
            }},
        }));
    }
    chain.insert(0, proxy);
    Ok(chain)
}

/// Full client config: SOCKS + HTTP inbounds, routing, DNS and the loopback stats API.
pub fn generate(server: &Server, settings: &Settings) -> Result<Value> {
    let listen = if settings.allow_lan { "0.0.0.0" } else { "127.0.0.1" };

    let mut rules = vec![json!({"type": "field", "inboundTag": ["api"], "outboundTag": "api"})];
    if settings.routing != RoutingMode::Global {
        rules.push(json!({"type": "field", "ip": ["geoip:private"], "outboundTag": "direct"}));
    }
    if settings.routing == RoutingMode::BypassRegion {
        rules.extend(region_rules(&settings.region));
    }

    let mut outbounds = proxy_chain(server, settings)?;
    outbounds.push(json!({"tag": "direct", "protocol": "freedom"}));
    outbounds.push(json!({"tag": "block", "protocol": "blackhole"}));

    Ok(json!({
        "log": {"loglevel": "warning"},
        "stats": {},
        "api": {"tag": "api", "services": ["StatsService"]},
        "policy": {"system": {"statsOutboundUplink": true, "statsOutboundDownlink": true}},
        "dns": {"servers": settings.dns, "queryStrategy": "UseIP"},
        "inbounds": [
            {
                "tag": "socks", "listen": listen, "port": settings.socks_port,
                "protocol": "socks", "settings": {"udp": true, "auth": "noauth"},
                "sniffing": sniffing(),
            },
            {
                "tag": "http", "listen": listen, "port": settings.http_port,
                "protocol": "http", "settings": {}, "sniffing": sniffing(),
            },
            {
                "tag": "api", "listen": "127.0.0.1", "port": API_PORT,
                "protocol": "dokodemo-door", "settings": {"address": "127.0.0.1"},
            },
        ],
        "outbounds": outbounds,
        "routing": {"domainStrategy": "IPIfNonMatch", "rules": rules},
    }))
}

/// Checks that a server can be turned into a working outbound before it is saved.
pub fn validate_server(s: &Server) -> Result<()> {
    if s.address.trim().is_empty() {
        bail!("address is required");
    }
    if s.port == 0 {
        bail!("port must be between 1 and 65535");
    }
    if s.secret.trim().is_empty() {
        bail!(match s.protocol {
            Protocol::Vmess | Protocol::Vless => "UUID is required",
            _ => "password is required",
        });
    }
    if matches!(s.protocol, Protocol::Vmess | Protocol::Vless)
        && uuid::Uuid::parse_str(s.secret.trim()).is_err()
    {
        bail!("UUID is not valid");
    }
    if s.protocol == Protocol::Shadowsocks && s.method.trim().is_empty() {
        bail!("cipher is required");
    }
    proxy_outbound(s).map(|_| ())
}

/// Accepts "100" or "100-200", the forms Xray's fragment options take.
pub fn valid_range(value: &str) -> bool {
    let mut parts = value.split('-');
    let number = |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    let (first, second, rest) = (parts.next(), parts.next(), parts.next());
    number(first) && rest.is_none() && (second.is_none() || number(second))
}

/// Throwaway config used to measure a server's real round-trip delay.
pub fn generate_probe(server: &Server, settings: &Settings, socks_port: u16) -> Result<Value> {
    Ok(json!({
        "log": {"loglevel": "none"},
        "inbounds": [{
            "tag": "probe", "listen": "127.0.0.1", "port": socks_port,
            "protocol": "socks", "settings": {"udp": false, "auth": "noauth"},
        }],
        "outbounds": proxy_chain(server, settings)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_link;

    const UUID: &str = "11111111-2222-3333-4444-555555555555";

    fn server(link: &str) -> Server {
        parse_link(link).unwrap()
    }

    #[test]
    fn reality_outbound() {
        let s = server(&format!(
            "vless://{UUID}@h.example.com:443?security=reality&sni=www.microsoft.com&pbk=PK&sid=ab&flow=xtls-rprx-vision&type=tcp#n"
        ));
        let cfg = generate(&s, &Settings::default()).unwrap();
        let out = &cfg["outbounds"][0];
        assert_eq!(out["protocol"], "vless");
        assert_eq!(out["settings"]["vnext"][0]["users"][0]["flow"], "xtls-rprx-vision");
        assert_eq!(out["streamSettings"]["security"], "reality");
        assert_eq!(out["streamSettings"]["realitySettings"]["publicKey"], "PK");
        assert_eq!(out["streamSettings"]["realitySettings"]["fingerprint"], "chrome");
    }

    #[test]
    fn websocket_tls_outbound() {
        let s = server(&format!(
            "vless://{UUID}@h.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Fws&alpn=h2%2Chttp%2F1.1&allowInsecure=1#n"
        ));
        let stream = stream_settings(&s).unwrap();
        assert_eq!(stream["wsSettings"]["path"], "/ws");
        assert_eq!(stream["wsSettings"]["headers"]["Host"], "cdn.example.com");
        assert_eq!(stream["tlsSettings"]["serverName"], "cdn.example.com");
        assert_eq!(stream["tlsSettings"]["alpn"], json!(["h2", "http/1.1"]));
        assert_eq!(stream["tlsSettings"]["allowInsecure"], true);
    }

    #[test]
    fn trojan_and_shadowsocks_outbounds() {
        let t = proxy_outbound(&server("trojan://pw@t.example.com:443#t")).unwrap();
        assert_eq!(t["settings"]["servers"][0]["password"], "pw");
        assert_eq!(t["streamSettings"]["security"], "tls");

        let ss = proxy_outbound(&server("ss://YWVzLTI1Ni1nY206cHc@1.2.3.4:8388#s")).unwrap();
        assert_eq!(ss["protocol"], "shadowsocks");
        assert_eq!(ss["settings"]["servers"][0]["method"], "aes-256-gcm");
    }

    #[test]
    fn routing_modes() {
        let s = server("trojan://pw@t.example.com:443#t");
        let rule_count = |mode, region: &str| {
            let settings = Settings { routing: mode, region: region.into(), ..Settings::default() };
            generate(&s, &settings).unwrap()["routing"]["rules"]
                .as_array()
                .unwrap()
                .len()
        };
        assert_eq!(rule_count(RoutingMode::Global, "ir"), 1);
        assert_eq!(rule_count(RoutingMode::BypassLan, "ir"), 2);
        assert_eq!(rule_count(RoutingMode::BypassRegion, "ir"), 4);
        // Regions without a geosite list only get the geoip rule.
        assert_eq!(rule_count(RoutingMode::BypassRegion, "de"), 3);
    }

    #[test]
    fn allow_lan_changes_listen_address() {
        let s = server("trojan://pw@t.example.com:443#t");
        let lan = Settings { allow_lan: true, ..Settings::default() };
        assert_eq!(generate(&s, &lan).unwrap()["inbounds"][0]["listen"], "0.0.0.0");
        assert_eq!(generate(&s, &Settings::default()).unwrap()["inbounds"][0]["listen"], "127.0.0.1");
        // The stats API must never be exposed to the LAN.
        assert_eq!(generate(&s, &lan).unwrap()["inbounds"][2]["listen"], "127.0.0.1");
    }

    #[test]
    fn fragment_adds_dialer_outbound_only_for_tls() {
        let tls = server("trojan://pw@t.example.com:443#t");
        let plain = server("ss://YWVzLTI1Ni1nY206cHc@1.2.3.4:8388#s");
        let on = Settings { fragment: true, ..Settings::default() };

        let cfg = generate(&tls, &on).unwrap();
        assert_eq!(cfg["outbounds"][0]["streamSettings"]["sockopt"]["dialerProxy"], "fragment");
        let fragment = cfg["outbounds"].as_array().unwrap().iter().find(|o| o["tag"] == "fragment").unwrap();
        assert_eq!(fragment["settings"]["fragment"]["packets"], "tlshello");

        let cfg = generate(&plain, &on).unwrap();
        assert!(cfg["outbounds"].as_array().unwrap().iter().all(|o| o["tag"] != "fragment"));
        let off = generate(&tls, &Settings::default()).unwrap();
        assert!(off["outbounds"].as_array().unwrap().iter().all(|o| o["tag"] != "fragment"));
    }

    #[test]
    fn validates_servers_and_ranges() {
        let mut s = server(&format!("vless://{UUID}@h.example.com:443?security=tls#n"));
        assert!(validate_server(&s).is_ok());
        s.secret = "not-a-uuid".into();
        assert!(validate_server(&s).is_err());
        s.secret = UUID.into();
        s.address = "  ".into();
        assert!(validate_server(&s).is_err());

        assert!(valid_range("100"));
        assert!(valid_range("10-20"));
        assert!(!valid_range("10-"));
        assert!(!valid_range("a-b"));
        assert!(!valid_range("1-2-3"));
        assert!(!valid_range(""));
    }

    #[test]
    fn rejects_unsupported_transport_and_incomplete_reality() {
        let mut s = server("trojan://pw@t.example.com:443#t");
        s.network = "kcp".into();
        assert!(generate(&s, &Settings::default()).is_err());

        let mut r = server(&format!("vless://{UUID}@h.example.com:443?security=reality#n"));
        r.public_key.clear();
        assert!(generate(&r, &Settings::default()).is_err());
    }
}
