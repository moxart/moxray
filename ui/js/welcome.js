// First-run welcome.

import { h, icon, openModal, spinner } from "./dom.js";
import { t, setLang } from "./i18n.js";
import { S, attempt, patchSettings, refresh } from "./api.js";
import { openImport } from "./editor.js";
import { openSubscriptionEditor } from "./subs.js";

export function showWelcome(applyAppearance) {
  const body = h("div", { class: "welcome" });
  let handle;
  let reopening = false;

  const finish = async () => {
    await patchSettings({ onboarded: true });
    handle.close();
  };

  function render() {
    const st = S.state;
    const coreButton = h("button", { class: "btn btn-soft", disabled: st.core_found }, icon(st.core_found ? "check" : "download", 16),
      st.core_found ? t("coreReady") : t("download"));
    coreButton.onclick = async () => {
      coreButton.replaceChildren(spinner(), t("coreDownloading"));
      coreButton.disabled = true;
      const tag = await attempt("download_core");
      await refresh();
      if (!tag) render();
    };
    const step = (n, title, text, ...actions) =>
      h("div", { class: "welcome-step" },
        h("div", { class: "step-n" }, n),
        h("div", { class: "step-body" }, h("h4", {}, title), h("p", { class: "muted" }, text), h("div", { class: "inline" }, ...actions)));

    body.replaceChildren(
      h("p", { class: "welcome-lead" }, t("welcomeLead")),
      h("div", { class: "inline welcome-prefs" },
        h("div", { class: "segmented" }, [["en", "English"], ["fa", "فارسی"]].map(([code, label]) =>
          h("button", { class: st.settings.language === code ? "on" : "", onclick: async () => {
            await patchSettings({ language: code });
            applyAppearance();
            // The dialog's own title and buttons need the new language too.
            reopening = true;
            handle.close();
            showWelcome(applyAppearance);
          } }, label))),
        h("div", { class: "segmented" }, [["dark", "moon"], ["light", "sun"]].map(([theme, iconName]) =>
          h("button", { class: st.settings.theme === theme ? "on" : "", "aria-label": theme, onclick: async () => {
            await patchSettings({ theme });
            applyAppearance();
            render();
          } }, icon(iconName, 15))))),
      step("1", t("welcomeCore"), t("welcomeCoreText"), coreButton),
      step("2", t("welcomeServers"), t("welcomeServersText"),
        h("button", { class: "btn btn-soft", onclick: openImport }, icon("clipboard", 16), t("pasteLinks")),
        h("button", { class: "btn btn-ghost", onclick: () => openSubscriptionEditor() }, icon("rss", 16), t("addSubscription"))),
      step("3", t("welcomeConnect"), t("welcomeConnectText")));
  }
  render();

  handle = openModal({
    title: t("welcomeTitle"),
    body, wide: true,
    buttons: [{ label: t("startUsing"), primary: true, onClick: finish }],
    onClose: () => { if (!reopening && !S.state.settings.onboarded) patchSettings({ onboarded: true }).catch(() => {}); },
  });
}
