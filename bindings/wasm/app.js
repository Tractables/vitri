// The page: the input, the settings, the run, and the panels that show the
// result. The parts that need no page are the modules imported here; the
// worker that runs vitri is worker.js.

import { drawScene, sceneSvg } from "./drawing.js";
import { EXAMPLES } from "./examples.js";
import { LEAF_HEIGHT, ROW, expandBelow, initialExpansion, layoutTree, nodeAt, sizesFor } from "./layout.js";
import { Runner } from "./runner.js";
import { TextView } from "./text-view.js";
import { dimacsHeader, isLeaf, isUnder, leafVariables, occurrences, parseVtree } from "./vtree.js";
import { ERROR_KINDS, MODE_NAMES, OUTCOMES, STAGE_NAMES, duration, megabytes, originalLiteral, originalToken, plural } from "./words.js";
import { zipStore } from "./zip.js";

// ------------------------------------------------------------------ limits

// Nodes drawn when a tree first appears. Deeper subtrees start collapsed, so
// a tree of any size opens as quickly as a small one.
const INITIAL_NODES = 1023;
// Nodes one "Expand below" adds at most.
const EXPAND_NODES = 40000;
// An input past this size stays out of the text box, which becomes slow on
// multi-megabyte text; it is held in memory and sent as it is.
const HELD_INPUT_BYTES = 1024 * 1024;
// Items in the list form of the drawn tree.
const OUTLINE_ITEMS = 2000;
// Leaf variables listed for an internal node.
const LISTED_LEAVES = 24;
// The largest PNG side and area the export draws; browsers refuse larger
// canvases.
const PNG_SIDE = 16384;
const PNG_AREA = 1.2e8;

// This file's version stamp (see README.md), passed on to the worker.
const stamp = new URL(import.meta.url).search;

const element = (id) => document.getElementById(id);
const make = (name, className, text) => {
  const node = document.createElement(name);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
};

const cnfBox = element("cnf");
const statusLine = element("status");
const treeView = element("tree-view");
const canvas = element("tree-canvas");
const fileView = new TextView(element("text-view"), element("text-spacer"), element("text-lines"));

// The input: the text box, or a large file held in memory with its name.
let held = null;
let inputName = null;
const inputText = () => (held === null ? cnfBox.value : held.text);

let capabilities = null;
// The result on show, or null.
let current = null;
// The tree view on show, or null.
let scene = null;

// ------------------------------------------------------------- examples

const examples = element("examples");
EXAMPLES.forEach(([, name], i) => {
  const chip = make("button", "", name);
  chip.type = "button";
  chip.dataset.example = String(i);
  chip.setAttribute("aria-pressed", "false");
  examples.append(chip);
});
const markExample = (i) => {
  for (const chip of examples.children) chip.setAttribute("aria-pressed", String(chip.dataset.example === String(i)));
};
const chosenExample = () => {
  const chip = examples.querySelector('[aria-pressed="true"]');
  return chip === null ? -1 : Number(chip.dataset.example);
};

let loading = 0;
async function loadExample(i) {
  const ticket = ++loading;
  markExample(i);
  let text;
  try {
    text = await EXAMPLES[i][2]();
  } catch (failure) {
    statusLine.textContent = `The example did not load: ${failure.message ?? failure}`;
    return;
  }
  if (ticket !== loading) return;
  setInput(text, null);
  markExample(i);
  requestRun();
}
examples.addEventListener("click", (event) => {
  const chip = event.target.closest("button");
  if (chip !== null) loadExample(Number(chip.dataset.example));
});

// ---------------------------------------------------------------- input

function setInput(text, name) {
  inputName = name;
  if (text.length > HELD_INPUT_BYTES) {
    held = { text };
    cnfBox.value = "";
    cnfBox.disabled = true;
    cnfBox.placeholder = "This file is too large to show here. It is held in memory and will be sent as it is; Clear removes it.";
    element("clear").hidden = false;
  } else {
    held = null;
    cnfBox.disabled = false;
    cnfBox.value = text;
    cnfBox.placeholder = "p cnf 3 2\n1 -2 0\n2 3 0";
    element("clear").hidden = true;
  }
  markExample(-1);
  describeInput();
  markStale();
}

function describeInput() {
  const text = inputText();
  const parts = [];
  if (inputName !== null) parts.push(inputName);
  if (held !== null || text.length > 64 * 1024) parts.push(megabytes(new TextEncoder().encode(text.slice(0, 1 << 24)).length + Math.max(0, text.length - (1 << 24))));
  const header = dimacsHeader(text);
  if (header !== null) parts.push(`${plural(header.variables, "variable")}, ${plural(header.clauses, "clause")} declared`);
  element("input-info").textContent = parts.join(", ");
}

let typing = 0;
cnfBox.addEventListener("input", () => {
  inputName = null;
  markExample(-1);
  markStale();
  clearTimeout(typing);
  typing = setTimeout(describeInput, 300);
});

element("clear").addEventListener("click", () => {
  setInput("", null);
  cnfBox.focus();
});

// A gzip file is decompressed in the tab; other compressed formats are
// refused by name rather than sent as bytes.
async function readFile(file) {
  const head = new Uint8Array(await file.slice(0, 6).arrayBuffer());
  const starts = (...bytes) => bytes.every((byte, i) => head[i] === byte);
  if (starts(0x1f, 0x8b)) {
    if (typeof DecompressionStream === "undefined") throw new Error("this browser cannot decompress gzip");
    return new Response(file.stream().pipeThrough(new DecompressionStream("gzip"))).text();
  }
  if (starts(0xfd, 0x37, 0x7a, 0x58, 0x5a)) throw new Error("it is xz-compressed; decompress it first");
  if (starts(0x42, 0x5a, 0x68)) throw new Error("it is bzip2-compressed; decompress it first");
  if (starts(0x50, 0x4b, 0x03, 0x04)) throw new Error("it is a zip archive; extract the CNF first");
  return file.text();
}

async function loadFile(file) {
  if (file === undefined) return;
  const ticket = ++loading;
  let text;
  try {
    text = await readFile(file);
  } catch (failure) {
    statusLine.textContent = `${file.name} was not read: ${failure.message ?? failure}`;
    return;
  }
  if (ticket !== loading) return;
  setInput(text, file.name);
  requestRun();
}

const fileInput = element("file");
element("open").addEventListener("click", () => fileInput.click());
fileInput.addEventListener("change", () => {
  loadFile(fileInput.files[0]);
  fileInput.value = "";
});

// The whole page takes a drop. Dragging over child elements fires leave and
// enter in pairs, so a count tells when the file has left the page.
let dragging = 0;
const hasFiles = (event) => event.dataTransfer !== null && Array.from(event.dataTransfer.types).includes("Files");
document.addEventListener("dragenter", (event) => {
  if (!hasFiles(event)) return;
  event.preventDefault();
  dragging += 1;
  document.body.classList.add("dropping");
});
document.addEventListener("dragover", (event) => {
  if (!hasFiles(event)) return;
  event.preventDefault();
  event.dataTransfer.dropEffect = "copy";
});
document.addEventListener("dragleave", (event) => {
  if (!hasFiles(event)) return;
  dragging = Math.max(0, dragging - 1);
  if (dragging === 0) document.body.classList.remove("dropping");
});
document.addEventListener("drop", (event) => {
  if (!hasFiles(event)) return;
  event.preventDefault();
  dragging = 0;
  document.body.classList.remove("dropping");
  loadFile(event.dataTransfer.files[0]);
});

// ------------------------------------------------------------- settings

const control = {
  mode: element("mode"),
  vtree: element("vtree"),
  budget: element("budget"),
  components: element("components"),
  candidates: element("candidates"),
  simplify: element("simplify"),
  arjun: element("arjun"),
};
// Whether this build takes a request key. A build that lists no keys is
// taken to accept the ones this page sends.
const accepts = (key) =>
  capabilities !== null && (!Array.isArray(capabilities.request_keys) || capabilities.request_keys.includes(key));
const available = (stage) =>
  accepts(stage) && capabilities.stages !== undefined && capabilities.stages[stage] === true;
// Each setting, with the request key it fills.
const SETTING_KEYS = [
  ["mode", "mode"], ["vtree", "vtree"], ["budget", "budget_ms"], ["components", "components"],
  ["candidates", "candidates"], ["simplify", "simplify"], ["arjun", "arjun"],
];
// Settings from the address, kept until the capabilities arrive.
let addressSettings = null;

function setUpControls() {
  const option = (select, value, text) => {
    const item = make("option", "", text);
    item.value = value;
    select.append(item);
  };
  control.mode.replaceChildren();
  option(control.mode, "", "detected from the file");
  for (const mode of capabilities.modes ?? []) option(control.mode, mode, MODE_NAMES[mode] ? `${mode}: ${MODE_NAMES[mode]}` : mode);
  const bases = element("vtree-bases");
  bases.replaceChildren();
  for (const base of capabilities.vtree_bases ?? []) option(bases, base, "");
  control.vtree.value = capabilities.default_vtree ?? "portfolio";
  control.components.replaceChildren();
  for (const policy of capabilities.components ?? []) option(control.components, policy, policy);
  control.candidates.max = String(capabilities.max_candidates ?? 1);
  for (const stage of ["simplify", "arjun"]) control[stage].checked = available(stage);
  // A setting this build does not take cannot be changed, and is not sent.
  for (const [name, key] of SETTING_KEYS) {
    if (!accepts(key)) {
      control[name].disabled = true;
      control[name].title = "This build of vitri does not take this setting.";
    }
  }
  if (addressSettings !== null) applySettings(addressSettings);
  addressSettings = null;
  updateStages();
}

// Whether a mode's preprocessing has a stage. With no mode chosen the file
// decides the mode, so the switch stays open and vitri checks it.
const modeHas = (mode, stage) => {
  const table = capabilities.mode_stages;
  if (mode === "" || table === undefined || table === null || table[mode] === undefined) return true;
  return table[mode][stage] === true;
};
// A stage switch can be changed, and is sent, only when this build has the
// stage and the chosen mode's preprocessing has it.
const switchable = (stage) => available(stage) && modeHas(control.mode.value, stage);
const STAGE_LABELS = { simplify: "simplify", arjun: "Arjun" };

// A locked switch shows the stage off and remembers the choice it had, which
// comes back when a mode with the stage is chosen again.
function updateStages() {
  const reasons = [];
  for (const stage of ["simplify", "arjun"]) {
    const box = control[stage];
    const open = switchable(stage);
    if (open && box.disabled && box.dataset.wanted !== undefined) {
      box.checked = box.dataset.wanted === "1";
      delete box.dataset.wanted;
    } else if (!open && !box.disabled) {
      box.dataset.wanted = box.checked ? "1" : "0";
      box.checked = false;
    }
    box.disabled = !open;
    if (!accepts(stage)) continue;
    if (capabilities.stages?.[stage] !== true) {
      reasons.push(stage === "arjun" ? "This build of vitri does not include Arjun." : "This build of vitri has no simplify stage.");
    } else if (!open) {
      reasons.push(`Mode ${control.mode.value} has no ${STAGE_LABELS[stage]} stage.`);
    }
  }
  const note = element("arjun-note");
  note.hidden = reasons.length === 0;
  note.textContent = reasons.join(" ");
}

function applySettings(params) {
  const get = (key) => params.get(key);
  const pick = (select, value) => {
    if (value !== null && [...select.options].some((item) => item.value === value)) select.value = value;
  };
  pick(control.mode, get("mode"));
  if (get("vtree") !== null && get("vtree").length <= 200) control.vtree.value = get("vtree");
  if (get("budget") !== null && /^\d{1,9}$/.test(get("budget"))) control.budget.value = get("budget");
  pick(control.components, get("components"));
  if (get("candidates") !== null && /^\d{1,3}$/.test(get("candidates"))) control.candidates.value = get("candidates");
  for (const stage of ["simplify", "arjun"]) {
    const value = get(stage);
    if (available(stage) && (value === "0" || value === "1")) control[stage].checked = value === "1";
  }
}

// The request for the worker: only what differs from vitri's defaults, so
// the summary's echo of the effective settings stays the one record of
// them. Returns {request} or {error}.
function buildRequest() {
  const request = { format: capabilities.request_format ?? "vitri-request-v1" };
  if (control.mode.value !== "") request.mode = control.mode.value;
  const spec = control.vtree.value.trim();
  if (spec !== "" && spec !== capabilities.default_vtree) request.vtree = spec;
  const budget = control.budget.value.trim();
  if (budget !== "" || control.budget.validity.badInput) {
    const value = Number(budget);
    if (!Number.isSafeInteger(value) || value < 1) {
      return { error: "Budget is a whole number of milliseconds, 1 or more. Leave it empty for no budget." };
    }
    request.budget_ms = value;
  }
  if (control.components.value !== "") request.components = control.components.value;
  const most = capabilities.max_candidates ?? 1;
  const candidates = Number(control.candidates.value);
  if (!Number.isInteger(candidates) || candidates < 1 || candidates > most) {
    return { error: `Candidates is a whole number from 1 to ${most}.` };
  }
  if (candidates !== 1) request.candidates = candidates;
  for (const stage of ["simplify", "arjun"]) {
    // Only a change from the default, on, is sent.
    if (switchable(stage) && !control[stage].checked) request[stage] = false;
  }
  for (const key of Object.keys(request)) {
    if (key !== "format" && !accepts(key)) delete request[key];
  }
  return { request };
}

element("settings").addEventListener("input", markStale);
control.mode.addEventListener("change", updateStages);
element("settings").addEventListener("submit", (event) => {
  event.preventDefault();
  requestRun();
});

// The address carries the example and the settings, so a result can be
// linked to. It is read once when the page opens and written at every run.
// A formula typed or opened is not in the address, so a run on one keeps
// only the settings there and hides the button that copies it.
function writeAddress() {
  const params = new URLSearchParams();
  const example = chosenExample();
  if (example >= 0) params.set("example", EXAMPLES[example][0]);
  if (control.mode.value !== "") params.set("mode", control.mode.value);
  if (control.vtree.value.trim() !== "" && control.vtree.value.trim() !== capabilities.default_vtree) {
    params.set("vtree", control.vtree.value.trim());
  }
  if (control.budget.value.trim() !== "") params.set("budget", control.budget.value.trim());
  if (capabilities.components && control.components.value !== capabilities.components[0]) {
    params.set("components", control.components.value);
  }
  if (control.candidates.value !== "1") params.set("candidates", control.candidates.value);
  for (const stage of ["simplify", "arjun"]) {
    if (switchable(stage) && !control[stage].checked) params.set(stage, "0");
  }
  const query = params.toString();
  history.replaceState(null, "", query === "" ? location.pathname : `${location.pathname}?${query}`);
  element("link").hidden = example < 0;
}

function flash(button, text) {
  const label = button.textContent;
  button.textContent = text;
  setTimeout(() => {
    button.textContent = label;
  }, 1500);
}
element("link").addEventListener("click", () => {
  navigator.clipboard.writeText(location.href).then(
    () => flash(element("link"), "Copied"),
    () => flash(element("link"), "Not copied"),
  );
});

// ---------------------------------------------------------------- runs

// What the run on show was sent, for the file names and the summary.
let sent = null;
// A run asked for before the capabilities arrived; it is asked for again
// once they have, so its request is built from them.
let pending = false;
let ticking = 0;

const runner = new Runner("worker.js" + stamp, {
  loading() {
    element("run").disabled = true;
  },
  ready(found) {
    const first = capabilities === null;
    capabilities = found;
    if (first) setUpControls();
    element("run").disabled = false;
    statusLine.textContent = "Ready.";
    if (pending) {
      pending = false;
      requestRun();
    }
  },
  notLoaded(message) {
    statusLine.textContent = `vitri did not load: ${message}`;
  },
  started() {
    const started = performance.now();
    statusLine.textContent = "Running.";
    ticking = setInterval(() => {
      statusLine.textContent = `Running, ${Math.round((performance.now() - started) / 1000)} s.`;
    }, 1000);
    element("run").hidden = true;
    element("cancel").hidden = false;
    document.body.classList.add("busy");
  },
  done(result, elapsed) {
    settle();
    finish(result, elapsed);
  },
  failed(error) {
    settle();
    showFailure(error);
  },
});

// The page after a run, whichever way it ended.
function settle() {
  clearInterval(ticking);
  document.body.classList.remove("busy");
  element("cancel").hidden = true;
  element("run").hidden = false;
}

function requestRun() {
  const text = inputText();
  if (text.trim() === "") {
    statusLine.textContent = "Paste, open or drop a DIMACS CNF first.";
    return;
  }
  if (capabilities === null) {
    statusLine.textContent = "Loading vitri; the run starts when it is ready.";
    pending = true;
    return;
  }
  const built = buildRequest();
  if (built.error !== undefined) {
    statusLine.textContent = built.error;
    return;
  }
  writeAddress();
  if (runner.running) settle();
  sent = { request: built.request, name: inputName, example: chosenExample() };
  runner.run(text, built.request);
}

element("cancel").addEventListener("click", () => {
  if (!runner.cancel()) return;
  settle();
  statusLine.textContent = "Stopped. Loading vitri again.";
});

function markStale() {
  if (current !== null) element("stale").hidden = false;
}

// ---------------------------------------------------------------- result

const parseJson = (text) => {
  if (typeof text !== "string") return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
};

function finish(result, elapsed) {
  element("stale").hidden = true;
  const summary = result === null || typeof result !== "object" ? null : result.summary;
  if (summary === null || typeof summary !== "object" || summary.format !== "vitri-result-v1") {
    showFailure({ kind: "format", message: `the result format ${JSON.stringify(summary?.format)} is not one this page reads` });
    return;
  }
  const files = result.files ?? {};
  const record = parseJson(files["preprocess.json"]);
  let manifest = parseJson(files["components.json"]);
  let manifestNote = "";
  if (manifest !== null && !/^vitri-components-v[12]$/.test(manifest.format)) {
    manifestNote = `components.json has format ${JSON.stringify(manifest.format)}, which this page does not read.`;
    manifest = null;
  }
  current = { summary, files, elapsed, record, manifest, manifestNote, sent, trees: new Map(), scenes: new Map(), counts: new Map() };
  statusLine.textContent = `Done in ${duration(elapsed)}.`;
  renderSummary();
  renderExports();
  element("panels").hidden = false;
  fillFileChoice();
  fillTreeChoice();
}

function showFailure(error) {
  element("stale").hidden = true;
  current = null;
  scene = null;
  const kind = error && typeof error.kind === "string" ? error.kind : "worker";
  const message = error && typeof error.message === "string" ? error.message : String(error);
  statusLine.textContent = "Not prepared; see the result.";
  const box = element("summary");
  const line = make("div", "status-line failed");
  const heading = make("p");
  heading.style.margin = "0";
  heading.append(make("strong", "", "Not prepared: "), document.createTextNode(`${ERROR_KINDS[kind] ?? kind}.`));
  line.append(heading, make("pre", "", message));
  box.replaceChildren(line);
  element("exports").hidden = true;
  element("panels").hidden = true;
}

function renderSummary() {
  const { summary, manifest, manifestNote } = current;
  const box = element("summary");
  const status = summary.status;
  const line = make("div", `status-line ${status}`);
  const p = make("p");
  p.style.margin = "0";
  if (status === "built") {
    const variables = summary.reduced?.variables ?? 0;
    p.append(make("strong", "", "Built. "), document.createTextNode(
      `Preprocessing left ${plural(variables, "variable")} to compile, and vitri built a vtree over ${variables === 1 ? "it" : "them"}.`,
    ));
  } else if (status === "fully_resolved") {
    p.append(make("strong", "", "Fully resolved. "), document.createTextNode(
      "Preprocessing settled every variable, so there is nothing to compile and no vtree. " +
        "The count of the original formula is the lift below, and the bundle is reduced.cnf and preprocess.json alone.",
    ));
  } else if (status === "refuted") {
    p.append(make("strong", "", "Refuted. "), document.createTextNode(
      "Preprocessing proved the formula unsatisfiable, so its count is 0 and there is nothing to compile. " +
        "reduced.cnf holds an explicit contradiction (x and not x), because DIMACS has no portable empty clause; " +
        "the bundle is reduced.cnf and preprocess.json alone.",
    ));
  } else {
    p.append(make("strong", "", `Status ${status}.`));
  }
  line.append(p);

  const stats = make("div", "stats");
  const stat = (value, word, from) => {
    const item = make("span", "stat");
    const figure = make("b", "", value);
    item.append(figure, document.createTextNode(word));
    if (from !== undefined) item.append(make("span", "from", from));
    stats.append(item);
  };
  const count = (n) => (typeof n === "number" ? n.toLocaleString("en-US") : "?");
  stat(count(summary.reduced?.variables), "variables", `of ${count(summary.input?.variables)}`);
  stat(count(summary.reduced?.clauses), "clauses", `of ${count(summary.input?.clauses)}`);
  if (summary.vtree) {
    stat(count(summary.vtree.components), summary.vtree.components === 1 ? "component" : "components");
    stat(count(summary.vtree.free_variables), summary.vtree.free_variables === 1 ? "free variable" : "free variables");
  }
  stat(duration(current.elapsed), "");

  const facts = make("dl", "facts");
  const fact = (term, ...values) => {
    facts.append(make("dt", "", term));
    const dd = make("dd");
    for (const value of values) dd.append(typeof value === "string" ? document.createTextNode(value) : value);
    facts.append(dd);
  };
  const mode = summary.mode;
  const detected = current.sent !== null && current.sent.request.mode === undefined;
  fact("Mode", `${mode}${MODE_NAMES[mode] ? `, ${MODE_NAMES[mode]}` : ""}${detected ? " (detected from the file)" : ""}`);

  const lift = summary.lift ?? {};
  const liftBox = make("span");
  if (status === "refuted") {
    liftBox.append(make("span", "lift", "count(original) = 0"));
  } else if (status === "fully_resolved") {
    liftBox.append(make("span", "lift", `count(original) = ${lift.factor}`));
  } else {
    liftBox.append(make("span", "lift", `count(original) = count(reduced) × ${lift.factor}`));
    const notes = ["count(reduced) comes from your compiler or model counter; vitri does not count."];
    if (typeof mode === "string" && mode.includes("w") && mode !== "compile") {
      notes.push("Take it under the weights written in reduced.cnf, not the input's.");
    }
    if (typeof mode === "string" && mode.startsWith("p")) {
      notes.push("Take it over the show set written in reduced.cnf, not the input's.");
    }
    liftBox.append(make("br"), make("span", "note", notes.join(" ")));
  }
  fact("Lift", liftBox);
  if (lift.count_lift_pow2 !== undefined) {
    fact("Recorded as", `count_lift_pow2 ${lift.count_lift_pow2}, weight_lift ${lift.weight_lift}`);
  }

  // A null outcome is a stage this run's chain did not reach or does not
  // have in this mode; only the others are listed.
  const reached = Object.entries(summary.stages ?? {}).filter(([, outcome]) => outcome !== null);
  fact("Stages", reached.length === 0
    ? "none ran"
    : reached.map(([stage, outcome]) => `${STAGE_NAMES[stage] ?? stage}: ${OUTCOMES[outcome] ?? outcome}`).join("; "));

  if (summary.vtree && manifest !== null) {
    const winners = new Map();
    for (const entry of manifest.components ?? []) {
      const spec = entry.selection?.winning_spec;
      if (spec) winners.set(spec, (winners.get(spec) ?? 0) + 1);
    }
    const chosen = [...winners].map(([spec, times]) => (manifest.components.length > 1 ? `${spec} (${times})` : spec));
    fact("Construction", `${summary.request?.vtree ?? "?"}${chosen.length ? `, selected ${chosen.join(", ")}` : ""}`);
    fact("Vtree", `${plural(summary.vtree.leaves, "leaf")}, ${plural(summary.vtree.nodes, "node")}`);
  }
  if (manifestNote !== "") fact("Components", manifestNote);
  const request = summary.request ?? {};
  const settings = [
    `budget ${request.budget_ms === null || request.budget_ms === undefined ? "unbounded" : `${request.budget_ms} ms`}`,
    `components ${request.components}`,
    `candidates ${request.candidates}`,
  ];
  // A stage this build or this mode lacks does not run whatever the request says.
  for (const stage of ["simplify", "arjun"]) {
    if (available(stage) && modeHas(summary.mode, stage)) settings.push(`${STAGE_LABELS[stage]} ${request[stage] ? "on" : "off"}`);
  }
  fact("Settings used", settings.join(", "));
  fact("vitri", String(summary.vitri_version ?? "?"));
  box.replaceChildren(line, stats, facts);
}

// ---------------------------------------------------------------- saving

function saveBlob(blob, name) {
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = name;
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(link.href), 5000);
}

// The stem the saved files are named after: the file that was opened, the
// example, or "formula".
function stem() {
  const sentBy = current.sent;
  if (sentBy !== null && sentBy.name) return sentBy.name.replace(/\.(cnf|dimacs|txt)(\.gz)?$/i, "").replace(/[^\w.-]+/g, "_") || "formula";
  if (sentBy !== null && sentBy.example >= 0) return EXAMPLES[sentBy.example][0];
  return "formula";
}

function bundlePaths() {
  const order = ["reduced.cnf", "preprocess.json", "vtree.vtree", "components.json"];
  const paths = Object.keys(current.files);
  return [...order.filter((path) => paths.includes(path)), ...paths.filter((path) => !order.includes(path)).sort()];
}

function renderExports() {
  const exports = element("exports");
  exports.hidden = false;
  for (const button of exports.querySelectorAll("button")) {
    const what = button.dataset.save;
    if (what === "zip") button.disabled = false;
    else if (what === "svg" || what === "png") button.disabled = current.summary.status !== "built";
    else button.disabled = typeof current.files[what] !== "string";
  }
}

element("exports").addEventListener("click", async (event) => {
  const button = event.target.closest("button");
  if (button === null || current === null) return;
  const what = button.dataset.save;
  try {
    if (what === "zip") {
      const base = stem();
      const zip = zipStore(bundlePaths().map((path) => ({ name: `${base}/${path}`, data: current.files[path] })));
      saveBlob(new Blob([zip], { type: "application/zip" }), `${base}.zip`);
    } else if (what === "svg") {
      if (scene === null) return;
      const text = sceneSvg(scene, readColors(), `${scene.view.path}`);
      saveBlob(new Blob([text], { type: "image/svg+xml" }), `${stem()}-${scene.view.fileStem}.svg`);
    } else if (what === "png") {
      if (scene === null) return;
      const blob = await scenePng(scene);
      saveBlob(blob, `${stem()}-${scene.view.fileStem}.png`);
    } else if (typeof current.files[what] === "string") {
      saveBlob(new Blob([current.files[what]], { type: "text/plain" }), what);
    }
  } catch (failure) {
    flash(button, "Not saved");
    statusLine.textContent = `Not saved: ${failure.message ?? failure}`;
  }
});

function scenePng(shown) {
  const pad = 20;
  const world = { w: shown.layout.width + 2 * pad, h: shown.layout.height + 2 * pad };
  const scale = Math.min(2, PNG_SIDE / world.w, PNG_SIDE / world.h, Math.sqrt(PNG_AREA / (world.w * world.h)));
  const image = document.createElement("canvas");
  image.width = Math.max(1, Math.floor(world.w * scale));
  image.height = Math.max(1, Math.floor(world.h * scale));
  const context = image.getContext("2d");
  const colors = readColors();
  context.fillStyle = colors.card;
  context.fillRect(0, 0, image.width, image.height);
  context.setTransform(scale, 0, 0, scale, pad * scale, (pad + LEAF_HEIGHT / 2) * scale);
  drawScene(context, shown, colors, { x0: -Infinity, y0: -Infinity, x1: Infinity, y1: Infinity }, scale);
  return new Promise((resolve, reject) => {
    image.toBlob((blob) => (blob === null ? reject(new Error("the browser did not make the image")) : resolve(blob)), "image/png");
  });
}

// ------------------------------------------------------------ file view

function fillFileChoice() {
  const select = element("file-choice");
  select.replaceChildren();
  for (const path of bundlePaths()) {
    const item = make("option", "", path);
    item.value = path;
    select.append(item);
  }
  select.value = "reduced.cnf";
  showFile(select.value);
}

function showFile(path) {
  const content = current.files[path] ?? "";
  const lines = fileView.show(content);
  element("file-info").textContent = `${plural(lines, "line")}, ${megabytes(new Blob([content]).size)}`;
}

element("file-choice").addEventListener("change", (event) => showFile(event.target.value));

// ------------------------------------------------------------ tree views

// The vtrees a result offers: the whole formula's, each component's own
// when there are several, and the retained candidates.
function treeViews() {
  const { summary, files, manifest } = current;
  const views = [];
  if (summary.status !== "built" || typeof files["vtree.vtree"] !== "string") return views;
  views.push({ key: "whole", path: "vtree.vtree", label: "whole formula (vtree.vtree)", fileStem: "vtree", component: -1, cnf: "reduced.cnf" });
  const entries = manifest === null ? [] : manifest.components ?? [];
  entries.forEach((entry, i) => {
    const name = `component ${String(i).padStart(3, "0")}`;
    if (entry.vtree !== "vtree.vtree" && typeof files[entry.vtree] === "string") {
      const spec = entry.selection?.winning_spec ? `, ${entry.selection.winning_spec}` : "";
      views.push({
        key: `c${i}`, path: entry.vtree, component: i, cnf: entry.cnf,
        label: `${name}: ${plural(entry.local_to_reduced_dimacs.length, "variable")}${spec}`,
        fileStem: entry.vtree.replace(/^.*\//, "").replace(/\.vtree$/, ""),
      });
    }
    (entry.vtree_candidates ?? []).forEach((candidate, rank) => {
      if (rank === 0 || typeof files[candidate.vtree] !== "string") return;
      views.push({
        key: `c${i}r${rank}`, path: candidate.vtree, component: i, cnf: entry.cnf, builtBy: candidate.built_by,
        label: `${entries.length > 1 ? `${name} ` : ""}candidate ${rank}: ${(candidate.built_by ?? []).join(", ")}`,
        fileStem: candidate.vtree.replace(/^.*\//, "").replace(/\.vtree$/, ""),
      });
    });
  });
  return views;
}

function fillTreeChoice() {
  const select = element("tree-choice");
  select.replaceChildren();
  const views = treeViews();
  current.views = views;
  for (const view of views) {
    const item = make("option", "", view.label);
    item.value = view.key;
    select.append(item);
  }
  select.disabled = views.length <= 1;
  const empty = element("tree-empty");
  const zoomButtons = element("zoom").querySelectorAll("button");
  if (views.length === 0) {
    scene = null;
    empty.hidden = false;
    empty.textContent = current.summary.status === "refuted"
      ? "No vtree: preprocessing refuted the formula."
      : current.summary.status === "fully_resolved"
        ? "No vtree: preprocessing resolved every variable."
        : "No vtree in this result.";
    zoomButtons.forEach((button) => (button.disabled = true));
    element("details").hidden = true;
    element("tree-description").textContent = empty.textContent;
    element("outline").replaceChildren();
    clearCanvas();
    return;
  }
  zoomButtons.forEach((button) => (button.disabled = false));
  element("details").hidden = false;
  showTree(views[0]);
}

element("tree-choice").addEventListener("change", (event) => {
  const view = current.views.find((item) => item.key === event.target.value);
  if (view === undefined) return;
  showTree(view);
  if (typeof current.files[view.cnf] === "string") {
    element("file-choice").value = view.cnf;
    showFile(view.cnf);
  }
});

// Component of every node of the whole-formula tree: a leaf takes its
// variable's component, or -2 for a free variable; an internal node takes
// its children's when they agree and -1 when they do not, which marks the
// nodes that join components.
function componentsOf(tree) {
  const { manifest } = current;
  if (manifest === null) return null;
  const entries = manifest.components ?? [];
  const free = manifest.free_vars_reduced_dimacs ?? [];
  if (entries.length <= 1 && free.length === 0) return null;
  const of = new Map();
  entries.forEach((entry, i) => entry.local_to_reduced_dimacs.forEach((reduced) => of.set(reduced, i)));
  for (const reduced of free) of.set(reduced, -2);
  const component = new Int32Array(tree.count);
  for (let i = tree.count - 1; i >= 0; i--) {
    const node = tree.order[i];
    if (isLeaf(tree, node)) component[node] = of.get(tree.variable[node]) ?? -1;
    else {
      const a = component[tree.left[node]];
      component[node] = a >= 0 && a === component[tree.right[node]] ? a : -1;
    }
  }
  return component;
}

function showTree(view) {
  element("tree-empty").hidden = true;
  let shown = current.scenes.get(view.key);
  if (shown === undefined) {
    const tree = parseVtree(current.files[view.path]);
    if (tree.error !== undefined) {
      scene = null;
      const empty = element("tree-empty");
      empty.hidden = false;
      empty.textContent = `${view.path} was not read: ${tree.error}.`;
      element("tree-description").textContent = empty.textContent;
      clearCanvas();
      return;
    }
    const sizes = sizesFor(tree);
    const room = Math.max(300, treeView.clientWidth);
    const budget = Math.min(INITIAL_NODES, Math.max(63, 2 * Math.floor(room / (sizes.slot * 0.6))));
    shown = {
      view, tree, sizes,
      expanded: initialExpansion(tree, budget),
      budget,
      x: new Float64Array(tree.count),
      component: view.component === -1 ? componentsOf(tree) : null,
      selected: -1,
      onPath: new Uint8Array(tree.count),
      scale: 1, tx: 0, ty: 0,
      fitted: false,
    };
    shown.layout = layoutTree(tree, shown.expanded, sizes, shown.x);
    current.scenes.set(view.key, shown);
  }
  scene = shown;
  if (!scene.fitted) {
    fit();
    scene.fitted = true;
  }
  element("tree-choice").value = view.key;
  describeTree();
  renderDetails();
  draw();
}

// --------------------------------------------------------- tree drawing

let colorCache = null;
function readColors() {
  if (colorCache !== null) return colorCache;
  const style = getComputedStyle(document.documentElement);
  const token = (name) => style.getPropertyValue(name).trim();
  colorCache = {
    card: token("--card"),
    code: token("--code"),
    ink: token("--ink"),
    muted: token("--muted"),
    accent: token("--accent"),
    edge: token("--edge"),
    pick: token("--pick"),
    mono: token("--mono"),
    components: [1, 2, 3, 4, 5, 6, 7, 8].map((i) => token(`--comp-${i}`)),
  };
  return colorCache;
}
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
  colorCache = null;
  draw();
});

function clearCanvas() {
  const context = canvas.getContext("2d");
  context.setTransform(1, 0, 0, 1, 0, 0);
  context.clearRect(0, 0, canvas.width, canvas.height);
}

let drawQueued = false;
function scheduleDraw() {
  if (drawQueued) return;
  drawQueued = true;
  requestAnimationFrame(() => {
    drawQueued = false;
    draw();
  });
}

function draw() {
  if (scene === null) return;
  const ratio = window.devicePixelRatio || 1;
  const width = treeView.clientWidth;
  const height = treeView.clientHeight;
  if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
  }
  const context = canvas.getContext("2d");
  const colors = readColors();
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.fillStyle = colors.code;
  context.fillRect(0, 0, width, height);
  const { scale, tx, ty } = scene;
  context.setTransform(ratio * scale, 0, 0, ratio * scale, ratio * tx, ratio * ty);
  const bounds = { x0: -tx / scale, y0: -ty / scale, x1: (width - tx) / scale, y1: (height - ty) / scale };
  drawScene(context, scene, colors, bounds, scale);
  element("zoom-level").textContent = `${Math.round(scale * 100)}%`;
}

const MIN_SCALE = 0.0005;
const MAX_SCALE = 6;

function fit() {
  const width = treeView.clientWidth || 600;
  const height = treeView.clientHeight || 400;
  const pad = 24;
  const { layout } = scene;
  const scale = Math.min(1.5, (width - 2 * pad) / Math.max(1, layout.width), (height - 2 * pad) / Math.max(1, layout.height));
  scene.scale = Math.max(MIN_SCALE, scale);
  scene.tx = (width - layout.width * scene.scale) / 2;
  scene.ty = (height - layout.height * scene.scale) / 2 + (LEAF_HEIGHT / 2) * scene.scale;
}

function zoomAt(factor, sx, sy) {
  const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, scene.scale * factor));
  const wx = (sx - scene.tx) / scene.scale;
  const wy = (sy - scene.ty) / scene.scale;
  scene.scale = scale;
  scene.tx = sx - wx * scale;
  scene.ty = sy - wy * scale;
  scheduleDraw();
}

// Re-lays the tree after an expansion changed, keeping `anchor` where it
// was on screen.
function relayout(anchor) {
  const before = anchor >= 0 ? scene.x[anchor] * scene.scale + scene.tx : 0;
  scene.layout = layoutTree(scene.tree, scene.expanded, scene.sizes, scene.x);
  if (anchor >= 0) scene.tx = before - scene.x[anchor] * scene.scale;
  describeTree();
  scheduleDraw();
}

function toggle(node) {
  const { tree, expanded } = scene;
  if (node < 0 || isLeaf(tree, node)) return;
  if (expanded[node] === 1) {
    expanded[node] = 0;
    if (scene.selected >= 0 && scene.selected !== node && isUnder(tree, scene.selected, node)) select(node, false);
    announce(`Collapsed node ${node}; ${plural(tree.leaves[node], "leaf")} below it.`);
  } else {
    expanded[node] = 1;
    announce(`Expanded node ${node}.`);
  }
  relayout(node);
  renderDetails();
}

function expandSelected() {
  const node = scene.selected >= 0 ? scene.selected : scene.tree.root;
  const { added, stopped } = expandBelow(scene.tree, scene.expanded, node, EXPAND_NODES);
  relayout(node);
  announce(stopped
    ? `Expanded ${plural(added, "node")} below node ${node}; deeper subtrees are still collapsed.`
    : `Expanded everything below node ${node}, ${plural(added, "node")}.`);
  renderDetails();
}

function resetView() {
  scene.expanded = initialExpansion(scene.tree, scene.budget);
  scene.layout = layoutTree(scene.tree, scene.expanded, scene.sizes, scene.x);
  let node = scene.selected;
  while (node >= 0 && !isDrawn(node)) node = scene.tree.parent[node];
  if (scene.selected >= 0) select(node, false);
  fit();
  describeTree();
  renderDetails();
  scheduleDraw();
}

// Whether a node is drawn: every node above it is expanded.
function isDrawn(node) {
  for (let up = scene.tree.parent[node]; up >= 0; up = scene.tree.parent[up]) {
    if (scene.expanded[up] === 0) return false;
  }
  return true;
}

function select(node, keepInView = true) {
  const { tree, onPath } = scene;
  for (let at = scene.selected; at >= 0; at = tree.parent[at]) onPath[at] = 0;
  scene.selected = node;
  for (let at = node; at >= 0; at = tree.parent[at]) onPath[at] = 1;
  if (node >= 0 && keepInView) {
    const width = treeView.clientWidth;
    const height = treeView.clientHeight;
    const sx = scene.x[node] * scene.scale + scene.tx;
    const sy = tree.depth[node] * ROW * scene.scale + scene.ty;
    if (sx < 40 || sx > width - 40) scene.tx += width / 2 - sx;
    if (sy < 40 || sy > height - 60) scene.ty += height / 2 - sy;
  }
  renderDetails();
  if (node >= 0) announce(nodeSentence(node));
  scheduleDraw();
}

const announce = (sentence) => {
  element("tree-live").textContent = sentence;
};

function nodeSentence(node) {
  const { tree } = scene;
  if (isLeaf(tree, node)) {
    const chain = variableChain(tree.variable[node]);
    return `Leaf node ${node} at depth ${tree.depth[node]}: ${chain.map(([space, value]) => `${space} ${value}`).join(", ")}.`;
  }
  const state = scene.expanded[node] === 1 ? "expanded" : "collapsed";
  return `Internal node ${node} at depth ${tree.depth[node]}, ${state}, ${plural(tree.leaves[node], "leaf")} below.`;
}

// What a leaf's variable is in each numbering: [space, text] pairs from the
// tree's own space out to the original formula.
function variableChain(variable) {
  const { view } = scene;
  const { record, manifest } = current;
  if (view.component === -1) {
    return [["reduced variable", String(variable)], ["original", originalLiteral(record, variable)]];
  }
  const entry = manifest.components[view.component];
  const map = entry.local_to_reduced_dimacs;
  if (variable < 1 || variable > map.length) {
    return [["local variable", String(variable)], ["reduced", "outside local_to_reduced_dimacs"]];
  }
  const reduced = map[variable - 1];
  return [["local variable", String(variable)], ["reduced variable", String(reduced)], ["original", originalLiteral(record, reduced)]];
}

function occurrenceCounts() {
  const { view, tree } = scene;
  const key = view.cnf;
  if (!current.counts.has(key)) {
    const content = current.files[key];
    const header = typeof content === "string" ? dimacsHeader(content) : null;
    current.counts.set(key, header === null ? null : occurrences(content, Math.max(header.variables, tree.maxVariable)));
  }
  return current.counts.get(key);
}

function renderDetails() {
  const list = element("details-list");
  const rows = [];
  const row = (term, value, className) => {
    rows.push(make("dt", "", term));
    const dd = make("dd", className);
    if (typeof value === "string") dd.textContent = value;
    else dd.append(value);
    rows.push(dd);
  };
  if (scene === null || scene.selected < 0) {
    row("", "Click a node, or focus the drawing and press an arrow key.");
    list.replaceChildren(...rows);
    return;
  }
  const { tree, selected: node, view, component } = scene;
  const { manifest } = current;
  row("Node", `${node} in ${view.path}${tree.parent[node] < 0 ? ", the root" : ""}`);
  row("Depth", String(tree.depth[node]));
  if (isLeaf(tree, node)) {
    const variable = tree.variable[node];
    const chain = variableChain(variable);
    for (const [space, value] of chain) row(space[0].toUpperCase() + space.slice(1), value);
    const tokens = [];
    if (view.component !== -1) tokens.push(`local ${variable}`, `reduced ${chain[1][1]}`);
    else tokens.push(`reduced ${variable}`);
    const reduced = view.component === -1 ? variable : Number(chain[1][1]);
    if (Number.isInteger(reduced)) tokens.push(`original ${originalToken(current.record, reduced)}`);
    row("Numbering", tokens.join(" → "), "chain");
    if (view.component === -1 && manifest !== null) {
      const free = (manifest.free_vars_reduced_dimacs ?? []).includes(variable);
      const index = (manifest.components ?? []).findIndex((entry) => entry.local_to_reduced_dimacs.includes(variable));
      if (free) row("Component", "none: a free variable, in no clause of reduced.cnf");
      else if (index >= 0) {
        const local = manifest.components[index].local_to_reduced_dimacs.indexOf(variable) + 1;
        const swatch = make("span");
        if (component !== null) {
          const chip = make("span", "swatch");
          chip.style.background = readColors().components[index % 8];
          swatch.append(chip);
        }
        swatch.append(document.createTextNode(`${String(index).padStart(3, "0")}, as local variable ${local}`));
        row("Component", swatch);
      }
    }
    const counts = occurrenceCounts();
    if (counts !== null && variable < counts.length) row("Clauses", `occurs in ${plural(counts[variable], "clause")} of ${view.cnf}`);
  } else {
    row("Leaves below", tree.leaves[node].toLocaleString("en-US"));
    const { variables, all } = leafVariables(tree, node, LISTED_LEAVES);
    row(view.component === -1 ? "Variables" : "Local variables", `${variables.join(" ")}${all ? "" : " …"}`);
    row("Shown", scene.expanded[node] === 1 ? "expanded" : "collapsed; press Enter or double-click to expand");
    if (component !== null) {
      if (component[node] >= 0) row("Component", String(component[node]).padStart(3, "0"));
      else {
        const seen = new Set();
        let free = 0;
        for (let i = tree.pre[node]; i <= tree.last[node]; i++) {
          const below = tree.order[i];
          if (!isLeaf(tree, below)) continue;
          if (component[below] >= 0) seen.add(component[below]);
          else if (component[below] === -2) free += 1;
        }
        row("Joins", `${plural(seen.size, "component")}${free > 0 ? ` and ${plural(free, "free variable")}` : ""}`);
      }
    }
  }
  if (view.builtBy) row("Built by", view.builtBy.join(", "));
  list.replaceChildren(...rows);
}

function describeTree() {
  const { tree, layout, view, component } = scene;
  const parts = [
    `Showing the vtree of the ${view.label}: ${plural(tree.count, "node")}, ` +
      `${plural(tree.leaves[tree.root], "leaf")}, depth ${tree.height}.`,
    layout.collapsed > 0
      ? `${plural(layout.drawn, "node")} drawn; ${plural(layout.collapsed, "subtree")} collapsed, ` +
        (layout.collapsed === 1 ? "drawn as a triangle with its leaf count." : "drawn as triangles with their leaf counts.")
      : "Every node is drawn.",
  ];
  if (component !== null) {
    parts.push("Colours mark the components of reduced.cnf; grey internal nodes join components, and dashed leaves are free variables.");
  }
  element("tree-description").textContent = parts.join(" ");
  treeView.setAttribute("aria-label", `Vtree drawing, ${view.label}`);
  if (element("outline-box").open) renderOutline();
}

// The drawn part of the tree as nested lists, for reading without the
// picture.
function renderOutline() {
  const holder = element("outline");
  if (scene === null) {
    holder.replaceChildren();
    return;
  }
  const { tree, expanded } = scene;
  const root = make("ul");
  const stack = [[tree.root, root]];
  let items = 0;
  while (stack.length > 0 && items < OUTLINE_ITEMS) {
    const [node, list] = stack.pop();
    const item = make("li");
    if (isLeaf(tree, node)) {
      item.textContent = `leaf ${node}: variable ${tree.variable[node]}`;
    } else {
      const leaves = plural(tree.leaves[node], "leaf");
      item.textContent = `node ${node}: ${leaves}${expanded[node] === 1 ? "" : ", collapsed"}`;
      if (expanded[node] === 1) {
        const children = make("ul");
        item.append(children);
        stack.push([tree.right[node], children], [tree.left[node], children]);
      }
    }
    list.append(item);
    items += 1;
  }
  const parts = [root];
  if (stack.length > 0) parts.push(make("p", "note", `The list stops after ${OUTLINE_ITEMS.toLocaleString("en-US")} items.`));
  holder.replaceChildren(...parts);
}
element("outline-box").addEventListener("toggle", () => {
  if (element("outline-box").open) renderOutline();
});

// -------------------------------------------------------- tree controls

element("zoom").addEventListener("click", (event) => {
  const button = event.target.closest("button");
  if (button === null || scene === null) return;
  const width = treeView.clientWidth;
  const height = treeView.clientHeight;
  const asked = button.dataset.zoom;
  if (asked === "in") zoomAt(1.25, width / 2, height / 2);
  else if (asked === "out") zoomAt(0.8, width / 2, height / 2);
  else if (asked === "fit") {
    fit();
    scheduleDraw();
  } else if (asked === "collapse") resetView();
  else if (asked === "expand") expandSelected();
});

treeView.addEventListener("wheel", (event) => {
  if (scene === null) return;
  event.preventDefault();
  const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? treeView.clientHeight : 1;
  const box = treeView.getBoundingClientRect();
  zoomAt(Math.exp(-event.deltaY * unit * 0.0015), event.clientX - box.left, event.clientY - box.top);
}, { passive: false });

let press = null;
treeView.addEventListener("pointerdown", (event) => {
  if (scene === null || event.button !== 0) return;
  treeView.setPointerCapture(event.pointerId);
  press = { id: event.pointerId, x: event.clientX, y: event.clientY, tx: scene.tx, ty: scene.ty, moved: false };
});
treeView.addEventListener("pointermove", (event) => {
  if (press === null || event.pointerId !== press.id) return;
  const dx = event.clientX - press.x;
  const dy = event.clientY - press.y;
  if (!press.moved && Math.hypot(dx, dy) < 4) return;
  press.moved = true;
  treeView.classList.add("dragging");
  scene.tx = press.tx + dx;
  scene.ty = press.ty + dy;
  scheduleDraw();
});
const release = (event) => {
  if (press === null || event.pointerId !== press.id) return;
  const clicked = !press.moved && event.type === "pointerup";
  press = null;
  treeView.classList.remove("dragging");
  if (clicked) {
    const node = nodeUnder(event);
    if (node >= 0) select(node, false);
  }
};
treeView.addEventListener("pointerup", release);
treeView.addEventListener("pointercancel", release);
treeView.addEventListener("dblclick", (event) => {
  if (scene === null) return;
  const node = nodeUnder(event);
  if (node >= 0) toggle(node);
});

function nodeUnder(event) {
  const box = treeView.getBoundingClientRect();
  const wx = (event.clientX - box.left - scene.tx) / scene.scale;
  const wy = (event.clientY - box.top - scene.ty) / scene.scale;
  return nodeAt(scene.tree, scene.layout, scene.x, wx, wy, 4 / scene.scale);
}

// The drawn neighbour of a node in its row, one step left or right.
function rowNeighbour(node, step) {
  const { layout, tree } = scene;
  const depth = tree.depth[node];
  for (let i = layout.rowStart[depth]; i < layout.rowStart[depth + 1]; i++) {
    if (layout.rows[i] !== node) continue;
    const j = i + step;
    return j >= layout.rowStart[depth] && j < layout.rowStart[depth + 1] ? layout.rows[j] : -1;
  }
  return -1;
}

treeView.addEventListener("keydown", (event) => {
  if (scene === null || event.altKey || event.ctrlKey || event.metaKey) return;
  const { tree, expanded } = scene;
  const node = scene.selected;
  const width = treeView.clientWidth;
  const height = treeView.clientHeight;
  const move = (target) => {
    if (target >= 0) select(target);
  };
  switch (event.key) {
    case "ArrowUp":
      move(node < 0 ? tree.root : tree.parent[node]);
      break;
    case "ArrowDown":
      if (node < 0) move(tree.root);
      else if (!isLeaf(tree, node)) {
        if (expanded[node] === 0) toggle(node);
        move(tree.left[node]);
      }
      break;
    case "ArrowLeft":
    case "ArrowRight": {
      if (node < 0) {
        move(tree.root);
        break;
      }
      const up = tree.parent[node];
      const sibling = up < 0 ? -1 : event.key === "ArrowLeft" ? tree.left[up] : tree.right[up];
      move(sibling >= 0 && sibling !== node ? sibling : rowNeighbour(node, event.key === "ArrowLeft" ? -1 : 1));
      break;
    }
    case "Home":
      move(tree.root);
      break;
    case "Enter":
    case " ":
      if (node >= 0) toggle(node);
      break;
    case "*":
      expandSelected();
      break;
    case "+":
    case "=":
      zoomAt(1.25, width / 2, height / 2);
      break;
    case "-":
    case "_":
      zoomAt(0.8, width / 2, height / 2);
      break;
    case "0":
      fit();
      scheduleDraw();
      break;
    default:
      return;
  }
  event.preventDefault();
});

new ResizeObserver(() => scheduleDraw()).observe(treeView);

// ---------------------------------------------------------------- start

const params = new URLSearchParams(location.search);
addressSettings = params;
const opening = EXAMPLES.findIndex(([key]) => key === params.get("example"));
if (cnfBox.value === "") loadExample(opening === -1 ? 0 : opening);
else describeInput();
