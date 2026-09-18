# Preprocessing

What each mode does to the formula, and what a consumer does with the record to
get a correct answer over the original. [`bundle.md`](bundle.md) is the
field-by-field reference for the files described here.

## The lift identity

```text
count(original) == count(reduced) * 2^count_lift_pow2 * weight_lift
```

`count` is the mode's own count — plain, weighted, projected or
projected-weighted. Count `reduced.cnf` under `reduced_weights`, and under
`show_vars_reduced_dimacs` if the mode is projected; multiply by both factors,
in exact rational arithmetic.

| mode | `count_lift_pow2` | `weight_lift` |
| --- | --- | --- |
| `mc`, `pmc` | the whole lift | always `"1/1"` |
| `wmc`, `pwmc` | always `0` | the whole lift |
| `compile` | the unused variables, nothing else | always `"1/1"` |

## The steps

The mode picks one of three chains, which differ in which steps run and in
what order.

### `mc` and `wmc`

In order. Steps 1–7 are one unit — `--no-simplify` turns off all seven, and an
embedded caller configures them through `RunConfig::simplify`.

1. **Equivalence detection** — strongly connected components over the binary
   clauses. Rewrites onto a class representative but keeps every variable, so
   this step alone changes no ids.
2. **Backbone and equivalence probing** — one SAT session that finds forced
   literals, propagates them, re-runs step 1 over the clauses that propagation
   created, then probes for whatever equivalences remain. Time-budgeted.
3. **Clause simplification** in CaDiCaL. Rewrites clauses; removes no variable.
4. **Backbone and dead-variable stripping** — drops the forced variables and any
   variable no clause mentions. **First renumbering.**
5. **Equivalence reduction** — drops the partners step 1 found, keeping one
   representative per class. **Renumbers.**
6. **Gate detection** — finds AND/OR/XOR/ITE outputs. Removes nothing itself; it
   tells step 7 which variables are already known to be defined.
7. **Definability elimination (DVE)** — eliminates defined, free and
   newly-equivalent variables by resolution, under a round and time budget.
   **Renumbers.**
8. **Arjun** — independent-support minimization with resolution-based
   elimination, backbone and equivalence detection, and optional SBVA.
   **Renumbers.** Turned off by `--no-arjun`.

### `pmc` and `pwmc`

A different chain. Every stage is exactly ×1 for the projected count, and only
Arjun renumbers, so there is one map to compose.

1. **Arjun projection-set minimization** — shrinks the show set and removes
   non-show variables that are free or determined. Runs *first* here, unlike the
   count chain. Turned off by `--no-arjun`.
2. **Count-preserving unit propagation** — propagates to fixpoint, then re-pins
   each forced show variable as a unit clause so it still contributes ×1 rather
   than ×2.
3. **Show-frozen DVE** — the same elimination as the count chain's step 7, but
   frozen on the show set: only hidden variables go, and a show variable can be
   merged away only into another show variable.
4. **Projected BVE** — resolves away projected-out variables, bounded so the
   clause count cannot grow.

Under the default `ProjectionPolicy::Full`, steps 2–4 always run;
`--no-arjun` is the only command-line toggle this chain has. An embedded
caller can run Arjun alone through `RunConfig::projection_policy`.

### `compile`

Steps 1–5 of the count chain, and nothing after them. Gate detection, DVE,
Arjun, BVE and SBVA each remove a variable determined by a *function* of the
survivors, and a map entry names a literal, not a function; so `compile`
removes only backbone literals, equivalence partners and unused variables,
which is what makes `original_to_reduced_dimacs` total and an assignment
liftable with no propagation. `reduced_weights` and `show_vars_reduced_dimacs`
are the input's, renumbered.

### Steps that can be discarded

A step can run and then be discarded wholesale, so its presence in the list does
not mean it shaped the output:

- **DVE under `mc`/`wmc`** is thrown away unless it eliminated enough to be
  worth the renumbering.
- **DVE under `wmc`** is additionally reverted if it eliminated a variable whose
  two polarities carry different weights in a way no rational factor corrects.
- **Show-frozen DVE** reverts to the pre-DVE formula if a show-variable
  equivalence chain fails to resolve to a surviving show variable.
- **Arjun**, in all four counting modes, is kept only if its verdict says it
  helped and its variable map is injective. An embedded caller can change the
  clause-count verdict through `RunConfig::arjun_clause_growth`.

`PreprocessBundle::stages` reports each of these, separating a step that ran
out of budget (`StageOutcome::GaveUp`, where the late-result rule per mode is
written) from one whose result was refused; `PreprocessBundle::telemetry`
reports the work attempted. The weighted DVE revert and the Arjun discards
also print a `c note:` line when diagnostics are on (`vitri` turns them on; a
library caller does with `diagnostics::set_verbose`). `RunConfig::arjun_budget`
sizes Arjun's share of the wall.

### Disabling preprocessing

Under `mc` and `wmc`, `--no-simplify` and `--no-arjun` together give a bundle
with no preprocessing at all. `compile` has no Arjun stage, so `--no-simplify`
alone does it there, and `--no-arjun` is refused rather than ignored. A
projected mode has no such recipe: it has no simplify chain, so it refuses
`--no-simplify` in the same way, and `--no-arjun` drops only its first step
because steps 2–4 always run. Neither flag changes the answer.

## Operations on derived formulas

A compiler working on a derived formula — a component, a cofactor, a
conditioned residual — can reach one step of the chains without rerunning the
pipeline: `cnf::propagate_units`, `projection::eliminate_hidden` and
`projection::classify_hidden_defined_by_show`.

## Lifting an assignment

1. Read each reduced variable through `reduced_to_original_dimacs`, which is
   signed — a variable can come back negated.
2. Set every literal in `forced_literals_original_dimacs` to its polarity.
3. Choose freely for every variable in `free_vars_original_dimacs` — that is
   where the `2^k` models come from.

The result is partial: a variable the equivalence reduction or DVE removed is
determined by the others and appears in neither map, and unit propagation over
the original formula recovers it. Under `compile` nothing is partial — lift
through `original_to_reduced_dimacs` instead. Under a projected mode the
reduced formula's models are not models of the input at all; what lifts back
is a show-projection, where a feasible assignment of the retained show
variables names one of their originals.
