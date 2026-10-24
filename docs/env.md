# Environment variables

Every run-time knob `vitri` reads is named `VITRI_*`; the build-time variables
are in [`building.md`](building.md#build-time-variables). None is required:
unset, each one takes the default in the tables below, and the default column is
the production configuration — the `vitri` binary with no variables set and
the library with `RunConfig::default()` do the same thing.

## Who reads them

The `vitri` binary fills its two configs from the environment at startup,
through `RunConfig::from_env_defaults` and `SelectionCtx::with_env_defaults`.
A library caller, `request::prepare` included, gets no env-filled config:
`RunConfig::default()` and the `SelectionCtx` constructors ignore the
environment. The vendored Arjun stack reads its own three in every process,
whatever the config says ([below](#the-vendored-arjun-stack)); vitri checks
them first, so a value they cannot use fails the run. The vendored CaDiCaL
reads `CADICAL_<option>` variables, which override its option defaults in every
solver in the process. An embedder that wants a run independent of its shell
clears `VITRI_*` and `CADICAL_*`.

## Values

Flags accept `1`, `on`, `true` and `0`, `off`, `false`.

Whitespace around a value is not part of it, and a value that is a word — a
flag's `on`, `VITRI_ARJUN_SBVA`'s `auto`, `VITRI_PORTFOLIO_TRACE`'s `all` — is
read whatever its case.

`VITRI_BUDGET_MS` is marked *tolerant* below: a value it cannot parse leaves
the run unbounded, as if unset.

## The budget

| variable | what it tunes | value | default |
|---|---|---|---|
| `VITRI_BUDGET_MS` | wall-clock hint for the whole run, from which every internal sub-budget is scaled. The default for `RunConfig::budget_ms`; `--budget-ms`, or setting the field, overrides it. **Tolerant** | milliseconds | unset — unbounded |

## Vtree construction

Read by `SelectionCtx::with_env_defaults`.

| variable | what it tunes | value | default |
|---|---|---|---|
| `VITRI_PORTFOLIO_SEED` | `PortfolioKnobs::seed` | non-negative integer | `0` |
| `VITRI_PORTFOLIO_TRACE` | `PortfolioKnobs::trace`: one stderr line per scored candidate; `all` also builds and scores the candidates the generation gate skips | any value, or `all` | unset — no trace |
| `VITRI_PORTFOLIO_SKIP` | `PortfolioKnobs::skip`, in place of the default list; a name the catalog does not have is refused, and so is a list naming every entry | entry names separated by `;`; empty leaves none out | unset — `decompose::DEFAULT_SKIP` |
| `VITRI_CONVERSION_TRACE` | `ConversionKnobs::trace` | any value | unset — no trace |
| `VITRI_GOATD_REFINE_BUDGET_MS` | `GoatdKnobs::refine_budget_ms` | milliseconds; `0` = take the share | `0` |
| `VITRI_GOATD_FINAL_POLISHING` | `GoatdKnobs::polishing`, off or its default policy | flag | on |
| `VITRI_GOATD_CANDIDATES` | `GoatdKnobs::candidates` | `1` to `8` | `4` |
| `VITRI_SCORE_AGG` | `PortfolioKnobs::ranker`: the whole-tree ranker a `portfolio` build selects on, or `cost` to select on `score::vtree_cost` alone | `cost`, or the path of an exported ranker in JSON | unset — the ranker shipped in the crate |
| `VITRI_SCORE_AGG_MARGIN` | `PortfolioKnobs::margin`; `none` ranks every candidate. Set under `VITRI_SCORE_AGG=cost` it is refused | a number, zero or more, or `none` | `10` |

### Projected selection

Read at the same place; only a projected (`pmc` / `pwmc`) run consults them.

| variable | what it tunes | value | default |
|---|---|---|---|
| `VITRI_PMC_FLOWCUTTER_CAP_MS` | `PortfolioKnobs::flowcutter_cap_ms` | milliseconds; `0` = no cap | `0` |

## Preprocessing

Each of these sets a field of `RunConfig::arjun`, read by
`RunConfig::from_env_defaults` and named in the row.

| variable | what it tunes | value | default |
|---|---|---|---|
| `VITRI_ARJUN_SBVA` (`sbva`) | whether Arjun's bounded variable addition runs; `ArjunSbva` says what each spelling does | `on`, `off`, or `auto` | `on` |
| `VITRI_ARJUN_EFFORT` (`effort`) | which Arjun reduction runs; `ArjunEffort` says what each is | `full` or `lite` | `full` |
| `VITRI_ARJUN_KEEP_OVERRUN` (`keep_overrun`) | keep an `mc` reduction that finished past its budget instead of discarding it; `mc` only, with the Arjun stage on | flag | off |
| `VITRI_PMC_ARJUN_ORACLE_MAX_VARS` (`oracle_max_vars.projected`) | variable count above which the `pmc` pre-pass skips Arjun's oracle | variable count | `100000` |
| `VITRI_PWMC_ARJUN_ORACLE_MAX_VARS` (`oracle_max_vars.weighted_projected`) | the same cap for the `pwmc` pre-pass; `0` skips the oracle at every size | variable count | `0` |
| `VITRI_ARJUN_SKIP_MONOTONE` (`skip_monotone`) | skip Arjun under `mc` on a monotone formula; `mc` only, with the Arjun stage on | flag | off |
| `VITRI_ARJUN_EXPORT_LEARNED_CLAUSES` (`export_learned_clauses`) | harvest the redundant clauses Arjun's internal solver derived onto `PreprocessBundle::learnt_clauses_reduced_dimacs`; `mc` only, and no bundle file carries them | flag | off |
| `VITRI_ARJUN_SEED` (`seed`) | seed Arjun's internal randomization; a different seed gives a different sound reduction | unsigned integer | `42`, Arjun's own |

## The vendored Arjun stack

| variable | what it tunes | value | default |
|---|---|---|---|
| `VITRI_ARJUN_NO_BVE` | disable bounded variable elimination, so functionally defined gate variables survive into the reduced CNF | presence-only | unset — BVE runs |
| `VITRI_ARJUN_BVE_GROW` | clamp the BVE clause-growth budget. `0` eliminates a variable only when doing so does not increase the clause count | whole number from 0 to 2147483647 | unset — Arjun's own budget |
| `VITRI_ARJUN_NO_ORACLE` | force Arjun's oracle passes off regardless of the remaining budget | presence-only | unset — the budget decides |

**Presence-only** is not the flag spelling above: the two switches turn their
pass off by *being set*, whatever they are set to. `VITRI_ARJUN_NO_BVE=0` would
therefore turn BVE off, the opposite of what it looks like, so an off-looking
value — `0`, `off`, `false`, or empty — is refused with an error instead of
obeyed. To leave the pass on, leave the variable unset.
