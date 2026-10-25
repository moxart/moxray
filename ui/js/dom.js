// Small DOM toolkit: element builder, icons, toasts, modals, menus and formatters.
// Text always goes in through textContent, never as HTML, because server names come from
// subscriptions and are untrusted.

import { t } from "./i18n.js";

export const $ = (selector, root = document) => root.querySelector(selector);

export function h(tag, props = {}, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props ?? {})) {
    if (value == null || value === false) continue;
    if (key === "class") el.className = value;
    else if (key === "dataset") Object.assign(el.dataset, value);
    else if (key.startsWith("on")) el.addEventListener(key.slice(2), value);
    else if (key === "value" || key === "checked" || key === "disabled" || key === "selected") el[key] = value;
    else if (value === true) el.setAttribute(key, "");
    else el.setAttribute(key, value);
  }
  for (const child of children.flat(Infinity)) {
    if (child == null || child === false) continue;
    el.append(child.nodeType ? child : document.createTextNode(String(child)));
  }
  return el;
}

/* ---------- icons (static, trusted markup) ---------- */

const ICONS = {
  power: '<path d="M12 2v10"/><path d="M18.4 6.6a9 9 0 1 1-12.8 0"/>',
  servers: '<rect x="3" y="4" width="18" height="6" rx="2"/><rect x="3" y="14" width="18" height="6" rx="2"/><path d="M7 7h.01M7 17h.01"/>',
  rss: '<path d="M4 11a9 9 0 0 1 9 9"/><path d="M4 4a16 16 0 0 1 16 16"/><circle cx="5" cy="19" r="1"/>',
  tools: '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>',
  settings: '<path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6"/>',
  logs: '<path d="m4 17 6-6-6-6"/><path d="M12 19h8"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/>',
  bolt: '<path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z"/>',
  refresh: '<path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><path d="M21 3v5h-5"/>',
  trash: '<path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/>',
  edit: '<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/>',
  copy: '<rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>',
  qr: '<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><path d="M14 14h3v3M21 14v.01M14 21h3M21 17v4"/>',
  check: '<path d="m5 12 5 5L20 7"/>',
  x: '<path d="M18 6 6 18M6 6l12 12"/>',
  globe: '<circle cx="12" cy="12" r="10"/><path d="M2 12h20M12 2a15 15 0 0 1 0 20M12 2a15 15 0 0 0 0 20"/>',
  shield: '<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>',
  download: '<path d="M12 3v12m0 0-4-4m4 4 4-4"/><path d="M4 21h16"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
  moon: '<path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"/>',
  clipboard: '<rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/>',
  activity: '<path d="M22 12h-4l-3 9L9 3l-3 9H2"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
  radar: '<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="5"/><path d="M12 12 19 5"/>',
  layers: '<path d="m12 2 10 5-10 5L2 7z"/><path d="m2 17 10 5 10-5M2 12l10 5 10-5"/>',
  info: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/>',
  alert: '<path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z"/><path d="M12 9v4M12 17h.01"/>',
  link: '<path d="M10 13a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-1 1"/><path d="M14 11a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l1-1"/>',
  chevron: '<path d="m6 9 6 6 6-6"/>',
  up: '<path d="M12 19V5M5 12l7-7 7 7"/>',
  down: '<path d="M12 5v14M19 12l-7 7-7-7"/>',
  languages: '<path d="m5 8 6 6M4 14l6-6 2-3M2 5h12M7 2h1M22 22l-5-10-5 10M14 18h6"/>',
};

export function icon(name, size = 18) {
  const svg = new DOMParser().parseFromString(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] ?? ""}</svg>`,
    "image/svg+xml",
  ).documentElement;
  return document.importNode(svg, true);
}

export const spinner = () => h("span", { class: "spinner", role: "status" });

/* ---------- toasts ---------- */

export function toast(message, kind = "info", options = {}) {
  let host = $("#toasts");
  if (!host) {
    host = h("div", { id: "toasts", class: "toasts" });
    document.body.append(host);
  }
  const node = h("div", { class: `toast ${kind}`, role: "status" },
    h("span", { class: "toast-text" }, message),
    options.action
      ? h("button", { class: "btn btn-ghost btn-sm", onclick: () => { options.action.onClick(); node.remove(); } }, options.action.label)
      : null,
    h("button", { class: "btn-icon", "aria-label": t("close"), onclick: () => node.remove() }, icon("x", 14)),
  );
  host.append(node);
  const ms = options.duration ?? (kind === "error" ? 8000 : 4000);
  if (ms > 0) setTimeout(() => node.remove(), ms);
  return node;
}

/* ---------- modals ---------- */

const modalStack = [];

export function openModal({ title, body, buttons = [], wide = false, onClose }) {
  const previouslyFocused = document.activeElement;
  const close = () => {
    const index = modalStack.indexOf(handle);
    if (index === -1) return;
    modalStack.splice(index, 1);
    overlay.remove();
    onClose?.();
    previouslyFocused?.focus?.();
  };
  const footer = buttons.length
    ? h("div", { class: "modal-foot" }, buttons.map((b) => {
        const button = h("button", {
          class: `btn ${b.primary ? "btn-primary" : b.danger ? "btn-danger" : "btn-ghost"}`,
          onclick: async () => {
            button.disabled = true;
            try {
              if ((await b.onClick?.(close)) !== false && !b.keepOpen) close();
            } finally {
              button.disabled = false;
            }
          },
        }, b.icon ? icon(b.icon, 16) : null, b.label);
        return button;
      }))
    : null;
  const overlay = h("div", {
    class: "overlay",
    onmousedown: (e) => { if (e.target === overlay) close(); },
  }, h("div", { class: `modal ${wide ? "wide" : ""}`, role: "dialog", "aria-modal": "true", "aria-label": title },
    h("div", { class: "modal-head" }, h("h2", {}, title),
      h("button", { class: "btn-icon", "aria-label": t("close"), onclick: close }, icon("x", 16))),
    h("div", { class: "modal-body" }, body),
    footer));
  const handle = { close, root: overlay };
  modalStack.push(handle);
  document.body.append(overlay);
  overlay.querySelector("input:not([type=checkbox]), textarea, select")?.focus();
  return handle;
}

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && modalStack.length) modalStack[modalStack.length - 1].close();
});

export function confirmDialog(message, { confirmLabel = t("delete"), danger = true } = {}) {
  return new Promise((resolve) => {
    let answered = false;
    const answer = (value) => { answered = true; resolve(value); };
    openModal({
      title: t("confirm"),
      body: h("p", { class: "confirm-text" }, message),
      buttons: [
        { label: t("cancel"), onClick: () => answer(false) },
        { label: confirmLabel, danger, primary: !danger, onClick: () => answer(true) },
      ],
      onClose: () => { if (!answered) resolve(false); },
    });
  });
}

/* ---------- popover menu ---------- */

export function menu(anchor, items) {
  document.querySelector(".menu")?.remove();
  const rect = anchor.getBoundingClientRect();
  const rtl = document.documentElement.dir === "rtl";
  const node = h("div", {
    class: "menu",
    role: "menu",
    style: `top:${rect.bottom + 6}px;${rtl ? "right" : "left"}:${rtl ? window.innerWidth - rect.right : rect.left}px`,
  }, items.map((item) => item === "-"
    ? h("div", { class: "menu-sep" })
    : h("button", {
        class: "menu-item", role: "menuitem",
        onclick: () => { node.remove(); item.onClick(); },
      }, item.icon ? icon(item.icon, 16) : null, item.label)));
  document.body.append(node);
  const dismiss = (e) => {
    if (node.contains(e.target)) return;
    node.remove();
    document.removeEventListener("mousedown", dismiss, true);
  };
  setTimeout(() => document.addEventListener("mousedown", dismiss, true), 0);
  return node;
}

/* ---------- formatters ---------- */

export function formatBytes(n) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${n.toFixed(i === 0 || n >= 100 ? 0 : 1)} ${units[i]}`;
}

export const formatRate = (n) => `${formatBytes(n)}/s`;

export function formatDate(epochSeconds, lang) {
  return new Date(epochSeconds * 1000).toLocaleDateString(lang === "fa" ? "fa-IR" : undefined, {
    year: "numeric", month: "short", day: "numeric",
  });
}

export function formatDateTime(epochSeconds, lang) {
  return new Date(epochSeconds * 1000).toLocaleString(lang === "fa" ? "fa-IR" : undefined);
}

export const daysLeft = (epochSeconds) => Math.ceil((epochSeconds * 1000 - Date.now()) / 86_400_000);

/** Country code to flag emoji (regional indicator letters). */
export function flag(code) {
  if (!/^[A-Za-z]{2}$/.test(code ?? "")) return "";
  return String.fromCodePoint(...[...code.toUpperCase()].map((c) => 127397 + c.charCodeAt(0)));
}

export async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

export function debounce(fn, ms) {
  let timer;
  return (...args) => { clearTimeout(timer); timer = setTimeout(() => fn(...args), ms); };
}

/** "tcp", "ws + tls", "tcp + reality" */
export function transportLabel(s) {
  return [s.network, s.security !== "none" ? s.security : null].filter(Boolean).join(" + ");
}

export const PROTOCOL_LABEL = { vless: "VLESS", vmess: "VMess", trojan: "Trojan", shadowsocks: "Shadowsocks" };
