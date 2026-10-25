// Fails when a t("key") used in the UI is missing from a language, or the languages differ.
import { readdirSync, readFileSync } from "node:fs";
import { STRINGS } from "../ui/js/i18n.js";

const dir = new URL("../ui/js/", import.meta.url);
const used = new Set();
for (const file of readdirSync(dir).filter((f) => f.endsWith(".js"))) {
  const source = readFileSync(new URL(file, dir), "utf8");
  for (const match of source.matchAll(/\bt\(\s*"([A-Za-z_0-9]+)"/g)) used.add(match[1]);
}
// Keys built dynamically: nav_<view> and sort_<mode>.
["servers", "subs", "tools", "logs", "settings"].forEach((v) => used.add(`nav_${v}`));
["default", "delay", "stability", "name"].forEach((k) => used.add(`sort_${k}`));

let problems = 0;
for (const [lang, table] of Object.entries(STRINGS)) {
  for (const key of used) if (!(key in table)) { console.error(`${lang}: missing "${key}"`); problems++; }
}
for (const key of Object.keys(STRINGS.en)) if (!(key in STRINGS.fa)) { console.error(`fa: no translation for "${key}"`); problems++; }
for (const key of Object.keys(STRINGS.fa)) if (!(key in STRINGS.en)) { console.error(`en: no source for "${key}"`); problems++; }
console.log(problems ? `${problems} problem(s)` : `i18n ok: ${used.size} keys used, ${Object.keys(STRINGS.en).length} defined`);
process.exit(problems ? 1 : 0);
