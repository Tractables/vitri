// The functions vitri.wasm exports, called one way from the page's worker and
// from the node smoke test. Loads as a classic script (`importScripts`), which
// defines `vitriAbi`, and as a CommonJS module.
//
// Emscripten returns a pointer above 2 GB as a negative number; `>>> 0` reads
// it back as the unsigned address it is.
"use strict";

// The exports of an instantiated `vitri` module as two calls that take and
// return JSON values. Every string the module returns is allocated on its side
// and released here.
function vitriAbi(vitri) {
  function take(pointer) {
    pointer >>>= 0;
    const text = vitri.UTF8ToString(pointer);
    vitri._vitri_string_free(pointer);
    return JSON.parse(text);
  }

  // `bytes` copied into the module's heap while `body` runs with their
  // address. Passed as a "string" argument, ccall would copy them onto the
  // module's stack, which a large formula overflows.
  function lend(bytes, body) {
    const at = vitri._malloc(Math.max(1, bytes.length)) >>> 0;
    try {
      vitri.HEAPU8.set(bytes, at);
      return body(at);
    } finally {
      vitri._free(at);
    }
  }

  return {
    // What this build accepts.
    capabilities: () => take(vitri._vitri_capabilities_json()),

    // `request` run over the DIMACS text `dimacs`: {ok: true, summary, files}
    // or {ok: false, error: {kind, message}}.
    prepare(dimacs, request) {
      const encoder = new TextEncoder();
      const formula = encoder.encode(dimacs);
      const requestText = encoder.encode(JSON.stringify(request) + "\0");
      return lend(formula, (formulaAt) =>
        lend(requestText, (requestAt) => take(vitri._vitri_prepare_json(formulaAt, formula.length, requestAt))),
      );
    },
  };
}

if (typeof module !== "undefined") module.exports = { vitriAbi };
