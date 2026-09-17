# vitri in the browser

A page that runs vitri in the tab: paste a DIMACS CNF, open or drop a file, or
pick an example, and it shows the reduced formula beside a drawing of the
vtree, with the count-lift record, the component split and each leaf's
variable in reduced, local and original numbering. Every file of the bundle
can be saved on its own or together as a zip, and the tree as SVG or PNG. The
address bar carries the example and the settings, so a result can be linked
to. Nothing is uploaded and nothing is fetched from another site.

The page is `index.html`, `styles.css` and the scripts in this directory, with
two example formulas, `example.cnf` and `mc2023_track1_008.reduced.cnf`, which
are links to the copies under `docs/`. `app.js` is the page itself, an ES
module over the parts that need no page: `vtree.js` reads a `.vtree` file and
a DIMACS header, `layout.js` places the drawn nodes, `drawing.js` draws them
on a canvas or as SVG, `text-view.js` scrolls a file of any size, `runner.js`
drives the worker, `examples.js`, `words.js` and `zip.js` are what their names
say. `worker.js` runs the Emscripten build of vitri through `abi.js`, so the
page stays live during a run and Cancel can stop one; both are classic scripts,
since the worker's `importScripts` needs them so. That build is `vitri.js` and
`vitri.wasm`, which load GMP's side modules `libgmp.so` and `libgmpxx.so`; the
site also serves the GMP source tarball they were built from and, under
`notices/`, the licence texts the page links to.

`smoke.mjs` runs the built module on CNFs, reads the vtrees it writes with
`vtree.js`, and compares the files with the native tool's; the module workflow
runs it.

A result can differ from a native run's on the same settings. The portfolio
runs against the clock, and the browser is slower. Arjun can also reduce a
formula to a different one with the same count.

## Build locally

Build the GMP prefix and the module as
[`building.md`](../../docs/building.md#building-for-emscripten) describes, then
put the page beside them and serve the directory:

```sh
cd bindings/wasm
./site.sh site "$VITRI_EMSCRIPTEN_PREFIX"
python3 -m http.server -d site
```

`site.sh` gathers what the page is served with; the module workflow builds its
artifact the same way. Opening `index.html` from the filesystem does not work:
the module and the examples are fetched, so the files have to come from a
server.

## Cache stamps

A workflow that publishes the page stamps the two references in `index.html`
and the imports between the scripts with the commit (`app.js?v=<sha>`). A
script reads its own stamp from `import.meta.url`; the page passes it to the
worker, the worker to `vitri.js` and every file the module loads, and the page
to the example files it fetches, so a new page never runs with an older file a
browser still holds.
