# Preprocessing

What each mode does to your formula, and what you must do to get a correct
answer over the original. [`bundle.md`](bundle.md) is the field-by-field
reference for the files described here.

## The one identity

```text
count(original) == count(reduced) * 2^count_lift_pow2 * weight_lift
```

`count` is the mode's own count — plain, weighted, projected or
projected-weighted. The two factors are disjoint, so apply both unconditionally
and never branch on the mode.

| mode | `count_lift_pow2` | `weight_lift` |
| --- | --- | --- |
| `mc`, `pmc` | the whole lift | always `"1/1"` |
| `wmc`, `pwmc` | always `0` | the whole lift |
| `compile` | the unused variables, nothing else | always `"1/1"` |

So: count `reduced.cnf` under `reduced_weights`, and under
`show_vars_reduced_dimacs` if the mode is projected; multiply by both factors.
`weight_lift` is an exact rational `"numerator/denominator"` — keep exact
rational arithmetic throughout, because a float rounds a `1/3` that no later
step recovers.

The mode comes from your file's declarations unless `--mode` overrides them,
and every run reports the mode it used.

## The steps

There is no single pipeline with per-mode switches. The mode picks one of three
chains, and the chains differ in which steps run **and in what order**.

### `mc` and `wmc`

In order. Steps 1–7 are one unit — `--no-simplify` turns off all seven.

1. **Clause simplification** — subsumption, vivification and self-subsumption in
   CaDiCaL. Rewrites clauses; removes no variable.
2. **Equivalence detection** — strongly connected components over the binary
   clauses. Rewrites onto a class representative but keeps every variable, so
   this step alone changes no ids.
3. **Backbone and equivalence probing** — one SAT session that finds forced
   literals, propagates them, re-runs step 2 over the clauses that propagation
   created, then probes for whatever equivalences remain. Time-budgeted.
4. **Backbone and dead-variable stripping** — drops the forced variables and any
   variable no clause mentions. **First renumbering.**
5. **Equivalence reduction** — drops the partners step 2 found, keeping one
   representative per class. **Renumbers.** Under `wmc` a dropped partner's
   weight is folded into its representative's, which is why `reduced_weights`
   comes out of preprocessing, not out of your input.
6. **Gate detection** — finds AND/OR/XOR/ITE outputs. Removes nothing itself; it
   tells step 7 which variables are already known to be defined.
7. **Definability elimination (DVE)** — eliminates defined, free and
   newly-equivalent variables by resolution, looped under a round and time
   budget. The time budget bounds the vivification each round ends with as well
   as the rounds themselves, so a formula whose vivification runs long gets a
   weaker reduction rather than a longer pass. **Renumbers.**
8. **Arjun** — independent-support minimization with resolution-based
   elimination, backbone and equivalence detection, and optional SBVA.
   **Renumbers.** Turned off by `--no-arjun`.

### `pmc` and `pwmc`

A different chain, not the one above with steps disabled. Every stage is exactly
×1 for the projected count, and **only Arjun renumbers** — the rest preserve
variable ids by design, so there is just one map to compose.

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

Steps 2–4 always run; `--no-arjun` is the only toggle this chain has.

### `compile`

Steps 1–5 of the count chain, and nothing after them.

Gate detection, DVE, Arjun, BVE and SBVA are all excluded on purpose: each
removes a variable determined by a *function* of the survivors, and a map entry
names a literal, not a function. So `compile` removes only backbone literals,
equivalence partners and unused variables — every other variable survives, which
is what makes `original_to_reduced_dimacs` total and an assignment liftable with
no propagation.

`reduced_weights` and `show_vars_reduced_dimacs` are carried through unchanged
here, not folded — under `compile` alone they *are* yours, renumbered.

### Steps that undo themselves

A step can run and then be discarded wholesale, so its presence in the list is
not a promise that it shaped the output:

- **DVE under `mc`/`wmc`** is thrown away unless it eliminated enough to be
  worth the renumbering.
- **DVE under `wmc`** is additionally reverted if it eliminated a variable whose
  two polarities carry different weights in a way no rational factor corrects.
- **Show-frozen DVE** reverts to the pre-DVE formula if a show-variable
  equivalence chain fails to resolve to a surviving show variable.
- **Arjun**, in all four counting modes, is kept only if its verdict says it
  helped and its variable map is injective.

Each of these is reported when it fires: a `c note:` line on stderr names the
step and why it went. The bundle itself describes only the preprocessing that
survived.

### Turning it off

Under `mc` and `wmc`, `--no-simplify` and `--no-arjun` together give a bundle
with no preprocessing at all. `compile` has no Arjun stage, so `--no-simplify`
alone does it there — and `--no-arjun` is refused rather than ignored. A
projected mode has no such recipe at all: it has no simplify chain, so it
refuses `--no-simplify` in the same way, and `--no-arjun` drops only its first
step because steps 2–4 always run.

Neither flag can change the answer — both only decide how much work happens
before you get it.

## Read the show set and the weights from the bundle

Counting `reduced.cnf` over your original show ids, or under your own weights,
is a silently wrong count: `show_vars_reduced_dimacs` and `reduced_weights`
both come out of preprocessing, not out of your input.

An empty show set is a real answer: `c p show 0` means every show variable was
retired, so the projected count is 1 if `reduced.cnf` is satisfiable and 0 if
not. It does not mean "unprojected".

## Lifting an assignment

1. Read each reduced variable through `reduced_to_original_dimacs`, which is
   signed — a variable can come back negated.
2. Set every literal in `forced_literals_original_dimacs` to its polarity.
3. Choose freely for every variable in `free_vars_original_dimacs` — that is
   where the `2^k` models come from.

The result is partial: a variable the equivalence reduction or DVE removed is
determined by the others and appears in neither map, and unit propagation over
your original formula recovers it. Under `compile` nothing is partial — lift
through `original_to_reduced_dimacs` instead. Under a projected mode the
reduced formula's models are not models of your input at all; what lifts back
is a show-projection, where a feasible assignment of the retained show
variables names one of their originals.

## Refutation

If preprocessing proves the instance unsatisfiable, `unsat` is `true` and the
count is 0. `reduced.cnf` then holds an explicit contradiction (`x` and `¬x`)
rather than the empty clause, which DIMACS cannot portably spell.
