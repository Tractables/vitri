# Count valid configurations

This walkthrough prepares one small problem with vitri, then counts its valid
configurations with a knowledge compiler: **tididi**, which builds tree
decision diagrams (TDDs), or **PySDD** or **RSDD**, which build sentential
decision diagrams (SDDs). Each compiler reads the formula vitri writes together
with its **vtree**, a binary tree over the formula's variables that fixes the
shape of the circuit the compiler builds. Follow any one route, or run them all
to compare the answers.

## The problem

Choose one of four options, A–D, for each of three time slots. You cannot
repeat A in adjacent slots, or repeat D in adjacent slots. How many schedules
are allowed?

There are 64 choices before those restrictions. If the middle option is B or
C, all 16 combinations of the outer slots work. If it is A or D, each outer
slot has three choices. The answer is therefore **2 × 16 + 2 × 9 = 50**.

The [example CNF](example.cnf) encodes those rules with twelve Boolean variables:

| Variables | Meaning |
| --- | --- |
| 1–4 | Choose A, B, C, or D in the first slot |
| 5–8 | Choose A, B, C, or D in the second slot |
| 9–12 | Choose A, B, C, or D in the third slot |

In DIMACS, `1 2 3 4 0` requires at least one first-slot option, and `-1 -2 0`
forbids choosing both A and B there. Similar pairs enforce exactly one option
per slot; the final four clauses impose the adjacent-slot restrictions.

## Install

In an empty working directory:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install pyvitri tididi tididi-cnf pysdd
```

`pyvitri` is vitri's Python package and imports as `vitri`. PyPI has it
prebuilt for x86-64 Linux; elsewhere pip compiles it, which needs the
[build prerequisites](building.md#toolchain). `tididi`, `tididi-cnf` and
`pysdd` are the Python compilers below; the RSDD route needs Rust instead.

## Prepare the input

```sh
curl -fL https://raw.githubusercontent.com/Tractables/vitri/main/docs/example.cnf -o choices.cnf
python - <<'PREPARE'
import vitri

with open("choices.cnf", "rb") as stream:
    result = vitri.prepare(stream.read(), mode="mc", budget_ms=60_000)
print(result.summary["input"], "->", result.summary["reduced"])
print("lift:", result.summary["lift"]["factor"])
result.write("bundle/")
PREPARE
```

Output:

```text
{'variables': 12, 'clauses': 25} -> {'variables': 9, 'clauses': 13}
lift: 2^0
```

`mode="mc"` asks preprocessing to keep the model count, and `budget_ms` is the
time vitri may spend on the whole run. Preprocessing left nine variables. A
compiler reads `bundle/reduced.cnf` with `bundle/vtree.vtree`, and
`bundle/preprocess.json` records how its count becomes a count of
`choices.cnf`.

The `vitri` command writes the same files:
`vitri choices.cnf --mode mc --out-dir bundle/ --budget-ms 60000`. Install it
with `cargo install vitri --locked` after the build prerequisites, or take the
x86-64 Linux archive from the
[latest release](https://github.com/Tractables/vitri/releases/latest).

## Count with tididi

[tididi](https://github.com/Tractables/tididi) builds tree decision diagrams.
This reads vitri's vtree, builds a circuit for each clause of `reduced.cnf` and
conjoins them. Lines that start with `c` or `p` are comments and the header;
every other line is one clause, ending in `0`:

```sh
python - <<'TDD'
import tididi

with open("bundle/vtree.vtree") as stream:
    vtree = tididi.Vtree.from_text(stream.read())
formula = tididi.one(vtree)
with open("bundle/reduced.cnf") as stream:
    for line in stream:
        if line.startswith(("c", "p")) or not line.strip():
            continue
        literals = [int(token) for token in line.split()[:-1]]
        formula &= tididi.clause(vtree, literals)
print("Reduced count:", formula.model_count())
TDD
```

Output:

```text
Reduced count: 50
```

To compile without writing that loop, use
[tididi-cnf](https://github.com/Tractables/tididi-cnf). It takes `choices.cnf`
itself and runs vitri inside: it preprocesses the formula and builds its own
vtree instead of reading the bundle, and prints the original count:

```sh
python - <<'TDDCNF'
import tididi_cnf

print("Count:", tididi_cnf.count_file("choices.cnf").count)
TDDCNF
```

Output:

```text
Count: 50
```

## Count with PySDD

[PySDD](https://github.com/ML-KULeuven/PySDD) builds sentential decision
diagrams. Its `pysdd` command reads both files:

```sh
pysdd -c bundle/reduced.cnf -v bundle/vtree.vtree -r 0
```

`-r 0` turns off PySDD's own vtree search, so it compiles on vitri's vtree as
given. Among its output:

```text
 sdd model count         : 50    0.000 sec
```

## Count with RSDD

[RSDD](https://github.com/neuppl/rsdd) is a Rust library for sentential
decision diagrams. The companion
[Rust example](https://github.com/Tractables/vitri/blob/main/examples/rsdd-count/src/main.rs)
reads vitri's `.vtree` file, builds the same tree in RSDD and compiles the CNF.
It needs Rust and vitri's [build prerequisites](building.md#toolchain); the
first run compiles vitri's C++ stack, which takes a few minutes. From the same
working directory:

```sh
git clone --depth 1 https://github.com/Tractables/vitri.git
cargo run --release --locked --manifest-path vitri/examples/rsdd-count/Cargo.toml -- bundle/reduced.cnf bundle/vtree.vtree
```

Output:

```text
Reduced count: 50
```

The example counts in floating point, so it accepts small unweighted CNFs
only; RSDD itself compiles larger ones.

## Recover the original answer

The compilers counted `reduced.cnf`, not `choices.cnf`. `preprocess.json`
records the factor between the two counts, the lift:
count(`choices.cnf`) = count(`reduced.cnf`) × 2^`count_lift_pow2` ×
`weight_lift`. Here the factor is 1 (`2^0` above), so the answer stays 50. For
any formula, apply the record:

```sh
python - <<'LIFT'
import json
from fractions import Fraction

with open("bundle/preprocess.json") as stream:
    record = json.load(stream)
reduced_count = 50  # The count your compiler printed.
original_count = (reduced_count * 2 ** record["count_lift_pow2"]
                  * Fraction(record["weight_lift"]))
print("Original count:", original_count)
LIFT
```

Output:

```text
Original count: 50
```

The [bundle reference](bundle.md) covers the record, including formulas that
preprocessing settles without a compiler.

## Compile for later queries

Under `mode="mc"`, `reduced.cnf` keeps only the count: preprocessing renumbers
variables and removes some, so a circuit compiled from it cannot answer
questions about `choices.cnf`'s variables. `mode="compile"` with
`equivalent_cnf=True`
([`RunConfig::equivalent_cnf`](https://docs.rs/vitri/latest/vitri/config/struct.RunConfig.html#structfield.equivalent_cnf))
writes a formula equivalent to the input, in its variable numbers:

```sh
python - <<'COMPILE'
import vitri

with open("choices.cnf", "rb") as stream:
    result = vitri.prepare(stream.read(), mode="compile", equivalent_cnf=True,
                           budget_ms=60_000)
result.write("query-bundle/")
COMPILE
pysdd -c query-bundle/reduced.cnf -v query-bundle/vtree.vtree -r 0 -R queries.sdd -W queries.vtree
```

PySDD prints the same count, 50, and saves the circuit and its vtree in
`queries.sdd` and `queries.vtree`; its
[Python API](https://pysdd.readthedocs.io/en/latest/usage/package.html) loads
them for queries. The tididi snippet above works on `query-bundle/` too, and
`formula.save(path)` keeps its circuit. Without `equivalent_cnf`,
[`Mode::Compile`](https://docs.rs/vitri/latest/vitri/cnf/enum.Mode.html#variant.Compile)
writes a smaller formula that needs `preprocess.json` to restore the input's
function.

## Use your own problem

Replace `choices.cnf` with your DIMACS formula and choose the
[mode](preprocessing.md#choosing-a-mode). A larger `budget_ms` can give a
better vtree. For tuning or supplying a decomposition, see
[vtree construction](vtrees.md); for embedding, see the
[Rust worked example](https://docs.rs/vitri/latest/vitri/#a-worked-example).

The routes above count every model of `reduced.cnf`. Weighted and projected
counting need a compiler that reads the `c p weight` and `c p show` lines vitri
writes into `reduced.cnf`: the `pysdd` command and the tididi snippet above
read neither and count without them, and the RSDD example refuses such files.

Some compilers accept only certain vtrees. [miniC2D](http://reasoning.cs.ucla.edu/minic2d/),
for example, needs a decision vtree and can return a wrong count on any other;
vitri does not guarantee a decision vtree.
