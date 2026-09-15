# Count valid configurations

This walkthrough prepares one small problem with Vitri, then counts its valid
configurations with **PySDD** or **RSDD**. Both read the same `.cnf` and
`.vtree` files. Pick either route, or run both to compare the answers.

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

## Install Vitri and prepare the input

Install the [native build prerequisites](building.md#toolchain), including Rust,
a C++20 compiler, CMake and the arithmetic libraries. Then, in an empty working
directory:

```sh
cargo install vitri --locked
curl -fL https://raw.githubusercontent.com/Tractables/vitri/v0.2.0/docs/example.cnf -o choices.cnf
vitri choices.cnf --mode mc --out-dir bundle/ --budget-ms 60000
```

The solver will use `bundle/reduced.cnf` and `bundle/vtree.vtree` together;
`bundle/preprocess.json` records how to recover the original count.

## Route 1: PySDD

[PySDD](https://github.com/ML-KULeuven/PySDD) provides a Python interface and
command-line compiler for sentential decision diagrams (SDDs). Its Python API
also supports working with the circuit after compilation.

Install it in a Python virtual environment:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install pysdd
pysdd -c bundle/reduced.cnf -v bundle/vtree.vtree -r 0 -R compiled.sdd -W compiled.vtree
```

`-v` reads Vitri's vtree and `-r 0` disables the compiler's vtree search for this
example. Find `sdd model count` in the output; its value should be **50**.
The `.sdd` and `.vtree` output files retain the compiled circuit and its tree.

## Route 2: RSDD

[RSDD](https://github.com/neuppl/rsdd) is an independent Rust implementation of
sentential decision diagrams. The companion [Rust example](https://github.com/Tractables/vitri/blob/main/examples/rsdd-count/src/main.rs)
reads Vitri's `.vtree` file and builds the corresponding RSDD tree before
compiling the CNF.

From the same working directory, with Rust and Vitri's build prerequisites
installed:

```sh
git clone https://github.com/Tractables/vitri.git
cargo run --release --locked --manifest-path vitri/examples/rsdd-count/Cargo.toml -- bundle/reduced.cnf bundle/vtree.vtree
```

The result should be `Reduced count: 50`. This example supports unweighted
CNFs with at most 52 variables so its floating-point counting stays exact;
use it as a small integration example. RSDD itself can compile larger inputs.

## Recover the original answer

The compiler counted the reduced formula. Apply Vitri's preprocessing record
to recover the count of the original, using the reported reduced count below:

```sh
python3 - <<'PYCOUNT'
import json
from fractions import Fraction

with open("bundle/preprocess.json") as stream:
    record = json.load(stream)
reduced_count = 50  # The count printed by your chosen compiler.
original_count = (reduced_count * 2 ** record["count_lift_pow2"]
                  * Fraction(record["weight_lift"]))
assert original_count.denominator == 1
print("Original count:", original_count.numerator)
PYCOUNT
```

The answer is **50**, matching our direct calculation. This example's count
lift is one; other formulas can have a nontrivial lift. The [bundle reference](bundle.md)
covers the record, including cases preprocessing solves without a compiler.

## Compile for later queries

If you need a circuit for later queries about the Boolean function, prepare
with `--mode compile` and then use the same compiler route on that bundle:

```sh
vitri choices.cnf --mode compile --out-dir query-bundle/ --budget-ms 60000
pysdd -c query-bundle/reduced.cnf -v query-bundle/vtree.vtree -r 0 -R queries.sdd -W queries.vtree
```

Read the [variable map](bundle.md#preprocessjson) before expressing queries in
the circuit's numbering; [preprocessing modes](preprocessing.md) specify what
each task preserves. PySDD's [Python API](https://pysdd.readthedocs.io/en/latest/usage/package.html)
provides operations on the saved circuit.

## Use your own problem

Replace `choices.cnf` with your DIMACS formula and choose the appropriate
[mode](../README.md#modes). The two commands above demonstrate ordinary,
unweighted model counting; weighted and projected tasks require corresponding
support and metadata handling in the downstream solver.

Start with Vitri's default portfolio and adjust `--budget-ms` to the preparation
time you can afford. For tuning or supplying a decomposition, see
[vtree construction](vtrees.md); for embedding, see the
[Rust worked example](https://docs.rs/vitri/latest/vitri/#a-worked-example).

For another compiler, check its vtree contract as well as its file format.
For example, [miniC2D](http://reasoning.cs.ucla.edu/minic2d/) requires a decision
vtree for the input formula; Vitri's default portfolio does not guarantee
that property.
