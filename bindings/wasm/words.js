// The words the page puts to vitri's tokens, and small formatters.

export const MODE_NAMES = {
  mc: "model counting",
  wmc: "weighted model counting",
  pmc: "projected model counting",
  pwmc: "projected weighted model counting",
  compile: "compilation, preserving the Boolean function",
};

export const STAGE_NAMES = { simplify: "Simplify", arjun: "Arjun", sbva: "SBVA" };

export const OUTCOMES = {
  ran: "ran",
  skipped: "skipped",
  gave_up: "gave up",
  discarded: "ran; its result was discarded",
};

export const ERROR_KINDS = {
  config: "a setting was refused",
  spec: "the vtree spec was refused",
  env: "the environment was refused",
  input: "the input was refused",
  io: "a file could not be read or written",
  mismatch: "the result did not match its input",
  construction: "the vtree construction failed",
  load: "vitri did not load",
  worker: "the worker failed",
  format: "the result could not be read",
};

const PLURALS = { leaf: "leaves" };
export const plural = (count, word) =>
  `${count.toLocaleString("en-US")} ${count === 1 ? word : PLURALS[word] ?? `${word}s`}`;

export function megabytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1048576).toFixed(1)} MB`;
}

export function duration(ms) {
  if (ms < 1) return "under 1 ms";
  if (ms < 10000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

// A signed literal of the original formula, from `reduced_to_original_dimacs`.
export function originalLiteral(record, reduced) {
  const map = record === null ? null : record.reduced_to_original_dimacs;
  if (!Array.isArray(map)) return "unknown: preprocess.json has no variable map";
  if (reduced < 1 || reduced > map.length) return "outside the variable map";
  const literal = map[reduced - 1];
  if (literal === null) return "none: introduced by preprocessing";
  return literal > 0 ? `variable ${literal}` : `the negation of variable ${-literal}`;
}

export function originalToken(record, reduced) {
  const map = record === null ? null : record.reduced_to_original_dimacs;
  if (!Array.isArray(map) || reduced < 1 || reduced > map.length) return "?";
  const literal = map[reduced - 1];
  return literal === null ? "none" : String(literal);
}
