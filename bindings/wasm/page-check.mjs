// Checks index.html against styles.css for the one way the two disagree
// silently: an element the page hides with the `hidden` attribute that a rule
// in the stylesheet gives a `display`. The browser's `[hidden] { display:
// none }` is a user-agent rule, so the stylesheet's `display` wins and the
// element stays on the page, which is how the empty-state box came to cover
// the tree drawing. Declaring `[hidden]` here with `!important` settles it.
//
//   node page-check.mjs [<index.html> <styles.css>]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const [htmlPath = path.join(here, "index.html"), cssPath = path.join(here, "styles.css")] = process.argv.slice(2);
const html = fs.readFileSync(htmlPath, "utf8");
const css = fs.readFileSync(cssPath, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

// The elements carrying the attribute, by the id and classes a selector can
// name them with.
const hidden = [];
for (const [tag] of html.matchAll(/<[a-z][^>]*>/g)) {
  if (!/\shidden(\s|=|>|\/)/.test(tag)) continue;
  const id = /\sid="([^"]*)"/.exec(tag);
  const classes = /\sclass="([^"]*)"/.exec(tag);
  hidden.push({
    tag: tag.slice(0, 60),
    names: [...(id ? [`#${id[1]}`] : []), ...(classes ? classes[1].trim().split(/\s+/).map((c) => `.${c}`) : [])],
  });
}
if (hidden.length === 0) {
  console.error(`${htmlPath} has no element with the hidden attribute; this check is reading the wrong file`);
  process.exit(1);
}

// Every rule that sets `display`, as its selector list and whether the
// declaration is important.
const rules = [];
for (const [, selector, body] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
  const display = /(^|;)\s*display\s*:([^;]*)/.exec(body);
  if (display !== null) rules.push({ selector: selector.trim().replace(/\s+/g, " "), important: /!important/.test(display[2]) });
}

const settled = rules.some((rule) => rule.important && rule.selector.split(",").some((one) => one.trim() === "[hidden]"));
const failures = [];
for (const element of hidden) {
  for (const rule of rules) {
    if (rule.important) continue;
    // A name matches only where the selector ends it, so `.note` is not read
    // out of `.notes`.
    const named = element.names.some((name) => new RegExp(`${name.replace(/[.#]/g, "\\$&")}(?![\\w-])`).test(rule.selector));
    if (named && !settled) failures.push(`${element.tag} has the hidden attribute, and \`${rule.selector}\` gives it a display`);
  }
}

if (failures.length > 0) {
  console.error(failures.join("\n"));
  console.error(`add \`[hidden] { display: none !important; }\` to ${path.basename(cssPath)}`);
  process.exit(1);
}
console.log(`page-check: ${hidden.length} elements use the hidden attribute, and the stylesheet does not override it`);
