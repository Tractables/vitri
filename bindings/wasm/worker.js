"use strict";

// vitri in a worker of its own, so the page stays live while a formula is
// prepared and a run can be stopped: the page ends this worker and starts
// another. In, one message per run: {dimacs, request}. Out: {ready: true,
// capabilities} once the module is up, then per run either
// {result: {summary, files}, elapsed} or {error: {kind, message}}.
//
// The page passes its version stamp in this worker's address (see app.js);
// the module and its wasm file take the same stamp.
importScripts("vitri.js" + location.search);

let vitri = null;

// Every string the module returns is allocated on its side and released here.
function take(pointer) {
  const text = vitri.UTF8ToString(pointer);
  vitri.ccall("vitri_string_free", null, ["number"], [pointer]);
  return text;
}

createVitri({ locateFile: (file) => file + location.search }).then(
  (module) => {
    vitri = module;
    const capabilities = JSON.parse(take(vitri.ccall("vitri_capabilities_json", "number", [], [])));
    postMessage({ ready: true, capabilities });
  },
  (failure) => {
    postMessage({ error: { kind: "load", message: String(failure) } });
  },
);

onmessage = (event) => {
  const { dimacs, request } = event.data;
  const started = performance.now();
  // The formula goes in as a buffer and a length. Passed as a "string"
  // argument, ccall would copy it onto the module's stack, which a large
  // formula overflows.
  const bytes = new TextEncoder().encode(dimacs);
  let buffer = 0;
  try {
    buffer = vitri._malloc(Math.max(1, bytes.length));
    vitri.HEAPU8.set(bytes, buffer);
    const answer = JSON.parse(
      take(
        vitri.ccall(
          "vitri_prepare_json",
          "number",
          ["number", "number", "string"],
          [buffer, bytes.length, JSON.stringify(request)],
        ),
      ),
    );
    if (answer.ok === true) {
      postMessage({
        result: { summary: answer.summary, files: answer.files },
        elapsed: performance.now() - started,
      });
    } else {
      postMessage({ error: answer.error });
    }
  } catch (failure) {
    postMessage({ error: { kind: "worker", message: String(failure) } });
  } finally {
    if (buffer !== 0) vitri._free(buffer);
  }
};
