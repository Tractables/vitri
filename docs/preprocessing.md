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
   created, then probes for whatever equivalences remain. Time-budgeted. A
   backbone probe that stops undecided at its conflict count is asked again at
   once, with the candidates after it, in a probe that runs to an answer. Each
   equivalence probe also stops at a fixed conflict count, and the second probe
   that ends undecided ends the probing; the equivalences proved before it are
   kept, and an undecided pair is never assumed equivalent.
3. **Clause simplification** in CaDiCaL. Rewrites clauses; removes no variable.
   A second round runs when step 1, re-run over the result, finds an
   equivalence class that was not there before.
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
   **Renumbers.** Turned off by `--no-arjun`. Under `mc`, skipped on a
   monotone formula ([below](#arjun-on-a-monotone-formula)).

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
   clause count cannot grow. Stops at the run's deadline with the eliminations
   it finished, which leave the projected count unchanged.

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

### Arjun on a monotone formula

Under `mc`, Arjun is skipped when the formula it would be handed (what steps
1–7 left, or the input under `--no-simplify`) is monotone up to renaming: every
variable occurs, always in the same polarity, and every clause names two
variables or more. `PreprocessBundle::stages` reports the skip as
`StageOutcome::Skipped(SkipReason::Monotone)`. Like a stage that is turned off
or has no variables left to work on, it is a skip gate with no setting of its
own.

Arjun has no variable to eliminate from such a formula. Rename each variable so
that it occurs positively. The assignment making every variable true satisfies
the formula, and so does each assignment that differs from it in one variable,
since every clause keeps a true literal on another variable. Both values of
every variable therefore extend one assignment of the others: no variable is a
backbone, none is defined by the others, and the only independent support is the
whole variable set. The variables Arjun eliminates are backbones, variables
defined by the ones it keeps, and variables the formula does not depend on,
which it drops for a factor of two each; requiring every variable to occur rules
out the last kind, with one exception below.

What the skip forgoes is Arjun's work on clauses: removing a clause that another
clause subsumes, which can leave a variable in no clause at all (the exception:
a variable whose every clause is subsumed by one without it), and bounded
variable addition, which rewrites clauses through variables it adds. Step 3
removes subsumed clauses before Arjun runs, though within its budget, so not
provably all of them.

The other counting modes run the stage. Under `wmc`, a weight of 0 on one
literal of a variable discards every model that sets that literal, so the count
can treat as forced a variable the formula leaves free. Under `pmc` and `pwmc`,
a variable outside the show set can be projected away whether or not anything
defines it.

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
sizes Arjun's share of the wall. Arjun stops where it is when that share runs
out, independent-support minimization included, and keeps what it has: a
variable leaves the support only once shown to be determined by the others.

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
