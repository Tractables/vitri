# vitri in the browser

A page that runs vitri in the tab: paste a DIMACS CNF, open or drop a file, or
pick an example, and it shows the reduced formula beside a drawing of the
vtree, with the count-lift record, the component split and each leaf's
variable in reduced, local and original numbering. Every file of the bundle
can be saved on its own or together as a zip, and the tree as SVG or PNG. The
address bar carries the example and the settings, so a result can be linked
to. Nothing is uploaded and nothing is fetched from another site.

The page is `index.html`, `styles.css`, `app.js` and `worker.js`, with two
example formulas, `example.cnf` and `mc2023_track1_008.reduced.cnf`, which are
links to the copies under `docs/`. It runs beside the Emscripten build of
vitri, `vitri.js` and `vitri.wasm`, inside a worker, so the page stays live
during a run and Cancel can stop one.

The browser build does not include Arjun, whose GMP and MPFR dependencies stay
dynamically linked. The page reads this from the build's capabilities and
switches the stage off, so a native run with `--no-arjun` is the one to compare
a page result with.

## Build locally

You need the
[Emscripten SDK](https://emscripten.org/docs/getting_started/downloads.html)
on PATH and the Rust target:

```sh
rustup target add wasm32-unknown-emscripten
cd bindings/wasm
CXX_wasm32_unknown_emscripten=em++ \
AR_wasm32_unknown_emscripten=emar \
CXXFLAGS_wasm32_unknown_emscripten=-fwasm-exceptions \
  cargo build --release
mkdir -p site
cp -L index.html styles.css app.js worker.js example.cnf mc2023_track1_008.reduced.cnf \
   target/wasm32-unknown-emscripten/release/vitri.{js,wasm} site/
python3 -m http.server -d site
```

`cp -L` copies the example formulas rather than the links to them. Opening
`index.html` from the filesystem does not work: the module and the examples
are fetched, so the files have to come from a server.

## Cache stamps

A workflow that publishes the page stamps the two references in `index.html`
with the commit (`app.js?v=<sha>`). The page passes that stamp to the worker,
the worker to `vitri.js` and `vitri.wasm`, and the page to the example files
it fetches, so a new page never runs with an older file a browser still holds.
