# vitri for Python

Python bindings for [vitri](https://github.com/Tractables/vitri), built with
[PyO3](https://pyo3.rs) and [maturin](https://www.maturin.rs). vitri
preprocesses a DIMACS CNF, records how to lift a model count of the reduced
formula back to the original, and builds a vtree for the reduced formula.

## Use

```python
import vitri

with open("formula.cnf", "rb") as stream:
    result = vitri.prepare(stream.read(), mode="mc", budget_ms=60_000)

result.status            # "built", "fully_resolved" or "refuted"
result.summary["lift"]   # how a count of the reduced formula becomes the input's
result.reduced_cnf       # the formula to compile, as DIMACS text
result.vtree             # the vtree over it in SDD format, or None unless built
result.write("bundle/")  # the files `vitri formula.cnf --out-dir bundle/` writes
```

`help(vitri.prepare)` covers the settings, the errors, calls from threads and
budgets. The [bundle reference](https://github.com/Tractables/vitri/blob/main/docs/bundle.md)
describes the files. [`examples/`](https://github.com/Tractables/vitri/tree/main/bindings/python/examples)
counts models with PySDD over the emitted vtree, and runs a call under a hard
time limit.

## Platforms

Wheels are built for CPython 3.10 and later on x86-64 Linux with glibc 2.34 or
newer. They carry the GMP libraries the extension loads, as shared libraries,
with GMP's notices. Anywhere else, build from source.

## Building from source

The extension compiles the vitri sources in this repository, so it needs the
toolchain and system packages listed in
[`building.md`](https://github.com/Tractables/vitri/blob/main/docs/building.md),
and maturin. PEP 639 resolves `license-files` against this directory and
forbids `..`, so the notices are copied in first:

```sh
sh bindings/python/collect-notices.sh
pip install maturin
maturin build --release --manifest-path bindings/python/Cargo.toml
```

`maturin develop` installs the extension into the active virtual environment
for `pytest bindings/python/tests`. Give pytest `--vitri-cli PATH` to also
compare the bundles with those of the `vitri` executable.
