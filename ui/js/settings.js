// Settings. Every control saves as soon as it changes; no Save button to forget.

import { h, icon, toast, spinner } from "./dom.js";
import { t } from "./i18n.js";
import { S, attempt, patchSettings, refresh } from "./api.js";

const DNS_PRESETS = [
  ["Cloudflare", "1.1.1.1"],
  ["Google", "8.8.8.8"],
  ["Quad9", "9.9.9.9"],
  ["Cloudflare DoH", "https://1.1.1.1/dns-query"],
  ["Google DoH", "https://8.8.8.8/dns-query"],
];

export function mountSettings(container) {
  const s = () => S.state.settings;

  /** Save a patch; on failure put the real values back on screen. */
  const save = async (patch) => {
    try {
      await patchSettings(patch);
    } catch {
      render();
    }
  };

  const row = (label, hint, control) =>
    h("div", { class: "setting" },
      h("div", { class: "setting-text" }, h("div", { class: "setting-label" }, label), hint ? h("div", { class: "hint" }, hint) : null),
      h("div", { class: "setting-control" }, control));

  const toggle = (key, onChange) =>
    h("label", { class: "switch" },
      h("input", { type: "checkbox", checked: s()[key], onchange: (e) => (onChange ?? ((v) => save({ [key]: v })))(e.target.checked) }),
      h("span", { class: "track" }));

  const number = (key, { min = 1, max = 65535 } = {}) =>
    h("input", {
      class: "input narrow", type: "number", min, max, value: s()[key], dir: "ltr",
      onchange: (e) => save({ [key]: Number(e.target.value) }),
    });

  const text = (key, { placeholder = "", wide = false } = {}) =>
    h("input", {
      class: `input ${wide ? "" : "narrow"}`, type: "text", value: s()[key], placeholder, dir: "ltr", spellcheck: false,
      onchange: (e) => save({ [key]: e.target.value.trim() }),
    });

  const select = (key, options, convert = (v) => v) =>
    h("select", { class: "input select-auto", onchange: (e) => save({ [key]: convert(e.target.value) }) },
      options.map(([value, label]) => h("option", { value, selected: String(s()[key]) === String(value) }, label)));

  const segmented = (key, options) =>
    h("div", { class: "segmented", role: "radiogroup" }, options.map(([value, label, iconName]) =>
      h("button", {
        class: s()[key] === value ? "on" : "", role: "radio", "aria-checked": s()[key] === value,
        onclick: () => save({ [key]: value }),
      }, iconName ? icon(iconName, 15) : null, label)));

  const section = (title, ...rows) =>
    h("section", { class: "card settings-card" }, h("h3", {}, title), ...rows);

  const dnsEditor = () => {
    const input = h("input", {
      class: "input", type: "text", dir: "ltr", spellcheck: false, value: s().dns.join(", "),
      onchange: (e) => save({ dns: e.target.value.split(",").map((d) => d.trim()).filter(Boolean) }),
    });
    const chips = h("div", { class: "chips" }, DNS_PRESETS.map(([label, value]) => {
      const on = s().dns.includes(value);
      return h("button", {
        class: `chip ${on ? "on" : ""}`, title: value,
        onclick: () => save({ dns: on ? s().dns.filter((d) => d !== value) : [...s().dns, value] }),
      }, on ? icon("check", 12) : null, label);
    }));
    return h("div", { class: "stack-sm wide-control" }, input, chips);
  };

  const coreBlock = () => {
    const box = h("div", { class: "stack-sm wide-control" }, spinner());
    attempt("core_info", null, { path: null, version: null }).then((info) => {
      const button = h("button", { class: "btn btn-soft" }, icon("download", 16), info.path ? t("update") : t("download"));
      const status = h("span", { class: "muted small" });
      button.onclick = async () => {
        button.disabled = true;
        status.textContent = t("coreDownloading");
        const tag = await attempt("download_core");
        button.disabled = false;
        status.textContent = tag ? t("coreInstalled", { tag }) : "";
        if (tag) { await refresh(); render(); }
      };
      box.replaceChildren(
        h("div", { class: "facts-inline" },
          h("span", { class: "muted" }, t("version")), h("b", { dir: "ltr" }, info.version ?? t("coreNone"))),
        info.path ? h("code", { class: "path", dir: "ltr" }, info.path) : null,
        h("div", { class: "inline" }, button, status));
    });
    return box;
  };

  function render() {
    const st = s();
    container.replaceChildren(
      h("header", { class: "view-head" }, h("h1", {}, t("nav_settings"))),
      h("div", { class: "stack" },
        section(t("general"),
          row(t("language"), null, segmented("language", [["en", "English"], ["fa", "فارسی"]])),
          row(t("theme"), null, segmented("theme", [["dark", t("dark"), "moon"], ["light", t("light"), "sun"]])),
          row(t("autoConnect"), t("autoConnectHint"), toggle("auto_connect")),
          row(t("sysProxy"), t("sysProxyHint"), toggle("set_system_proxy"))),
        section(t("reliability"),
          row(t("autoSwitch"), t("autoSwitchHint"), toggle("auto_switch")),
          row(t("healthInterval"), t("healthIntervalHint"),
            select("health_interval", [[30, "30 s"], [60, "1 min"], [120, "2 min"], [300, "5 min"]], Number)),
          row(t("failThreshold"), t("failThresholdHint"),
            select("fail_threshold", [[2, "2"], [3, "3"], [5, "5"]], Number))),
        section(t("antiCensorship"),
          row(t("fragment"), t("fragmentHint"), toggle("fragment")),
          ...(st.fragment
            ? [
                row(t("fragmentLength"), t("fragmentLengthHint"), text("fragment_length", { placeholder: "100-200" })),
                row(t("fragmentInterval"), null, text("fragment_interval", { placeholder: "10-20" })),
                row(t("fragmentPackets"), null, select("fragment_packets", [["tlshello", "tlshello"], ["1-3", "1-3"]])),
              ]
            : []),
          row(t("dns"), t("dnsHint"), dnsEditor())),
        section(t("network"),
          row(t("socksPort"), null, number("socks_port")),
          row(t("httpPort"), null, number("http_port")),
          row(t("allowLan"), t("allowLanHint"), toggle("allow_lan")),
          row(t("routing"), null, select("routing", [
            ["global", t("routingGlobal")], ["bypass_lan", t("routingLan")], ["bypass_region", t("routingRegion")],
          ])),
          ...(st.routing === "bypass_region"
            ? [row(t("region"), t("regionHint"), text("region", { placeholder: "ir" }))]
            : [])),
        section(t("core"),
          row(t("coreFile"), t("coreFileHint"), coreBlock()),
          row(t("corePath"), t("corePathHint"), text("core_path", { placeholder: "/usr/local/bin/xray", wide: true }))),
        section(t("advanced"),
          row(t("testUrl"), t("testUrlHint"), text("test_url", { wide: true })))));
  }
  render();
  return () => {};
}
