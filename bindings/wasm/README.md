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
links to the copies under `docs/`. It runs the Emscripten build of vitri inside
a worker, so the page stays live during a run and Cancel can stop one. That
build is `vitri.js` and `vitri.wasm`, which load GMP's side modules
`libgmp.so` and `libgmpxx.so`; the site also serves the GMP source tarball they
were built from.

A result can differ from a native run's on the same settings. The portfolio
runs against the clock, and the browser is slower. Arjun can also reduce a
formula to a different one with the same count.

## Build locally

Build the GMP prefix and the module as
[`building.md`](../../docs/building.md#building-for-emscripten) describes, then
put the page beside them and serve the directory:

```sh
cd bindings/wasm
mkdir -p site
cp -L index.html styles.css app.js worker.js example.cnf mc2023_track1_008.reduced.cnf \
   target/wasm32-unknown-emscripten/release/vitri.{js,wasm} \
   "$VITRI_EMSCRIPTEN_PREFIX"/lib/libgmp.so "$VITRI_EMSCRIPTEN_PREFIX"/lib/libgmpxx.so site/
python3 -m http.server -d site
```

`cp -L` copies the example formulas rather than the links to them. Opening
`index.html` from the filesystem does not work: the module and the examples
are fetched, so the files have to come from a server.

## Cache stamps

A workflow that publishes the page stamps the two references in `index.html`
with the commit (`app.js?v=<sha>`). The page passes that stamp to the worker,
the worker to `vitri.js` and every file the module loads, and the page to the example files
it fetches, so a new page never runs with an older file a browser still holds.
