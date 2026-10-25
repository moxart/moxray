// Backend bridge and shared client state.

import { toast } from "./dom.js";

const tauri = window.__TAURI__;

export const S = {
  state: null, // last get_state() result
  latency: {}, // id -> {tcp, real, tcpError, realError}
  logs: [],
  traffic: { up: 0, down: 0, upRate: 0, downRate: 0, history: [] },
  busy: false,
};

const listeners = new Map();

export const bus = {
  on(name, fn) {
    if (!listeners.has(name)) listeners.set(name, new Set());
    listeners.get(name).add(fn);
    return () => listeners.get(name).delete(fn);
  },
  emit(name, detail) {
    listeners.get(name)?.forEach((fn) => fn(detail));
  },
};

/** Invoke a backend command; failures are shown as a toast and re-thrown. */
export async function call(command, args) {
  try {
    return await tauri.core.invoke(command, args);
  } catch (err) {
    toast(String(err), "error");
    throw err;
  }
}

/** Like call(), but resolves to `fallback` instead of throwing. */
export async function attempt(command, args, fallback = null) {
  try {
    return await call(command, args);
  } catch {
    return fallback;
  }
}

export async function refresh() {
  S.state = await tauri.core.invoke("get_state");
  bus.emit("state", S.state);
}

export async function patchSettings(patch) {
  const settings = { ...S.state.settings, ...patch };
  await call("save_settings", { settings });
  await refresh();
}

export const serverById = (id) => S.state?.servers.find((s) => s.id === id);
export const activeServer = () => (S.state?.running ? serverById(S.state.active) : undefined);

export function pushLog(line) {
  S.logs.push(line);
  if (S.logs.length > 500) S.logs.shift();
  bus.emit("log", line);
}

export function listenToBackend() {
  const { listen } = tauri.event;
  listen("state-changed", () => refresh());
  listen("core-log", (e) => pushLog(e.payload));
  listen("health", () => refresh());
  listen("latency", (e) => {
    const { id, kind, ms, error } = e.payload;
    S.latency[id] = { ...S.latency[id], [kind]: ms, [`${kind}Error`]: error };
    bus.emit("latency", id);
  });
  listen("scan-hit", (e) => bus.emit("scan-hit", e.payload));
  listen("scan-progress", (e) => bus.emit("scan-progress", e.payload));
}

let lastTraffic = null;

/** Poll Xray's stats and keep a short history for the sparkline. */
export async function pollTraffic() {
  const t = S.traffic;
  if (!S.state?.running) {
    lastTraffic = null;
    if (t.upRate || t.downRate || t.history.length) {
      Object.assign(t, { up: 0, down: 0, upRate: 0, downRate: 0, history: [] });
      bus.emit("traffic");
    }
    return;
  }
  try {
    const now = Date.now();
    const { up, down } = await tauri.core.invoke("get_traffic");
    if (lastTraffic) {
      const dt = Math.max((now - lastTraffic.at) / 1000, 0.001);
      t.upRate = Math.max(up - lastTraffic.up, 0) / dt;
      t.downRate = Math.max(down - lastTraffic.down, 0) / dt;
      t.history.push(t.downRate);
      if (t.history.length > 60) t.history.shift();
    }
    Object.assign(t, { up, down });
    lastTraffic = { up, down, at: now };
    bus.emit("traffic");
  } catch {
    /* the stats API needs a moment after the core starts */
  }
}
