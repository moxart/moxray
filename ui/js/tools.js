// Tools: connection check and the clean-IP scanner.

import { h, icon, toast, flag, spinner } from "./dom.js";
import { t } from "./i18n.js";
import { S, bus, call, attempt, refresh } from "./api.js";

function traceBlock(title, result) {
  const body = !result
    ? h("p", { class: "muted" }, t("notConnected"))
    : result.error
      ? h("p", { class: "bad" }, result.error)
      : h("dl", { class: "facts" },
          h("dt", {}, "IP"), h("dd", { dir: "ltr" }, result.trace.ip),
          h("dt", {}, t("country")), h("dd", {}, `${flag(result.trace.country)} ${result.trace.country}`),
          h("dt", {}, t("datacenter")), h("dd", { dir: "ltr" }, result.trace.colo));
  return h("div", { class: "trace" }, h("h4", {}, title), body);
}

function connectionCard() {
  const results = h("div", { class: "trace-grid" });
  const verdict = h("div", { class: "verdict", hidden: true });
  const run = h("button", { class: "btn btn-soft" }, icon("globe", 16), t("runCheck"));
  run.onclick = async () => {
    run.disabled = true;
    results.replaceChildren(spinner());
    const report = await attempt("check_ip");
    run.disabled = false;
    if (!report) return results.replaceChildren();
    results.replaceChildren(
      traceBlock(t("direct"), report.direct),
      traceBlock(t("viaProxy"), report.proxied));
    const a = report.direct.trace?.ip;
    const b = report.proxied?.trace?.ip;
    verdict.hidden = !(a && b);
    if (a && b) {
      verdict.className = `verdict ${a !== b ? "ok" : "warn"}`;
      verdict.replaceChildren(icon(a !== b ? "check" : "alert", 16), a !== b ? t("verdictOk") : t("verdictSame"));
    }
  };
  return h("section", { class: "card tool" },
    h("div", { class: "tool-head" },
      h("div", {}, h("h3", {}, t("connectionCheck")), h("p", { class: "muted" }, t("connectionCheckText"))),
      run),
    results, verdict);
}

function scannerCard() {
  const servers = S.state.servers.filter((s) => s.security === "tls" || s.network === "ws");
  const serverSelect = h("select", { class: "input" },
    servers.length
      ? servers.map((s) => h("option", { value: s.id }, s.name))
      : [h("option", { value: "" }, t("noTlsServers"))]);
  const hostInput = h("input", { class: "input", dir: "ltr", spellcheck: false, placeholder: "example.com" });
  const samples = h("select", { class: "input" },
    [50, 100, 200, 400].map((n) => h("option", { value: n, selected: n === 100 }, t("samplesN", { n }))));
  const progress = h("div", { class: "meter", hidden: true }, h("span", { class: "ok", style: "width:0%" }));
  const status = h("div", { class: "muted small" });
  const table = h("div", { class: "scan-results" });
  const hits = [];

  const hostFor = (id) => {
    const s = S.state.servers.find((x) => x.id === id);
    if (!s) return "";
    for (const candidate of [s.sni, s.host, s.address]) {
      if (candidate && !/^[\d.:]+$/.test(candidate)) return candidate;
    }
    return "";
  };
  const syncHost = () => { hostInput.value = hostFor(serverSelect.value); };
  serverSelect.onchange = syncHost;
  syncHost();

  const renderHits = () => {
    hits.sort((a, b) => a.ms - b.ms);
    table.replaceChildren(...hits.slice(0, 15).map((hit) =>
      h("div", { class: "scan-row" },
        h("code", { dir: "ltr" }, hit.ip),
        h("span", { class: `pill ${hit.ms < 400 ? "ok" : hit.ms < 900 ? "warn" : "bad"}` }, `${hit.ms} ms`),
        h("span", { class: "tag", dir: "ltr" }, hit.colo),
        h("button", { class: "btn btn-soft btn-sm", disabled: !serverSelect.value, onclick: async () => {
          const id = await attempt("apply_clean_ip", { id: serverSelect.value, ip: hit.ip });
          if (id) { toast(t("cleanIpApplied", { ip: hit.ip }), "ok"); await refresh(); }
        } }, t("useThisIp")))));
  };

  const start = h("button", { class: "btn btn-primary" }, icon("radar", 16), t("startScan"));
  const stop = h("button", { class: "btn btn-ghost", hidden: true }, icon("stop", 16), t("stopScan"));
  stop.onclick = () => attempt("cancel_scan");

  const offs = [
    bus.on("scan-hit", (hit) => { hits.push(hit); renderHits(); }),
    bus.on("scan-progress", ({ done, total }) => {
      progress.firstChild.style.width = `${Math.round((done / total) * 100)}%`;
      status.textContent = t("scanProgress", { done, total, found: hits.length });
    }),
  ];

  start.onclick = async () => {
    if (!hostInput.value.trim()) return toast(t("needHost"), "error");
    hits.length = 0;
    renderHits();
    start.disabled = true;
    stop.hidden = false;
    progress.hidden = false;
    progress.firstChild.style.width = "0%";
    status.textContent = t("scanning");
    const final = await attempt("scan_clean_ips", { host: hostInput.value, samples: Number(samples.value), ranges: null });
    start.disabled = false;
    stop.hidden = true;
    if (final) {
      hits.length = 0;
      hits.push(...final);
      renderHits();
      status.textContent = final.length ? t("scanDone", { n: final.length }) : t("scanNone");
    }
  };

  const field = (label, input) => h("div", { class: "field" }, h("label", {}, label), input);
  return {
    node: h("section", { class: "card tool" },
      h("div", { class: "tool-head" },
        h("div", {}, h("h3", {}, t("cleanIpScanner")), h("p", { class: "muted" }, t("cleanIpText")))),
      h("div", { class: "row" }, field(t("server"), serverSelect), field(t("hostName"), hostInput), field(t("samples"), samples)),
      h("div", { class: "tool-actions" }, start, stop),
      progress, status, table,
      h("p", { class: "hint small" }, t("scanNote"))),
    dispose: () => offs.forEach((off) => off()),
  };
}

export function mountTools(container) {
  const scanner = scannerCard();
  container.replaceChildren(
    h("header", { class: "view-head" }, h("h1", {}, t("nav_tools"))),
    h("div", { class: "stack" }, connectionCard(), scanner.node));
  return scanner.dispose;
}
