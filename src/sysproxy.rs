//! Desktop system-proxy control: GNOME (gsettings) and KDE (kioslaverc) on Linux,
//! `networksetup` on macOS, the Internet Settings registry key on Windows.

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

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::process::Command;

    fn networksetup(args: &[&str]) -> Result<String> {
        let output = Command::new("networksetup").args(args).output()?;
        if !output.status.success() {
            bail!(
                "networksetup {} failed: {}",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Enabled network services (Wi-Fi, Ethernet, ...). Disabled ones are prefixed with `*`.
    fn services() -> Result<Vec<String>> {
        Ok(networksetup(&["-listallnetworkservices"])?
            .lines()
            .skip(1)
            .filter(|line| !line.is_empty() && !line.starts_with('*'))
            .map(String::from)
            .collect())
    }

    pub fn enable(http_port: u16, socks_port: u16) -> Result<()> {
        let (http, socks) = (http_port.to_string(), socks_port.to_string());
        for service in services()? {
            networksetup(&["-setwebproxy", &service, "127.0.0.1", &http])?;
            networksetup(&["-setsecurewebproxy", &service, "127.0.0.1", &http])?;
            networksetup(&["-setsocksfirewallproxy", &service, "127.0.0.1", &socks])?;
            networksetup(&[
                "-setproxybypassdomains",
                &service,
                "localhost",
                "127.0.0.1",
                "::1",
                "10.0.0.0/8",
                "172.16.0.0/12",
                "192.168.0.0/16",
            ])?;
        }
        Ok(())
    }

    pub fn disable() -> Result<()> {
        for service in services()? {
            networksetup(&["-setwebproxystate", &service, "off"])?;
            networksetup(&["-setsecurewebproxystate", &service, "off"])?;
            networksetup(&["-setsocksfirewallproxystate", &service, "off"])?;
        }
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";

    fn hidden(program: &str) -> Command {
        let mut command = Command::new(program);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }

    fn reg_set(name: &str, kind: &str, value: &str) -> Result<()> {
        let status = hidden("reg")
            .args(["add", KEY, "/v", name, "/t", kind, "/d", value, "/f"])
            .status()?;
        if !status.success() {
            bail!("reg add {name} exited with {status}");
        }
        Ok(())
    }

    /// Tells running programs (browsers, WinINet clients) that the proxy settings changed.
    fn notify() {
        const SCRIPT: &str = r#"$sig='[DllImport("wininet.dll")] public static extern bool InternetSetOption(IntPtr h,int o,IntPtr b,int l);'; $w=Add-Type -MemberDefinition $sig -Name W -Namespace N -PassThru; $null=$w::InternetSetOption([IntPtr]::Zero,39,[IntPtr]::Zero,0); $null=$w::InternetSetOption([IntPtr]::Zero,37,[IntPtr]::Zero,0)"#;
        let _ = hidden("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .status();
    }

    pub fn enable(http_port: u16, socks_port: u16) -> Result<()> {
        reg_set(
            "ProxyServer",
            "REG_SZ",
            &format!("http=127.0.0.1:{http_port};https=127.0.0.1:{http_port};socks=127.0.0.1:{socks_port}"),
        )?;
        reg_set(
            "ProxyOverride",
            "REG_SZ",
            "localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.2*;172.30.*;172.31.*;192.168.*;<local>",
        )?;
        reg_set("ProxyEnable", "REG_DWORD", "1")?;
        notify();
        Ok(())
    }

    pub fn disable() -> Result<()> {
        reg_set("ProxyEnable", "REG_DWORD", "0")?;
        notify();
        Ok(())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    use super::*;

    pub fn enable(_http_port: u16, _socks_port: u16) -> Result<()> {
        bail!("system proxy is not supported on this platform")
    }

    pub fn disable() -> Result<()> {
        bail!("system proxy is not supported on this platform")
    }
}

pub fn enable(http_port: u16, socks_port: u16) -> Result<()> {
    imp::enable(http_port, socks_port)
}

pub fn disable() -> Result<()> {
    imp::disable()
}
