// Runs the built module on CNFs and checks what comes back: a formula with the
// Arjun stage on and off, a projected one, one under compile mode and one
// preprocessing refutes. Then checks that a malformed input and an unknown
// request key are errors. Given the native command-line tool built from the
// same checkout, it also checks that the module writes the tool's files, byte
// for byte.
//
//   node smoke.cjs <vitri.js> <docs/example.cnf> <docs/showcase/mc2023_track1_008.reduced.cnf> [<vitri>]
//
// libgmp.so and libgmpxx.so have to be beside vitri.js; abi.js beside this file.

"use strict";

const childProcess = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const [modulePath, examplePath, showcasePath, nativePath] = process.argv.slice(2);
if (!modulePath || !examplePath || !showcasePath) {
  console.error("usage: node smoke.cjs <vitri.js> <docs/example.cnf> <docs/showcase/mc2023_track1_008.reduced.cnf> [<vitri>]");
  process.exit(2);
}
const createVitri = require(path.resolve(modulePath));
// The module is called the way the page's worker calls it.
const { vitriAbi } = require(path.join(__dirname, "abi.js"));

let failures = 0;
function check(condition, what) {
  if (!condition) {
    console.error(`FAIL: ${what}`);
    failures += 1;
  }
}

// The `p cnf <variables> <clauses>` header of a DIMACS text.
function header(dimacs) {
  const line = dimacs.split("\n").find((l) => l.startsWith("p cnf "));
  const [variables, clauses] = line.split(/\s+/).slice(2, 4).map(Number);
  return { variables, clauses };
}

// The variables on the leaves of a `.vtree` text, sorted, after checking that
// the node count matches the header and that the tree is binary.
function vtreeLeaves(text, name) {
  const lines = text.split("\n").filter((l) => l !== "" && !l.startsWith("c"));
  const declared = Number(lines[0].split(" ")[1]);
  const nodes = lines.slice(1);
  check(nodes.length === declared, `${name}: header declares ${declared} nodes, has ${nodes.length}`);
  const leaves = nodes.filter((l) => l.startsWith("L ")).map((l) => Number(l.split(" ")[2]));
  const internal = nodes.filter((l) => l.startsWith("I ")).length;
  check(internal === leaves.length - 1, `${name}: ${leaves.length} leaves need ${leaves.length - 1} internal nodes, has ${internal}`);
  return leaves.sort((a, b) => a - b);
}

// A built run: the vtree's leaves are exactly the reduced formula's variables,
// and the record carries the count lift.
function checkBuilt(name, result) {
  check(result.ok, `${name}: ${JSON.stringify(result.error)}`);
  if (!result.ok) return;
  check(result.summary.status === "built", `${name}: status ${result.summary.status}, expected built`);
  const files = result.files;
  for (const file of ["reduced.cnf", "preprocess.json", "vtree.vtree", "components.json"]) {
    check(typeof files[file] === "string", `${name}: no ${file}`);
  }
  if (typeof files["vtree.vtree"] !== "string") return;
  const { variables } = header(files["reduced.cnf"]);
  const leaves = vtreeLeaves(files["vtree.vtree"], name);
  const expected = Array.from({ length: variables }, (_, i) => i + 1);
  check(JSON.stringify(leaves) === JSON.stringify(expected), `${name}: leaves ${leaves} are not the variables 1..${variables}`);
  const record = JSON.parse(files["preprocess.json"]);
  check(Number.isInteger(record.count_lift_pow2), `${name}: record has no count_lift_pow2`);
  check(typeof record.weight_lift === "string", `${name}: record has no weight_lift`);
  const summary = result.summary;
  check(summary.lift.count_lift_pow2 === record.count_lift_pow2, `${name}: summary lift differs from preprocess.json`);
  check(summary.vtree.leaves === variables, `${name}: summary counts ${summary.vtree.leaves} leaves, reduced.cnf has ${variables} variables`);
  check(JSON.stringify([...summary.files].sort()) === JSON.stringify(Object.keys(files).sort()), `${name}: summary.files is not the file list`);
}

// The files the native tool writes for a CNF, by their paths in its output
// directory.
function nativeFiles(cnfPath, args) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "vitri-smoke-"));
  try {
    childProcess.execFileSync(nativePath, [...args, "--out-dir", dir, cnfPath], { stdio: "pipe" });
    const files = {};
    for (const name of fs.readdirSync(dir, { recursive: true })) {
      const file = path.join(dir, name);
      if (fs.statSync(file).isFile()) files[name.split(path.sep).join("/")] = fs.readFileSync(file, "utf8");
    }
    return files;
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

// The default vtree construction is a portfolio that runs against the clock,
// so the comparison pins min-fill. Edge binarization, because goatd's
// hypergraph binarization draws its random choices differently on a 32-bit
// target.
const PARITY_VTREE = "minfill-primal:binarize=edge";

function checkParity(vitri, name, cnfPath, arjun) {
  const label = `${name}, Arjun ${arjun ? "on" : "off"}, against the native tool`;
  const module = vitri.prepare(fs.readFileSync(cnfPath, "utf8"), arjun ? { vtree: PARITY_VTREE } : { vtree: PARITY_VTREE, arjun: false });
  check(module.ok, `${label}: ${JSON.stringify(module.error)}`);
  if (!module.ok) return;
  const native = nativeFiles(cnfPath, arjun ? ["--vtree", PARITY_VTREE] : ["--vtree", PARITY_VTREE, "--no-arjun"]);
  const names = Object.keys(native).sort();
  check(JSON.stringify(names) === JSON.stringify(Object.keys(module.files).sort()), `${label}: the module writes ${Object.keys(module.files).sort()}, the tool ${names}`);
  const differing = names.filter((file) => module.files[file] !== native[file]);
  check(differing.length === 0, `${label}: ${differing} differ`);
  console.log(`${label}: ${names.length} files compared`);
}

const PROJECTED = [
  "c t pmc",
  "p cnf 6 5",
  "c p show 1 2 0",
  "1 3 0",
  "-3 4 0",
  "2 5 6 0",
  "-5 -6 0",
  "4 5 0",
  "",
].join("\n");

const UNSAT = "p cnf 2 4\n1 2 0\n-1 2 0\n1 -2 0\n-1 -2 0\n";

createVitri().then((module) => {
  const vitri = vitriAbi(module);
  const capabilities = vitri.capabilities();
  console.log("capabilities", JSON.stringify(capabilities));
  check(capabilities.stages.simplify === true, "the build carries the simplify chain");
  check(capabilities.stages.arjun === true, "the build carries the Arjun stage");
  check(capabilities.request_keys.includes("arjun"), "the arjun request key is listed");

  const example = fs.readFileSync(examplePath, "utf8");
  for (const arjun of [true, false]) {
    const name = `example.cnf, Arjun ${arjun ? "on" : "off"}`;
    const started = Date.now();
    const run = vitri.prepare(example, arjun ? {} : { arjun: false });
    console.log(`${name}: ${Date.now() - started} ms`, JSON.stringify(run.summary));
    checkBuilt(name, run);
    if (!run.ok) continue;
    check(run.summary.request.arjun === arjun, `${name}: the effective request has arjun=${run.summary.request.arjun}`);
    const stage = run.summary.stages.arjun;
    check(arjun ? stage === "ran" : stage !== "ran", `${name}: the Arjun stage reports ${stage}`);
  }

  let started = Date.now();
  const projected = vitri.prepare(PROJECTED, {});
  console.log(`projected ${Date.now() - started} ms`, JSON.stringify(projected.summary));
  check(projected.ok && projected.summary.mode === "pmc", "the projected CNF runs under pmc");
  if (projected.ok && projected.summary.status === "built") {
    checkBuilt("projected", projected);
    check(/^c p show /m.test(projected.files["reduced.cnf"]), "projected: reduced.cnf keeps a show line");
  }

  // Compile mode's chain has no Arjun stage.
  started = Date.now();
  const compiled = vitri.prepare(example, { mode: "compile" });
  console.log(`compile ${Date.now() - started} ms`, JSON.stringify(compiled.summary || compiled.error));
  checkBuilt("example.cnf under compile", compiled);

  const refuted = vitri.prepare(UNSAT, {});
  console.log("unsat", JSON.stringify(refuted.summary));
  check(refuted.ok && refuted.summary.status === "refuted", "the UNSAT CNF is refuted");
  check(refuted.ok && refuted.files["vtree.vtree"] === undefined, "a refuted run has no vtree file");
  check(refuted.ok && typeof refuted.files["reduced.cnf"] === "string", "a refuted run still has reduced.cnf");

  const malformed = vitri.prepare("p cnf 2 1\n1 x 0\n", {});
  console.log("malformed", JSON.stringify(malformed.error));
  check(!malformed.ok && malformed.error.kind === "input", "a malformed CNF is an input error");

  const unknown = vitri.prepare(UNSAT, { colour: "blue" });
  console.log("unknown key", JSON.stringify(unknown.error));
  check(!unknown.ok && unknown.error.kind === "config", "an unknown request key is a config error");

  if (nativePath) {
    checkParity(vitri, "example.cnf", examplePath, true);
    checkParity(vitri, "example.cnf", examplePath, false);
    // With Arjun on, the module's Arjun stack reduces this formula to the same
    // variables and count as the native build's but numbers them in another
    // order, so it is compared with Arjun off.
    checkParity(vitri, path.basename(showcasePath), showcasePath, false);
  }

  if (failures > 0) {
    console.error(`${failures} check(s) failed`);
    process.exit(1);
  }
  console.log("smoke: all checks passed");
});
