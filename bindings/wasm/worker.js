"use strict";

// vitri in a worker of its own, so the page stays live while a formula is
// prepared and a run can be stopped: the page ends this worker and starts
// another. In, one message per run: {dimacs, request}. Out: {ready: true,
// capabilities} once the module is up, then per run either
// {result: {summary, files}, elapsed} or {error: {kind, message}}.
//
// The page passes its version stamp in this worker's address (see app.js);
// the module and its wasm file take the same stamp.
//
// A throw out of prepare is left to reach onerror, where the page terminates
// this worker and starts another: an Emscripten abort poisons the module, so
// every later run in the same worker would throw too.
importScripts("abi.js" + location.search, "vitri.js" + location.search);

let vitri = null;

createVitri({ locateFile: (file) => file + location.search }).then(
  (module) => {
    vitri = vitriAbi(module);
    postMessage({ ready: true, capabilities: vitri.capabilities() });
  },
  (failure) => {
    postMessage({ error: { kind: "load", message: String(failure) } });
  },
);

onmessage = (event) => {
  const { dimacs, request } = event.data;
  const started = performance.now();
  const answer = vitri.prepare(dimacs, request);
  if (answer.ok === true) {
    postMessage({
      result: { summary: answer.summary, files: answer.files },
      elapsed: performance.now() - started,
    });
  } else {
    postMessage({ error: answer.error });
  }
};
