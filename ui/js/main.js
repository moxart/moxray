// App shell: sidebar, view switching, appearance, shortcuts and clipboard offers.

import { h, icon, toast, $ } from "./dom.js";
import { t, setLang } from "./i18n.js";
import { S, bus, refresh, listenToBackend, pollTraffic, pushLog, attempt, patchSettings, activeServer } from "./api.js";
import { mountServers, focusSearch } from "./servers.js";
import { mountSubscriptions } from "./subs.js";
import { mountTools } from "./tools.js";
import { mountLogs } from "./logs.js";
import { mountSettings } from "./settings.js";
import { showWelcome } from "./welcome.js";

const VIEWS = [
  { id: "servers", icon: "servers", mount: mountServers },
  { id: "subs", icon: "rss", mount: mountSubscriptions },
  { id: "tools", icon: "tools", mount: mountTools },
  { id: "logs", icon: "logs", mount: mountLogs },
  { id: "settings", icon: "settings", mount: mountSettings },
];

let current = "servers";
let disposeView = () => {};
let appearance = "";

function applyAppearance() {
  const { theme, language } = S.state.settings;
  document.documentElement.dataset.theme = theme;
  setLang(language);
}

function showView(id) {
  disposeView();
  current = id;
  disposeView = VIEWS.find((v) => v.id === id).mount($("#view")) ?? (() => {});
  renderSidebar();
}

function renderSidebar() {
  const { running, settings, core_found } = S.state;
  const active = activeServer();
  $("#sidebar").replaceChildren(
    h("div", { class: "brand" },
      h("img", { class: "logo", src: "logo.svg", alt: "", width: 36, height: 36 }),
      h("span", { class: "brand-name" }, "Mox", h("span", { class: "brand-accent" }, "Ray"))),
    h("nav", { class: "nav", "aria-label": "Main" }, VIEWS.map((v) =>
      h("button", {
        class: `nav-item ${v.id === current ? "on" : ""}`,
        "aria-current": v.id === current ? "page" : null,
        onclick: () => showView(v.id),
      }, icon(v.icon, 18), h("span", { class: "nav-label" }, t(`nav_${v.id}`))))),
    h("div", { class: "sidebar-foot" },
      h("div", { class: `status-card ${running ? "on" : ""}` },
        h("span", { class: "live-dot" }),
        h("div", { class: "status-text" },
          h("b", {}, running ? t("connected") : t("disconnected")),
          h("span", { class: "muted small" }, running ? (active?.name ?? "") : core_found ? t("ready") : t("coreMissingShort")))),
      h("div", { class: "foot-actions" },
        h("button", {
          class: "btn-icon", title: t("theme"), "aria-label": t("theme"),
          onclick: () => patchSettings({ theme: settings.theme === "dark" ? "light" : "dark" }),
        }, icon(settings.theme === "dark" ? "sun" : "moon", 18)),
        h("button", {
          class: "btn-icon", title: t("language"), "aria-label": t("language"),
          onclick: () => patchSettings({ language: settings.language === "fa" ? "en" : "fa" }),
        }, icon("languages", 18)))));
}

/* Offer to import share links found on the clipboard when the window regains focus. */
let lastOffered = "";
async function offerClipboard() {
  try {
    const text = await navigator.clipboard.readText();
    const links = text.match(/(?:vmess|vless|trojan|ss):\/\/[^\s"'<>]+/gi) ?? [];
    const key = links.join("\n");
    if (!links.length || key === lastOffered) return;
    lastOffered = key;
    toast(t("clipboardFound", { n: links.length }), "info", {
      duration: 15000,
      action: {
        label: t("import"),
        onClick: async () => {
          const report = await attempt("import_links", { text: key });
          if (report) toast(t("importDone", report), report.added ? "ok" : "info");
          await refresh();
        },
      },
    });
  } catch {
    /* clipboard access can be denied; the Import dialog still works */
  }
}

document.addEventListener("keydown", (e) => {
  const typing = ["INPUT", "TEXTAREA", "SELECT"].includes(document.activeElement?.tagName);
  if (((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") || (e.key === "/" && !typing)) {
    e.preventDefault();
    if (current !== "servers") showView("servers");
    focusSearch();
  }
});

async function init() {
  listenToBackend();
  await refresh();
  for (const line of await attempt("get_logs", null, [])) pushLog(line);

  applyAppearance();
  appearance = `${S.state.settings.theme}|${S.state.settings.language}`;
  showView("servers");

  // Theme or language changed: re-render everything in the new look.
  bus.on("state", () => {
    const next = `${S.state.settings.theme}|${S.state.settings.language}`;
    if (next !== appearance) {
      appearance = next;
      applyAppearance();
      showView(current);
    } else {
      renderSidebar();
    }
  });

  setInterval(pollTraffic, 2000);
  window.addEventListener("focus", offerClipboard);
  setTimeout(offerClipboard, 1500);

  if (!S.state.settings.onboarded) showWelcome(() => { applyAppearance(); renderSidebar(); });
}

init().catch((err) => toast(String(err), "error", { duration: 0 }));
