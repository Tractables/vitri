# Environment variables

Every run-time knob `vitri` reads is named `VITRI_*`. The build reads four
more, three of which are not; they are at the end of this page and in
[`building.md`](building.md). **None of them is required**: unset,
each one takes the default in the tables below, and the default column
*is* the production configuration — the `vitri` binary with no variables set and
the library with `RunConfig::default()` do the same thing.

They are research knobs. They exist so a run can be varied without a rebuild;
they are not a second configuration surface beside the command line, and no
supported behaviour depends on setting one.

## Who reads them

- **The `vitri` binary** fills its two configs from the environment at startup,
  through `RunConfig::from_env_defaults` and `SelectionCtx::with_env_defaults`.
  That is not the last read — see the next point.
- **A library caller gets no env-filled config, but its run still reads the
  environment.** `RunConfig::default()` and the `SelectionCtx` constructors
  ignore the environment entirely, so nothing a caller *configures* comes from
  the shell unless it calls the two `*_env_defaults` constructors.
  The vendored stack is the other half of the story: it reads its own three
  with `getenv`, wherever the config came from — but not before this crate has
  read and validated those three itself, in the parent, before any shim exists.
  A value one of them cannot mean fails the run there, rather than being
  silently ignored inside a `getenv` the caller never sees. An embedder that
  wants a run sealed off from the shell that launched the host program should
  clear `VITRI_*` from the environment.

## Values

Flags accept `1`, `on`, `true` and `0`, `off`, `false`.

Whitespace around a value is not part of it, and a value that is a word — a
flag's `on`, `VITRI_ARJUN_SBVA`'s `auto`, `VITRI_PORTFOLIO_TRACE`'s `all` — is
read whatever its case.

`VITRI_BUDGET_MS` is marked *tolerant* below: it is a default for a field the
caller usually sets itself, so a value it cannot parse leaves the run unbounded
exactly as an unset variable does rather than stopping a run that never asked
for a budget.

## The budget

| variable | what it tunes | value | default |
|---|---|---|---|
| `VITRI_BUDGET_MS` | wall-clock hint for the whole run, from which every internal sub-budget is scaled. The default for `RunConfig::budget_ms`; `--budget-ms`, or setting the field, overrides it. **Tolerant** | milliseconds | unset — unbounded |

`RunConfig::construction_budget` decides how much of what is left of that
budget vtree construction may spend, and its variants say what each is for.

## Vtree construction

Read by `SelectionCtx::with_env_defaults`.

| variable | what it tunes | value | default |
|---|---|---|---|
| `VITRI_PORTFOLIO_SEED` | seed for the portfolio's goatd-incidence candidate | non-negative integer | `0` |
| `VITRI_PORTFOLIO_TRACE` | print one `[portfolio] cand …` stderr line per scored candidate, with its scores and the adoption decisions — why the portfolio picked what it picked; `all` additionally builds and scores the hypergraph-bisect family at every imbalance point, including the ones the generation gate skipped | any value, or `all` | unset — no trace |
| `VITRI_GOATD_REFINE_BUDGET_MS` | explicit budget for the goatd refine schedule, overriding the share the portfolio would give it | milliseconds; `0` = take the share | `0` |

### Projected selection

Read at the same place; only a projected (`pmc` / `pwmc`) run consults them.

| variable | what it tunes | value | default |
|---|---|---|---|
| `VITRI_PMC_FLOWCUTTER_CAP_MS` | wall-clock cap on the projected `flowcutter-primal` candidate. It applies only where peak-width selection is active *and* the component has more than 2000 variables; every other candidate on every other component runs uncapped | milliseconds; `0` = no cap | `0` |

## Preprocessing

Each of these sets a field of `RunConfig::arjun`, read by
`RunConfig::from_env_defaults` and named in the row. A caller that runs two
reductions in one process sets the fields instead: a variable is process-global
and would reach both.

| variable | what it tunes | value | default |
|---|---|---|---|
| `VITRI_ARJUN_SBVA` (`sbva`) | whether Arjun's bounded variable addition runs. `auto` runs it unless the input looks like a graph-colouring encoding, where the rewritten clause set loses the decomposition the vtree portfolio would otherwise find. All three are count-preserving | `on`, `off`, or `auto` | `on` |
| `VITRI_ARJUN_EFFORT` (`effort`) | which Arjun reduction runs: `full` is the whole pipeline; `lite` is BCP, backbone/probing and equivalent-literal substitution only — no SBVA, no BVE, oracle off. Both preserve the count | `full` or `lite` | `full` |
| `VITRI_ARJUN_KEEP_OVERRUN` (`keep_overrun`) | keep a full-count reduction that finished past its budget instead of discarding it. Off because a more-reduced formula bought with budget the caller no longer has is not reliably more compilable | flag | off |
| `VITRI_PMC_ARJUN_ORACLE_MAX_VARS` (`oracle_max_vars.projected`) | variable count above which the projected (`pmc`) pre-pass skips Arjun's oracle. Capped by default: the projected paths keep their checkpoint regardless of overrun, so on a large formula an oracle overrun can consume the whole budget | variable count | `100000` |
| `VITRI_PWMC_ARJUN_ORACLE_MAX_VARS` (`oracle_max_vars.weighted_projected`) | the same cap for the projected weighted (`pwmc`) pre-pass | variable count | `100000` |
| `VITRI_ARJUN_EXPORT_LEARNED_CLAUSES` (`export_learned_clauses`) | harvest the redundant clauses Arjun's internal solver derived and return them alongside the reduced formula — on `PreprocessBundle::learnt_clauses_reduced_dimacs`, in `reduced.cnf`'s own numbering — for a consumer that wants to feed them to its own solver. In-process only: no bundle file carries them (the `vitri` binary reports how many it harvested and keeps them nowhere), and each is implied by `reduced.cnf`, so a consumer may drop them freely. Only mode `mc` runs the Arjun stage that harvests, so asking for them under another mode or with `--no-arjun` is an error rather than an empty list | flag | off |
| `VITRI_ARJUN_SEED` (`seed`) | seed Arjun's internal randomization. Every seed gives a sound reduction; different seeds give different ones, which re-rolls everything downstream | unsigned integer | `42`, Arjun's own |

## The vendored Arjun stack

| variable | what it tunes | value | default |
|---|---|---|---|
| `VITRI_ARJUN_NO_BVE` | disable bounded variable elimination, so functionally defined gate variables survive into the reduced CNF | presence-only | unset — BVE runs |
| `VITRI_ARJUN_BVE_GROW` | clamp the BVE clause-growth budget. `0` eliminates a variable only when doing so does not increase the clause count | whole number from 0 to 2147483647 | unset — Arjun's own budget |
| `VITRI_ARJUN_NO_ORACLE` | force Arjun's oracle passes off regardless of the remaining budget | presence-only | unset — the budget decides |

**Presence-only** is not the flag spelling above: the two switches turn their
pass off by *being set*, whatever they are set to. `VITRI_ARJUN_NO_BVE=0` would
therefore turn BVE off, which is the opposite of what it looks like, so an
off-looking value — `0`, `off`, `false`, or empty — is refused with an error
instead of obeyed. To leave the pass on, leave the variable unset.

## Build-time variables

Read by `build.rs` rather than by the program, so a change takes effect on the
next build rather than the next run. See [`building.md`](building.md).

| variable | what it does |
|---|---|
| `VITRI_CXX` | the C++ compiler to build the vendored stack with, overriding the `g++-14` / `g++-13` / `g++-12` search |
| `DOCS_RS` | set by docs.rs; skips the native build so rustdoc can type-check the crate without CMake or GMP/MPFR |
| `AR` | the archiver that merges the vendored archives into the single one this crate links, defaulting to `ar` |
| `NUM_JOBS` | set by cargo from `-j`; how many compilations the vendored CMake build runs at once |
