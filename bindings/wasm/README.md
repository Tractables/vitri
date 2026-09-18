# vitri in the browser

**<https://tractables.github.io/vitri/>**

A page that runs vitri in the tab. Drop in a DIMACS CNF — or paste one, open a
file, or pick an example — and it shows what came out:

- the reduced formula beside a drawing of the vtree built for it,
- the count-lift record, so you can get back to the original formula's count,
- the component split, with each leaf's variable in reduced, local and original
  numbering,
- the scores the portfolio ranked that vtree on: peak context width, the worst
  node's clause load, how evenly clauses spread, and the combined cost.

Every file of the bundle saves on its own or together as a zip, and the tree as
SVG or PNG. The address bar carries the example and the settings, so a result
can be linked to. Nothing is uploaded and nothing is fetched from another site:
the formula never leaves the browser.

A result can differ from a native run on the same settings. The portfolio runs
against the clock and the browser is slower, and Arjun can reduce a formula to
a different one with the same count.

## Build and serve it locally

Build the GMP prefix and the module as
[`building.md`](../../docs/building.md#building-for-emscripten) describes, then
put the page beside them and serve the directory:

```sh
cd bindings/wasm
./site.sh site "$VITRI_EMSCRIPTEN_PREFIX"
python3 -m http.server -d site
```

`site.sh` gathers what the page is served with; the module workflow builds its
artifact the same way. Opening `index.html` from the filesystem does not work —
the module and the examples are fetched, so they have to come from a server.

`smoke.mjs` runs the built module on CNFs, reads the vtrees it writes, and
compares the files with the native tool's. `page-check.mjs` needs no module: it
checks `index.html` against `styles.css`. The module workflow runs both.

## Cache stamps

A workflow that publishes the page stamps the two references in `index.html`
and the imports between the scripts with the commit (`app.js?v=<sha>`). A
script reads its own stamp from `import.meta.url`; the page passes it to the
worker, the worker to `vitri.js` and every file the module loads, and the page
to the example files it fetches, so a new page never runs with an older file a
browser still holds.
