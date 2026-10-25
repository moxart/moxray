// Dialogs for adding and editing servers: link import, manual editor, QR code.

import { h, icon, openModal, toast, copyText } from "./dom.js";
import { t } from "./i18n.js";
import { call, attempt, refresh } from "./api.js";

const VMESS_SECURITY = ["auto", "aes-128-gcm", "chacha20-poly1305", "none", "zero"];
const SS_CIPHERS = [
  "aes-256-gcm", "aes-128-gcm", "chacha20-ietf-poly1305",
  "2022-blake3-aes-256-gcm", "2022-blake3-aes-128-gcm", "2022-blake3-chacha20-poly1305",
];
const FINGERPRINTS = ["", "chrome", "firefox", "safari", "ios", "android", "edge", "randomized"];

const pairs = (values) => values.map((v) => [v, v || "—"]);

export function blankServer() {
  return {
    id: "", name: "", protocol: "vless", address: "", port: 443, secret: "", method: "", alter_id: 0,
    flow: "", network: "tcp", security: "tls", sni: "", host: "", path: "", alpn: "", fingerprint: "chrome",
    public_key: "", short_id: "", spider_x: "", header_type: "", service_name: "", allow_insecure: false,
    subscription: null,
  };
}

export function openImport() {
  const area = h("textarea", {
    class: "input mono", dir: "ltr", spellcheck: false, rows: 8,
    placeholder: "vless://…\nvmess://…\ntrojan://…\nss://…",
  });
  openModal({
    title: t("importTitle"),
    body: [h("p", { class: "muted" }, t("importHint")), area],
    buttons: [
      { label: t("cancel") },
      {
        label: t("import"), primary: true, icon: "download",
        onClick: async () => {
          if (!area.value.trim()) return false;
          const report = await attempt("import_links", { text: area.value });
          if (!report) return false;
          toast(t("importDone", report), report.added ? "ok" : "info");
          await refresh();
        },
      },
    ],
  });
}

export async function showQr(server) {
  const [svg, link] = await Promise.all([
    attempt("link_qr", { id: server.id }),
    attempt("export_link", { id: server.id }),
  ]);
  if (!svg) return;
  // The SVG comes from our own backend (qrcode crate), never from a subscription.
  const holder = h("div", { class: "qr" });
  holder.innerHTML = svg;
  openModal({
    title: server.name,
    body: [holder, h("code", { class: "qr-link", dir: "ltr" }, link)],
    buttons: [
      { label: t("close") },
      {
        label: t("copyLink"), primary: true, icon: "copy",
        onClick: async () => {
          toast((await copyText(link)) ? t("linkCopied") : t("copyFailed"), "ok");
          return false;
        },
      },
    ],
  });
}

export async function copyLink(server) {
  const link = await attempt("export_link", { id: server.id });
  if (!link) return;
  if (await copyText(link)) {
    toast(t("linkCopied"), "ok");
  } else {
    toast(t("copyFailed"));
    showQr(server);
  }
}

export function openServerEditor(server) {
  const draft = server ? structuredClone(server) : blankServer();
  const body = h("div", { class: "form" });

  const field = (label, input, hint) =>
    h("div", { class: "field" }, h("label", {}, label), input, hint ? h("small", { class: "hint" }, hint) : null);

  const text = (key, label, { type = "text", placeholder = "", hint } = {}) =>
    field(label, h("input", {
      class: "input", type, value: draft[key] ?? "", placeholder, dir: "ltr", spellcheck: false,
      oninput: (e) => { draft[key] = type === "number" ? Number(e.target.value) : e.target.value; },
    }), hint);

  const select = (key, label, options, rerender = false) =>
    field(label, h("select", {
      class: "input",
      onchange: (e) => { draft[key] = e.target.value; if (rerender) render(); },
    }, options.map(([value, caption]) => h("option", { value, selected: draft[key] === value }, caption))));

  const toggle = (key, label) =>
    h("label", { class: "switch-row" },
      h("span", {}, label),
      h("span", { class: "switch" }, h("input", {
        type: "checkbox", checked: draft[key],
        onchange: (e) => { draft[key] = e.target.checked; },
      }), h("span", { class: "track" })));

  const onProtocolChange = (e) => {
    draft.protocol = e.target.value;
    draft.method = { vmess: "auto", shadowsocks: "aes-256-gcm" }[draft.protocol] ?? "";
    if (draft.protocol === "shadowsocks") Object.assign(draft, { network: "tcp", security: "none" });
    if (draft.protocol === "trojan" && draft.security === "none") draft.security = "tls";
    if (draft.protocol !== "vless") draft.flow = "";
    render();
  };

  function credentialFields() {
    const p = draft.protocol;
    const parts = [text("secret", p === "vmess" || p === "vless" ? "UUID" : t("password"))];
    if (p === "vmess") {
      parts.push(h("div", { class: "row" },
        select("method", t("encryption"), pairs(VMESS_SECURITY)),
        text("alter_id", "Alter ID", { type: "number" })));
    }
    if (p === "vless") {
      parts.push(select("flow", "Flow", [["", "—"], ["xtls-rprx-vision", "xtls-rprx-vision"]]));
    }
    if (p === "shadowsocks") parts.push(select("method", t("cipher"), pairs(SS_CIPHERS)));
    return parts;
  }

  function transportFields() {
    if (draft.protocol === "shadowsocks") return [];
    const parts = [select("network", t("transport"),
      ["tcp", "ws", "grpc", "httpupgrade", "xhttp", "h2"].map((v) => [v, v]), true)];
    const n = draft.network;
    if (n === "tcp") {
      parts.push(select("header_type", t("headerType"), [["", "none"], ["http", "http"]], true));
      if (draft.header_type === "http") parts.push(h("div", { class: "row" }, text("host", "Host"), text("path", "Path")));
    } else if (n === "grpc") {
      parts.push(text("service_name", t("serviceName")));
    } else {
      parts.push(h("div", { class: "row" }, text("host", "Host"), text("path", "Path", { placeholder: "/" })));
    }
    return parts;
  }

  function securityFields() {
    if (draft.protocol === "shadowsocks") return [];
    const parts = [select("security", t("security"), [["none", "none"], ["tls", "TLS"], ["reality", "REALITY"]], true)];
    if (draft.security === "tls") {
      parts.push(
        h("div", { class: "row" }, text("sni", "SNI"),
          select("fingerprint", t("fingerprint"), pairs(FINGERPRINTS))),
        select("alpn", "ALPN", [["", "—"], ["h2,http/1.1", "h2, http/1.1"], ["h2", "h2"], ["http/1.1", "http/1.1"]]),
        toggle("allow_insecure", t("allowInsecure")),
      );
    } else if (draft.security === "reality") {
      parts.push(
        h("div", { class: "row" }, text("sni", "SNI"),
          select("fingerprint", t("fingerprint"), pairs(FINGERPRINTS.slice(1)))),
        text("public_key", t("publicKey")),
        h("div", { class: "row" }, text("short_id", t("shortId")), text("spider_x", "SpiderX", { placeholder: "/" })),
      );
    }
    return parts;
  }

  function render() {
    const transport = transportFields();
    const security = securityFields();
    body.replaceChildren(
      text("name", t("name"), { placeholder: t("namePlaceholder") }),
      h("div", { class: "row" },
        field(t("protocol"), h("select", { class: "input", onchange: onProtocolChange },
          [["vless", "VLESS"], ["vmess", "VMess"], ["trojan", "Trojan"], ["shadowsocks", "Shadowsocks"]]
            .map(([value, caption]) => h("option", { value, selected: draft.protocol === value }, caption)))),
        text("port", t("port"), { type: "number" })),
      text("address", t("address"), { placeholder: "example.com" }),
      ...credentialFields(),
      ...(transport.length ? [h("h3", { class: "form-section" }, t("transport")), ...transport] : []),
      ...(security.length ? [h("h3", { class: "form-section" }, t("security")), ...security] : []),
    );
  }
  render();

  const save = async () => {
    draft.port = Number(draft.port) || 0;
    draft.alter_id = Number(draft.alter_id) || 0;
    return attempt("save_server", { server: draft });
  };

  openModal({
    title: server ? t("editServer") : t("newServer"),
    body, wide: true,
    buttons: [
      { label: t("cancel") },
      {
        label: t("saveAndCheck"), icon: "shield",
        onClick: async () => {
          const id = await save();
          if (!id) return false;
          await refresh();
          try {
            await call("check_config", { id });
            toast(t("configOk"), "ok");
          } catch {
            /* the error toast is already shown */
          }
        },
      },
      {
        label: t("save"), primary: true,
        onClick: async () => {
          const id = await save();
          if (!id) return false;
          await refresh();
          toast(t("saved"), "ok");
        },
      },
    ],
  });
}
