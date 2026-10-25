// Subscriptions: list with quota/expiry, add/edit dialog.

import { h, icon, openModal, toast, confirmDialog, formatBytes, formatDateTime, formatDate, daysLeft } from "./dom.js";
import { t, getLang } from "./i18n.js";
import { S, bus, call, attempt, refresh } from "./api.js";

const AUTO_OPTIONS = [0, 6, 12, 24, 72];

export function openSubscriptionEditor(sub = null) {
  const name = h("input", { class: "input", type: "text", value: sub?.name ?? "", placeholder: t("subName") });
  const url = h("input", {
    class: "input", type: "text", dir: "ltr", spellcheck: false, value: sub?.url ?? "",
    placeholder: "https://…",
  });
  const mirrors = h("textarea", {
    class: "input mono", dir: "ltr", spellcheck: false, rows: 3,
    placeholder: t("mirrorsPlaceholder"),
  }, (sub?.mirrors ?? []).join("\n"));
  const auto = h("select", { class: "input" }, AUTO_OPTIONS.map((hours) =>
    h("option", { value: hours, selected: (sub?.auto_update_hours ?? 24) === hours },
      hours === 0 ? t("autoUpdateOff") : t("everyHours", { n: hours }))));

  const field = (label, input, hint) =>
    h("div", { class: "field" }, h("label", {}, label), input, hint ? h("small", { class: "hint" }, hint) : null);

  openModal({
    title: sub ? t("editSubscription") : t("addSubscription"),
    wide: true,
    body: h("div", { class: "form" },
      field(t("subName"), name),
      field(t("subUrl"), url),
      field(t("mirrors"), mirrors, t("mirrorsHint")),
      field(t("autoUpdate"), auto)),
    buttons: [
      { label: t("cancel") },
      {
        label: sub ? t("save") : t("addAndFetch"), primary: true,
        onClick: async () => {
          if (!url.value.trim()) return false;
          const args = {
            name: name.value,
            url: url.value,
            mirrors: mirrors.value.split("\n"),
            autoUpdateHours: Number(auto.value),
          };
          if (sub) {
            if ((await attempt("edit_subscription", { id: sub.id, ...args }, false)) === false) return false;
            toast(t("saved"), "ok");
          } else {
            const report = await attempt("add_subscription", args);
            if (!report) return false;
            toast(t("subscriptionAdded", report), "ok");
          }
          await refresh();
        },
      },
    ],
  });
}

function usageBlock(sub) {
  if (sub.total == null && sub.expire == null) return null;
  const parts = [];
  if (sub.total != null) {
    const used = sub.used ?? 0;
    const percent = Math.min(100, Math.round((used / sub.total) * 100));
    parts.push(h("div", { class: "usage" },
      h("div", { class: "usage-text" },
        h("span", {}, t("usageOf", { used: formatBytes(used), total: formatBytes(sub.total) })),
        h("span", { class: "muted" }, t("usageLeft", { left: formatBytes(Math.max(sub.total - used, 0)) }))),
      h("div", { class: "meter" }, h("span", {
        class: percent >= 95 ? "bad" : percent >= 80 ? "warn" : "ok", style: `width:${percent}%`,
      }))));
  }
  if (sub.expire != null) {
    const days = daysLeft(sub.expire);
    const label = days < 0 ? t("expired") : t("expiresIn", { n: days });
    parts.push(h("div", { class: `expiry ${days < 0 ? "bad" : days <= 7 ? "warn" : "muted"}` },
      icon(days <= 7 ? "alert" : "info", 14), label, ` · ${formatDate(sub.expire, getLang())}`));
  }
  return h("div", { class: "usage-block" }, parts);
}

function subCard(sub) {
  const count = S.state.servers.filter((s) => s.subscription === sub.id).length;
  const host = (() => { try { return new URL(sub.url).host; } catch { return sub.url; } })();
  const update = h("button", { class: "btn btn-soft btn-sm" }, icon("refresh", 14), t("update"));
  update.onclick = async () => {
    update.disabled = true;
    const message = await attempt("update_subscriptions", { id: sub.id });
    update.disabled = false;
    if (message) toast(message, "ok");
    await refresh();
  };
  return h("article", { class: "card sub-card" },
    h("div", { class: "sub-head" },
      h("div", { class: "sub-title" }, icon("rss", 18), h("h3", {}, sub.name)),
      h("div", { class: "sub-actions" },
        update,
        h("button", { class: "btn-icon", title: t("edit"), onclick: () => openSubscriptionEditor(sub) }, icon("edit", 16)),
        h("button", { class: "btn-icon danger", title: t("remove"), onclick: async () => {
          if (!(await confirmDialog(t("confirmRemoveSub", { name: sub.name, n: count })))) return;
          await attempt("remove_subscription", { id: sub.id });
          await refresh();
        } }, icon("trash", 16)))),
    h("div", { class: "sub-meta" },
      h("span", { class: "tag" }, t("serversInSub", { n: count })),
      h("span", { class: "muted", dir: "ltr", title: sub.url }, host),
      sub.mirrors.length ? h("span", { class: "tag" }, t("mirrorsCount", { n: sub.mirrors.length })) : null,
      sub.auto_update_hours ? h("span", { class: "tag" }, icon("refresh", 12), t("everyHours", { n: sub.auto_update_hours })) : null),
    usageBlock(sub),
    h("div", { class: "muted small" },
      sub.updated_at ? t("updatedAt", { when: formatDateTime(sub.updated_at, getLang()) }) : t("never")));
}

export function mountSubscriptions(container) {
  const list = h("div", { class: "sub-list" });
  const render = () => {
    list.replaceChildren(...(S.state.subscriptions.length
      ? S.state.subscriptions.map(subCard)
      : [h("div", { class: "empty card" },
          icon("rss", 40),
          h("h3", {}, t("noSubsTitle")),
          h("p", { class: "muted" }, t("noSubsText")),
          h("button", { class: "btn btn-primary", onclick: () => openSubscriptionEditor() }, icon("plus", 16), t("addSubscription")))]));
  };
  const updateAll = h("button", { class: "btn btn-ghost" }, icon("refresh", 16), t("updateAll"));
  updateAll.onclick = async () => {
    updateAll.disabled = true;
    const message = await attempt("update_subscriptions", { id: null });
    updateAll.disabled = false;
    if (message) toast(message, "ok");
    await refresh();
  };
  container.replaceChildren(
    h("header", { class: "view-head" },
      h("h1", {}, t("nav_subs")),
      h("div", { class: "head-actions" },
        updateAll,
        h("button", { class: "btn btn-soft", onclick: () => openSubscriptionEditor() }, icon("plus", 16), t("addSubscription")))),
    list);
  render();
  return bus.on("state", render);
}
