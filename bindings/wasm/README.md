<p align="center">
  <img src="https://raw.githubusercontent.com/Tractables/vitri/assets/logo/vitri-logo.svg"
       alt="vitri" width="300">
</p>

<p align="center">
  <a href="https://tractables.github.io/vitri/"><img
     src="https://img.shields.io/badge/run-in%20the%20browser-blue" alt="Run in the browser"></a>
  <a href="https://github.com/Tractables/vitri/actions/workflows/wasm.yml"><img
     src="https://github.com/Tractables/vitri/actions/workflows/wasm.yml/badge.svg" alt="Web build"></a>
</p>

# vitri in the browser

**<https://tractables.github.io/vitri/>**

[vitri](../../README.md) simplifies a CNF formula and builds the vtree a
circuit compiler follows for it. This page runs it in the tab. Drop in a DIMACS
CNF, paste one, open a file or pick an example, and the page shows:

- the reduced formula beside a drawing of the vtree built for it,
- the count-lift record, which turns a count of the reduced formula into the
  original formula's count,
- the component split, with each leaf's variable in reduced, local and original
  numbering,
- the scores vtrees are ranked on: peak context width, the worst node's clause
  load, how evenly clauses spread, and the combined cost.

The four top-level files of the bundle save on their own, the whole bundle as a
zip, and the tree as SVG or PNG. The address bar carries the example and the
settings, so a result can be linked to. Nothing is uploaded and nothing is
fetched from another site: the formula never leaves the browser.

A result can differ from a native run on the same settings. The portfolio stops
on the clock and the browser is slower, and the browser's Arjun can number the
reduced variables differently.

## Build and serve it locally

These are the steps the Web build workflow (`wasm.yml`) runs. They need the
Emscripten SDK on `PATH`, the `wasm32-unknown-emscripten` Rust target and the
tools [`building.md`](../../docs/building.md#building-for-emscripten) lists.
From `bindings/wasm`:

```sh
./emscripten-prefix.sh "$PWD/work" "$PWD/prefix"
export VITRI_EMSCRIPTEN_PREFIX=$PWD/prefix
cargo build --release --locked
./site.sh site "$VITRI_EMSCRIPTEN_PREFIX"
cp "$(../../.github/scripts/gmp.sh fetch "$PWD/work")" site/
python3 -m http.server -d site
```

The last copy puts the GMP source the page links beside it. Opening
`index.html` from the filesystem does not work: the page fetches the module and
the examples, so they have to come from a server.

## Develop

`smoke.mjs` runs the built module on CNFs, reads the vtrees it writes, and
compares the files with the native tool's. `page-check.mjs` needs no module: it
checks `index.html` against `styles.css`. The Web build workflow runs both.

A workflow that publishes the page stamps every file the page loads with the
commit (`app.js?v=<sha>`), so a new page never runs with an older file a
browser still holds.
