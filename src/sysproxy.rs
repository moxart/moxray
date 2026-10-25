//! Desktop system-proxy control. GNOME (gsettings) and KDE (kioslaverc) on Linux.

use anyhow::{bail, Result};

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::process::Command;

    fn run(program: &str, args: &[&str]) -> Result<()> {
        let status = Command::new(program).args(args).status()?;
        if !status.success() {
            bail!("{program} exited with {status}");
        }
        Ok(())
    }

    fn is_kde() -> bool {
        std::env::var("XDG_CURRENT_DESKTOP")
            .map(|d| d.to_ascii_uppercase().contains("KDE"))
            .unwrap_or(false)
    }

    fn kwrite() -> &'static str {
        if which("kwriteconfig6") {
            "kwriteconfig6"
        } else {
            "kwriteconfig5"
        }
    }

    fn which(program: &str) -> bool {
        Command::new("sh")
            .args(["-c", &format!("command -v {program}")])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn kde_set(key: &str, value: &str) -> Result<()> {
        run(
            kwrite(),
            &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", key, value],
        )
    }

    fn kde_reload() {
        let _ = Command::new("dbus-send")
            .args([
                "--type=signal",
                "/KIO/Scheduler",
                "org.kde.KIO.Scheduler.reparseSlaveConfiguration",
                "string:",
            ])
            .status();
    }

    pub fn enable(http_port: u16, socks_port: u16) -> Result<()> {
        if is_kde() {
            kde_set("ProxyType", "1")?;
            kde_set("httpProxy", &format!("http://127.0.0.1 {http_port}"))?;
            kde_set("httpsProxy", &format!("http://127.0.0.1 {http_port}"))?;
            kde_set("socksProxy", &format!("socks://127.0.0.1 {socks_port}"))?;
            kde_set("NoProxyFor", "localhost,127.0.0.0/8,::1,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16")?;
            kde_reload();
            return Ok(());
        }
        let http_port = http_port.to_string();
        let socks_port = socks_port.to_string();
        let set = |schema: &str, key: &str, value: &str| run("gsettings", &["set", schema, key, value]);
        for schema in ["org.gnome.system.proxy.http", "org.gnome.system.proxy.https"] {
            set(schema, "host", "127.0.0.1")?;
            set(schema, "port", &http_port)?;
        }
        set("org.gnome.system.proxy.socks", "host", "127.0.0.1")?;
        set("org.gnome.system.proxy.socks", "port", &socks_port)?;
        set(
            "org.gnome.system.proxy",
            "ignore-hosts",
            "['localhost', '127.0.0.0/8', '::1', '10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16']",
        )?;
        set("org.gnome.system.proxy", "mode", "manual")
    }

    pub fn disable() -> Result<()> {
        if is_kde() {
            kde_set("ProxyType", "0")?;
            kde_reload();
            return Ok(());
        }
        run("gsettings", &["set", "org.gnome.system.proxy", "mode", "none"])
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;

    pub fn enable(_http_port: u16, _socks_port: u16) -> Result<()> {
        bail!("system proxy is only implemented on Linux")
    }

    pub fn disable() -> Result<()> {
        bail!("system proxy is only implemented on Linux")
    }
}

pub fn enable(http_port: u16, socks_port: u16) -> Result<()> {
    imp::enable(http_port, socks_port)
}

pub fn disable() -> Result<()> {
    imp::disable()
}
