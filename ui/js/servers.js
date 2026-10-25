// The main view: connection card plus the server list.

import {
  h, icon, toast, menu, confirmDialog, spinner,
  formatRate, formatBytes, transportLabel, PROTOCOL_LABEL,
} from "./dom.js";
import { t } from "./i18n.js";
import { S, bus, call, attempt, refresh, serverById, activeServer } from "./api.js";
import { openImport, openServerEditor, showQr, copyLink } from "./editor.js";
import { openSubscriptionEditor } from "./subs.js";

const view = {
  search: "",
  source: "all", // all | manual | <subscription id>
  sort: "default", // default | delay | stability | name
  focusId: null,
  checked: new Set(),
};

const reliabilityOf = (health) => (health ? (health.ok + 1) / (health.ok + health.fail + 2) : null);
const sampleCount = (health) => (health ? health.ok + health.fail : 0);

/** The delay shown for a server: a real test beats a TCP ping, a health probe beats nothing. */
function bestDelay(server) {
  const l = S.latency[server.id];
  if (l?.real != null) return l.real;
  if (l?.tcp != null) return l.tcp;
  return S.state.health[server.id]?.last_ms ?? null;
}

const delayClass = (ms) => (ms < 250 ? "ok" : ms < 700 ? "warn" : "bad");

function visibleServers() {
  const q = view.search.trim().toLowerCase();
  let list = S.state.servers.filter((s) => {
    if (view.source === "manual" && s.subscription) return false;
    if (!["all", "manual"].includes(view.source) && s.subscription !== view.source) return false;
    return !q || `${s.name} ${s.address} ${s.protocol} ${s.network}`.toLowerCase().includes(q);
  });
  const key = {
    delay: (s) => bestDelay(s) ?? Infinity,
    stability: (s) => -(sampleCount(S.state.health[s.id]) ? reliabilityOf(S.state.health[s.id]) : -1),
    name: (s) => s.name.toLowerCase(),
  }[view.sort];
  if (key) list = [...list].sort((a, b) => (key(a) > key(b) ? 1 : key(a) < key(b) ? -1 : 0));
  return list;
}

/* ---------- connection card ---------- */

function sparkline(history) {
  const width = 240;
  const height = 56;
  const points = history.length ? history : [0];
  const max = Math.max(...points, 1024);
  const step = width / Math.max(points.length - 1, 1);
  const coords = points.map((v, i) => `${(i * step).toFixed(1)},${(height - 4 - (v / max) * (height - 8)).toFixed(1)}`);
  const line = coords.join(" ");
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  svg.setAttribute("class", "spark");
  svg.setAttribute("preserveAspectRatio", "none");
  const area = document.createElementNS(ns, "polygon");
  area.setAttribute("points", `0,${height} ${line} ${width},${height}`);
  area.setAttribute("class", "spark-area");
  const stroke = document.createElementNS(ns, "polyline");
  stroke.setAttribute("points", line);
  stroke.setAttribute("class", "spark-line");
  svg.append(area, stroke);
  return svg;
}

function trafficBlock() {
  const { running } = S.state;
  const tr = S.traffic;
  return h("div", { class: "hero-traffic" },
    sparkline(running ? tr.history : []),
    h("div", { class: "rates" },
      h("span", { class: "rate" }, icon("down", 14), h("b", {}, formatRate(running ? tr.downRate : 0))),
      h("span", { class: "rate" }, icon("up", 14), h("b", {}, formatRate(running ? tr.upRate : 0)))),
    running ? h("div", { class: "muted small" }, `${formatBytes(tr.down)} ↓  ·  ${formatBytes(tr.up)} ↑`) : null);
}

function buildHero() {
  const root = h("section", { class: "hero card" });

  /** Traffic changes every couple of seconds; only that block is redrawn so the buttons stay put. */
  const updateTraffic = () => root.querySelector(".hero-traffic")?.replaceWith(trafficBlock());

  const update = () => {
    const { running } = S.state;
    const active = activeServer();
    const target = serverById(view.focusId) ?? serverById(S.state.selected);
    const shown = active ?? target;
    const delay = shown ? bestDelay(shown) : null;

    const power = h("button", {
      class: `power ${running ? "on" : ""} ${S.busy ? "busy" : ""}`,
      disabled: S.busy,
      "aria-label": running ? t("disconnect") : t("connect"),
      onclick: toggleConnection,
    }, S.busy ? spinner() : icon("power", 34));

    root.replaceChildren(
      power,
      h("div", { class: "hero-main" },
        h("div", { class: `hero-state ${running ? "on" : ""}` },
          S.busy ? t("connecting") : running ? t("connected") : t("disconnected")),
        h("div", { class: "hero-server" }, shown ? shown.name : t("noServerSelected")),
        shown
          ? h("div", { class: "hero-meta" },
              h("span", { class: "badge" }, PROTOCOL_LABEL[shown.protocol] ?? shown.protocol),
              h("span", { class: "muted" }, transportLabel(shown)),
              delay != null ? h("span", { class: `pill ${delayClass(delay)}` }, `${delay} ms`) : null)
          : h("div", { class: "muted" }, t("pickServerHint")),
        h("div", { class: "hero-actions" },
          h("button", { class: "btn btn-soft", disabled: S.busy || !S.state.core_found, onclick: connectBest },
            icon("bolt", 16), t("bestServer")),
          S.state.settings.auto_switch
            ? h("span", { class: "muted small", title: t("autoSwitchHint") }, icon("shield", 14), t("autoSwitchOn"))
            : null)),
      trafficBlock(),
    );
  };
  return { root, update, updateTraffic };
}

async function toggleConnection() {
  if (S.busy) return;
  if (S.state.running) {
    S.busy = true;
    bus.emit("busy");
    try { await call("disconnect"); } finally { S.busy = false; await refresh(); }
    return;
  }
  const id = view.focusId ?? S.state.selected;
  if (!id) return toast(t("selectOne"), "error");
  await connectTo(id);
}

export async function connectTo(id) {
  if (S.busy) return;
  S.busy = true;
  bus.emit("busy");
  try {
    await call("select_server", { id });
    if (!S.state.running) await call("connect", { id });
  } catch {
    /* the error toast is already shown */
  } finally {
    S.busy = false;
    await refresh();
  }
}

async function connectBest() {
  if (S.busy) return;
  S.busy = true;
  bus.emit("busy");
  toast(t("searchingBest"), "info", { duration: 2500 });
  try {
    const name = await call("connect_best");
    toast(t("connectedTo", { name }), "ok");
  } catch {
    /* the error toast is already shown */
  } finally {
    S.busy = false;
    await refresh();
  }
}

/* ---------- server rows ---------- */

function latencyCell(server, kind) {
  const l = S.latency[server.id]?.[kind];
  const error = S.latency[server.id]?.[`${kind}Error`];
  if (l === undefined) return h("span", { class: "faint" }, "—");
  if (l === null) return h("span", { class: "pill bad", title: error ?? "" }, t("timeout"));
  return h("span", { class: `pill ${delayClass(l)}` }, `${l} ms`);
}

function stabilityCell(server) {
  const health = S.state.health[server.id];
  if (!sampleCount(health)) return h("span", { class: "faint" }, "—");
  const value = reliabilityOf(health);
  const percent = Math.round(value * 100);
  return h("div", { class: "stability", title: t("stabilityTip", { ok: health.ok, fail: health.fail }) },
    h("div", { class: "meter" }, h("span", {
      class: percent >= 90 ? "ok" : percent >= 60 ? "warn" : "bad",
      style: `width:${percent}%`,
    })),
    h("span", { class: "small" }, `${percent}%`));
}

function rowActions(server) {
  const action = (name, label, onClick, extra = "") =>
    h("button", {
      class: `btn-icon ${extra}`, title: label, "aria-label": label,
      onclick: (e) => { e.stopPropagation(); onClick(); },
    }, icon(name, 16));
  return h("div", { class: "row-actions" },
    action("power", t("connect"), () => connectTo(server.id)),
    action("edit", t("edit"), () => openServerEditor(server)),
    action("copy", t("copyLink"), () => copyLink(server)),
    action("qr", t("qrCode"), () => showQr(server)),
    action("layers", t("duplicate"), async () => { await attempt("duplicate_server", { id: server.id }); }),
    action("trash", t("delete"), () => deleteServers([server.id]), "danger"));
}

async function deleteServers(ids) {
  if (!(await confirmDialog(t("confirmDelete", { n: ids.length })))) return;
  await attempt("delete_servers", { ids });
  ids.forEach((id) => view.checked.delete(id));
  await refresh();
}

function buildRow(server, refreshList) {
  const isActive = S.state.running && S.state.active === server.id;
  const subscription = S.state.subscriptions.find((s) => s.id === server.subscription);
  return h("div", {
    class: ["server-row", server.id === S.state.selected ? "selected" : "", server.id === view.focusId ? "focus" : "", isActive ? "active" : ""].join(" "),
    role: "row", tabindex: 0,
    onclick: () => { view.focusId = server.id; refreshList(); bus.emit("focus"); },
    ondblclick: () => connectTo(server.id),
    onkeydown: (e) => {
      if (e.key === "Enter") connectTo(server.id);
      if (e.key === "Delete") deleteServers([server.id]);
    },
  },
  h("label", { class: "check", onclick: (e) => e.stopPropagation() }, h("input", {
    type: "checkbox", checked: view.checked.has(server.id),
    onchange: (e) => {
      e.target.checked ? view.checked.add(server.id) : view.checked.delete(server.id);
      refreshList();
    },
  })),
  h("div", { class: "cell-main" },
    h("div", { class: "server-name" },
      isActive ? h("span", { class: "live-dot", title: t("connected") }) : null,
      h("span", { class: "name-text", title: server.name }, server.name)),
    h("div", { class: "server-sub" },
      h("span", { class: "badge" }, PROTOCOL_LABEL[server.protocol] ?? server.protocol),
      h("span", {}, transportLabel(server)),
      h("span", { class: "addr", dir: "ltr" }, `${server.address}:${server.port}`),
      subscription ? h("span", { class: "tag" }, subscription.name) : null)),
  h("div", { class: "cell cell-tcp" }, latencyCell(server, "tcp")),
  h("div", { class: "cell cell-real" }, latencyCell(server, "real")),
  h("div", { class: "cell cell-stab" }, stabilityCell(server)),
  rowActions(server));
}

/* ---------- the view ---------- */

export function mountServers(container) {
  const hero = buildHero();
  const list = h("div", { class: "server-list", role: "table" });
  const bulk = h("div", { class: "bulk", hidden: true });
  const empty = h("div", { class: "empty card", hidden: true });

  const sourceSelect = h("select", { class: "input select-sm", onchange: (e) => { view.source = e.target.value; updateList(); } });
  const sortSelect = h("select", { class: "input select-sm", onchange: (e) => { view.sort = e.target.value; updateList(); } },
    ["default", "delay", "stability", "name"].map((k) => h("option", { value: k, selected: view.sort === k }, t(`sort_${k}`))));
  const search = h("input", {
    class: "input", type: "search", placeholder: t("searchPlaceholder"), title: "Ctrl+K", value: view.search, id: "search-input",
    oninput: (e) => { view.search = e.target.value; updateList(); },
  });

  const targetIds = () => (view.checked.size ? [...view.checked] : visibleServers().map((s) => s.id));
  const runTest = async (command) => {
    const ids = targetIds();
    if (!ids.length) return;
    const kind = command === "test_tcp" ? "tcp" : "real";
    ids.forEach((id) => { if (S.latency[id]) delete S.latency[id][kind]; });
    updateList();
    await attempt(command, { ids });
    await refresh();
  };

  const addButton = h("button", { class: "btn btn-soft", onclick: () =>
    menu(addButton, [
      { icon: "clipboard", label: t("pasteLinks"), onClick: openImport },
      { icon: "rss", label: t("addSubscription"), onClick: () => openSubscriptionEditor() },
      { icon: "plus", label: t("addManually"), onClick: () => openServerEditor(null) },
    ]) }, icon("plus", 16), t("add"));

  function refreshSources() {
    const current = view.source;
    sourceSelect.replaceChildren(
      h("option", { value: "all" }, t("allServers")),
      h("option", { value: "manual" }, t("manualServers")),
      ...S.state.subscriptions.map((s) => h("option", { value: s.id }, s.name)));
    const valid = ["all", "manual", ...S.state.subscriptions.map((s) => s.id)];
    view.source = valid.includes(current) ? current : "all";
    sourceSelect.value = view.source;
  }

  function updateList() {
    const servers = visibleServers();
    const known = new Set(S.state.servers.map((s) => s.id));
    view.checked.forEach((id) => { if (!known.has(id)) view.checked.delete(id); });
    if (view.focusId && !known.has(view.focusId)) view.focusId = null;

    const nothingAtAll = S.state.servers.length === 0;
    empty.hidden = !nothingAtAll;
    list.hidden = nothingAtAll;
    if (nothingAtAll) {
      empty.replaceChildren(
        icon("servers", 40),
        h("h3", {}, t("emptyTitle")),
        h("p", { class: "muted" }, t("emptyText")),
        h("div", { class: "empty-actions" },
          h("button", { class: "btn btn-primary", onclick: openImport }, icon("clipboard", 16), t("pasteLinks")),
          h("button", { class: "btn btn-soft", onclick: () => openSubscriptionEditor() }, icon("rss", 16), t("addSubscription")),
          h("button", { class: "btn btn-ghost", onclick: () => openServerEditor(null) }, icon("plus", 16), t("addManually"))));
    }

    const rows = servers.map((s) => buildRow(s, updateList));
    list.replaceChildren(...[
      servers.length
        ? h("div", { class: "list-head", role: "row" },
            h("label", { class: "check" }, h("input", {
              type: "checkbox",
              checked: servers.every((s) => view.checked.has(s.id)),
              onchange: (e) => {
                servers.forEach((s) => (e.target.checked ? view.checked.add(s.id) : view.checked.delete(s.id)));
                updateList();
              },
            })),
            h("div", { class: "cell-main" }, `${servers.length} ${t("serversCount")}`),
            h("div", { class: "cell cell-tcp" }, "TCP"),
            h("div", { class: "cell cell-real" }, t("delay")),
            h("div", { class: "cell cell-stab" }, t("stability")))
        : null,
      ...rows,
      !servers.length && !nothingAtAll ? h("div", { class: "empty-small muted" }, t("noMatches")) : null,
    ].filter(Boolean));

    bulk.hidden = view.checked.size === 0;
    bulk.replaceChildren(
      h("span", {}, t("selectedCount", { n: view.checked.size })),
      h("button", { class: "btn btn-ghost btn-sm", onclick: () => { view.checked.clear(); updateList(); } }, t("clearSelection")),
      h("span", { class: "grow" }),
      h("button", { class: "btn btn-ghost btn-sm", onclick: () => runTest("test_tcp") }, t("tcpTest")),
      h("button", { class: "btn btn-ghost btn-sm", onclick: () => runTest("test_real") }, t("realTest")),
      h("button", { class: "btn btn-danger btn-sm", onclick: () => deleteServers([...view.checked]) }, icon("trash", 14), t("delete")));
    hero.update();
  }

  container.replaceChildren(
    h("header", { class: "view-head" }, h("h1", {}, t("nav_servers"))),
    hero.root,
    h("div", { class: "toolbar" },
      h("div", { class: "search" }, icon("search", 16), search),
      sourceSelect, sortSelect,
      h("span", { class: "grow" }),
      h("button", { class: "btn btn-ghost", onclick: () => runTest("test_tcp"), title: t("tcpTestHint") }, icon("activity", 16), t("tcpTest")),
      h("button", { class: "btn btn-ghost", onclick: () => runTest("test_real"), title: t("realTestHint") }, icon("radar", 16), t("realTest")),
      addButton),
    bulk, list, empty);

  refreshSources();
  updateList();

  const offs = [
    bus.on("state", () => { refreshSources(); updateList(); }),
    bus.on("latency", updateList),
    bus.on("traffic", hero.updateTraffic),
    bus.on("busy", hero.update),
  ];
  return () => offs.forEach((off) => off());
}

export function focusSearch() {
  const input = document.getElementById("search-input");
  input?.focus();
  input?.select();
}
