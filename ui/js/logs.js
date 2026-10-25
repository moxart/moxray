// Core log view.

import { h, icon, toast, copyText } from "./dom.js";
import { t } from "./i18n.js";
import { S, bus } from "./api.js";

function lineNode(line) {
  const level = /\[(error|fatal)\]|failed|error/i.test(line) ? "bad" : /\[warning\]|warn/i.test(line) ? "warn" : "";
  return h("div", { class: `log-line ${level}` }, line);
}

export function mountLogs(container) {
  const view = h("div", { class: "log-view", dir: "ltr" });
  const stickToBottom = () => view.scrollTop + view.clientHeight >= view.scrollHeight - 12;
  S.logs.forEach((line) => view.append(lineNode(line)));

  container.replaceChildren(
    h("header", { class: "view-head" },
      h("h1", {}, t("nav_logs")),
      h("div", { class: "head-actions" },
        h("button", { class: "btn btn-ghost", onclick: async () => {
          toast((await copyText(S.logs.join("\n"))) ? t("copied") : t("copyFailed"), "ok");
        } }, icon("copy", 16), t("copy")),
        h("button", { class: "btn btn-ghost", onclick: () => { S.logs.length = 0; view.replaceChildren(); } }, icon("trash", 16), t("clear")))),
    view);
  view.scrollTop = view.scrollHeight;

  return bus.on("log", (line) => {
    const stick = stickToBottom();
    view.append(lineNode(line));
    while (view.childNodes.length > 500) view.firstChild.remove();
    if (stick) view.scrollTop = view.scrollHeight;
  });
}
