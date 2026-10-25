use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Vmess,
    Vless,
    Trojan,
    Shadowsocks,
}

impl Protocol {
    pub fn scheme(self) -> &'static str {
        match self {
            Protocol::Vmess => "vmess",
            Protocol::Vless => "vless",
            Protocol::Trojan => "trojan",
            Protocol::Shadowsocks => "ss",
        }
    }
}

/// One proxy endpoint. Transport/TLS fields use the same vocabulary as Xray-core
/// so config generation is a direct mapping.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub address: String,
    pub port: u16,
    /// UUID for vmess/vless, password for trojan/shadowsocks.
    pub secret: String,
    /// Shadowsocks cipher or vmess security.
    pub method: String,
    pub alter_id: u32,
    pub flow: String,
    /// tcp | ws | grpc | httpupgrade | xhttp | h2
    pub network: String,
    /// none | tls | reality
    pub security: String,
    pub sni: String,
    pub host: String,
    pub path: String,
    pub alpn: String,
    pub fingerprint: String,
    pub public_key: String,
    pub short_id: String,
    pub spider_x: String,
    pub header_type: String,
    pub service_name: String,
    pub allow_insecure: bool,
    pub subscription: Option<String>,
}

impl Default for Server {
    fn default() -> Self {
        Server {
            id: String::new(),
            name: String::new(),
            protocol: Protocol::Vless,
            address: String::new(),
            port: 443,
            secret: String::new(),
            method: String::new(),
            alter_id: 0,
            flow: String::new(),
            network: "tcp".into(),
            security: "none".into(),
            sni: String::new(),
            host: String::new(),
            path: String::new(),
            alpn: String::new(),
            fingerprint: String::new(),
            public_key: String::new(),
            short_id: String::new(),
            spider_x: String::new(),
            header_type: String::new(),
            service_name: String::new(),
            allow_insecure: false,
            subscription: None,
        }
    }
}

impl Server {
    /// Identity used to de-duplicate servers when importing or refreshing subscriptions.
    pub fn fingerprint_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            self.protocol.scheme(),
            self.address,
            self.port,
            self.secret,
            self.network,
            self.security,
            self.path,
            self.service_name
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub url: String,
    /// Fallback URLs tried in order when `url` fails (blocked, down).
    pub mirrors: Vec<String>,
    /// Refresh automatically every this many hours; 0 means manual only.
    pub auto_update_hours: u32,
    pub updated_at: Option<u64>,
    /// From the provider's `subscription-userinfo` header, in bytes / unix seconds.
    pub used: Option<u64>,
    pub total: Option<u64>,
    pub expire: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    /// Everything goes through the proxy.
    Global,
    /// LAN and private ranges go direct.
    BypassLan,
    /// LAN plus the configured region (geoip/geosite) go direct.
    BypassRegion,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub socks_port: u16,
    pub http_port: u16,
    pub allow_lan: bool,
    pub routing: RoutingMode,
    /// Two-letter region code, e.g. "ir" or "cn".
    pub region: String,
    pub dns: Vec<String>,
    pub core_path: String,
    pub auto_connect: bool,
    pub set_system_proxy: bool,
    pub download_via_proxy: bool,
    pub language: String,
    pub test_url: String,
    /// "dark" or "light".
    pub theme: String,
    /// Switch to the best working server when the active one keeps failing.
    pub auto_switch: bool,
    pub health_interval: u32,
    pub fail_threshold: u32,
    /// Split the TLS hello so DPI boxes cannot read the server name.
    pub fragment: bool,
    pub fragment_packets: String,
    pub fragment_length: String,
    pub fragment_interval: String,
    pub onboarded: bool,
}

impl Settings {
    /// True when `other` differs in something the running core was configured with.
    pub fn affects_core(&self, other: &Settings) -> bool {
        self.socks_port != other.socks_port
            || self.http_port != other.http_port
            || self.allow_lan != other.allow_lan
            || self.routing != other.routing
            || self.region != other.region
            || self.dns != other.dns
            || self.fragment != other.fragment
            || self.fragment_packets != other.fragment_packets
            || self.fragment_length != other.fragment_length
            || self.fragment_interval != other.fragment_interval
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            socks_port: 10808,
            http_port: 10809,
            allow_lan: false,
            routing: RoutingMode::BypassRegion,
            region: "ir".into(),
            dns: vec!["1.1.1.1".into(), "8.8.8.8".into()],
            core_path: String::new(),
            auto_connect: false,
            set_system_proxy: true,
            download_via_proxy: true,
            language: "en".into(),
            test_url: "http://www.gstatic.com/generate_204".into(),
            theme: "dark".into(),
            auto_switch: true,
            health_interval: 60,
            fail_threshold: 3,
            fragment: false,
            fragment_packets: "tlshello".into(),
            fragment_length: "100-200".into(),
            fragment_interval: "10-20".into(),
            onboarded: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Store {
    pub servers: Vec<Server>,
    pub subscriptions: Vec<Subscription>,
    pub settings: Settings,
    pub selected: Option<String>,
    /// Long-running reliability per server id, kept across restarts.
    pub health: std::collections::HashMap<String, crate::health::Health>,
}
