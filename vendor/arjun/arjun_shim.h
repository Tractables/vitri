/* arjun_shim.h — narrow C ABI over ArjunNS::Arjun for this crate's in-process,
 * anytime preprocessing path. Lets the Rust side drive Arjun stage-by-stage and
 * read a SOUND checkpoint (reduced CNF + multiplier) off the SimplifiedCNF after
 * every stage — so a deadline hit between stages still yields a usable, sound
 * partial reduction instead of the subprocess path's "SIGKILL → discard → raw".
 *
 * Soundness: get_multiplier_weight() travels WITH the SimplifiedCNF object, so
 * (current reduced clauses, current multiplier) is always a consistent pair —
 * no stdout-scrape / CNF-multiplier mismatch hazard.
 *
 * All readback getters are probe-then-fill: call with cap=0 (or too-small cap)
 * to get the required length, allocate, call again.
 */
#ifndef VITRI_ARJUN_SHIM_H
#define VITRI_ARJUN_SHIM_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ArjunShim ArjunShim;

/* Lifecycle. Integer (FGenMpz) field — the unweighted path. `seed` seeds
 * Arjun's internal RNG; 42 is Arjun's own default. */
ArjunShim *arjun_shim_new(uint32_t seed);
/* Weighted (FGenMpq, rational) field — the --weighted (WMC) path, equivalent to
 * arjun_bin's `--mode 1`. set_weighted(true) is applied on the fresh
 * SimplifiedCNF so set_lit_weight is legal. The travelling multiplier is then a
 * rational (num/den), read back via arjun_shim_cur_multiplier unchanged. */
ArjunShim *arjun_shim_new_weighted(uint32_t seed);
void arjun_shim_free(ArjunShim *s);

/* Weighted-mode ingest/readback (only valid on a shim from
 * arjun_shim_new_weighted). `weight_str` is a decimal or rational `num/den`
 * literal weight, parsed via the field's own parser (the same path the DIMACS
 * `c p weight` reader uses). Setting both polarities of a var with their
 * explicit weights reproduces arjun_bin's per-literal weight ingest exactly. */
void arjun_shim_set_lit_weight(ArjunShim *s, int32_t lit, const char *weight_str);
/* Replicate the no-`c p show` branch of arjun_bin's read_in_a_file: push every
 * variable into the sampling AND opt-sampling sets (full weighted count). Must
 * be called after all clauses/weights are added and before the stages; pair with
 * all_indep=1 so eliminated mass folds into the multiplier instead of being
 * projected away (else the multiplier collapses to 1). */
void arjun_shim_clean_sampl(ArjunShim *s);
/* Current per-literal weight as a NUL-terminated rational/decimal string
 * (get_lit_weight(lit)->display); probe-then-fill like arjun_shim_cur_multiplier.
 * Returns 1 (the field one()) for a var with no explicit weight. `lit` is DIMACS
 * (1-based signed) in the CURRENT (reduced/renumbered) variable space. */
size_t arjun_shim_lit_weight(ArjunShim *s, int32_t lit, char *buf, size_t cap);

/* Build the input formula. Vars are 0-based internally; clause `lits` are
 * standard DIMACS (1-based, signed, NO trailing 0). */
void arjun_shim_new_vars(ArjunShim *s, uint32_t n);
void arjun_shim_add_clause(ArjunShim *s, const int32_t *lits, size_t n);
void arjun_shim_set_sampl(ArjunShim *s, const uint32_t *vars0, size_t n);
void arjun_shim_set_verb(ArjunShim *s, uint32_t verb);
/* Lite-config knob: cap the Puura backbone/probing effort inside the heavy
 * simplify stage to `max_confl` conflicts (SimpConf::backbone_max_confl).
 * `-1` (the value if this is never called) is Arjun's default (unlimited), so
 * the full-config path stays byte-identical when it is not set. Applies to
 * every subsequent arjun_shim_stage_simplify call. Count-preserving — it only
 * bounds backbone search effort, never changes the model count. */
void arjun_shim_set_backbone_max_confl(ArjunShim *s, int64_t max_confl);

/* Bound the ACTUAL work of the heavy stage's oracle (oracle-vivif / -sparsify)
 * by scaling SimpConf::oracle_mult. Arjun's oracle sizes its SAT effort as a
 * fixed constant * global_timeout_multiplier * oracle_mult "mems" (memory-op)
 * budget per pass (CryptoMiniSat oracle_use.cpp), so oracle_mult is a LINEAR
 * scalar on total oracle SAT work: halving it ~halves the oracle's worst-case
 * runtime. `mult < 0` (the value if this is never called) leaves SimpConf's default
 * (1.0) untouched, so the full-config path stays byte-identical when unset.
 * Applies to every subsequent arjun_shim_stage_simplify call.
 *
 * COUNT-PRESERVING / SOUND at any value: the oracle removes a clause only when
 * its SAT query PROVES the clause redundant (oracle_use.cpp: sparsify acts on
 * ret.isFalse only; vivify strengthens only on proven-redundant literals). When
 * the mems budget is exhausted the query returns UNKNOWN and the pass ABORTS
 * (goto fin / goto end1), keeping every not-yet-processed clause intact -> a
 * LARGER-but-exact formula. A smaller oracle_mult can only cause fewer proven
 * removals, yielding a formula strictly between "oracle fully on" and "oracle
 * off" -- both of which are already differential-tested count-preserving. */
void arjun_shim_set_oracle_mult(ArjunShim *s, double mult);

/* Wall-clock deadline for the stages below, in milliseconds FROM NOW.
 * `ms_from_now < 0` clears it (Arjun's default: no deadline, byte-identical to
 * an unpatched build). Must be called before the stages; it is absolute from
 * the moment of the call, so one call covers both stages.
 *
 * This is the in-process counterpart of the fork+SIGKILL budget: instead of
 * killing a stage that overruns and discarding everything it produced, Arjun
 * stops at the deadline and RETURNS. It requires the local Arjun patch
 * (the deadline patch, already applied in `vendor/`) — build.rs refuses to build
 * against an installation whose arjun.h lacks Arjun::set_deadline.
 *
 * SOUNDNESS — a deadline hit is an exact checkpoint, not an approximation.
 * The deadline gates OPTIONAL work only, at points where stopping early yields
 * a LESS-reduced but exactly count-preserving formula: between the steps of
 * elim_to_file (each step is a count-preserving rewrite, so every boundary is
 * a checkpoint), at the top of the independent-support and extend loops (both
 * re-derive their sampling set on exit, so the support comes back
 * larger-but-valid), and at the existing budget-exhausted abort paths of the
 * CryptoMiniSat oracle and the CadiBack backbone (fewer clauses proven
 * redundant, fewer backbone literals proven — never a wrong one). Arjun's
 * finalization (sampling-set cleanup, renumbering, the CNF read-back) is never
 * gated, so the (clauses, sampl_vars, multiplier) triple this shim reads back
 * is internally consistent whether or not the deadline fired. */
void arjun_shim_set_deadline_ms(ArjunShim *s, int64_t ms_from_now);

/* Stages — each advances the internal "most-reduced so far" SimplifiedCNF.
 * Return 0 on success, nonzero if Arjun threw (the checkpoint is left at the
 * last good state). Intended call order: minimize_indep (cheap) then simplify
 * (heavy); the caller checks its deadline BETWEEN them.
 *
 * `simplify` runs the SAME pipeline as arjun_bin's CLI `do_minimize()`:
 * standalone_elim_to_file (extend-indep + autarky + BCE + SBVA + renumber +
 * BVE/oracle simplify), NOT the bare get_simplified_cnf. `all_indep` must match
 * the value passed to minimize_indep (true for full unweighted count). */
int arjun_shim_stage_minimize_indep(ArjunShim *s, int all_indep);
/* `oracle_enabled`: when 0, the expensive oracle simplify passes are disabled so
 * the stage returns in <1s (the rest of the pipeline) instead of overrunning the
 * budget by 10-20s. The caller decides based on remaining budget.
 * `no_sbva`: when non-zero, SBVA (structured bounded variable addition) is
 * disabled (num_sbva_steps=0). SBVA can produce a reduced formula that compiles
 * far worse (apply-OOM) than the no-SBVA reduction; this is count-preserving and
 * used as a sound OOM-triggered revert target. The caller decides; this side
 * reads no environment.
 * `no_bve`: when non-zero, bounded variable elimination is disabled (do_bve=false)
 * so functionally-defined gate vars survive into the reduced CNF (the AIG race
 * lane needs this for ITS reduce only; env VITRI_ARJUN_NO_BVE also forces it).
 * Count-preserving — BVE off only keeps more vars/clauses. */
int arjun_shim_stage_simplify(ArjunShim *s, int all_indep, int oracle_enabled, int no_sbva, int no_bve);

/* Readback of the current checkpoint. */
uint32_t arjun_shim_cur_nvars(ArjunShim *s);
size_t arjun_shim_cur_nclauses(ArjunShim *s);
/* Flatten current clauses into `buf` as DIMACS lits, each clause 0-terminated.
 * Returns total ints required; writes only if cap >= required. */
size_t arjun_shim_cur_clauses(ArjunShim *s, int32_t *buf, size_t cap);
/* Current sampling (independent-support) vars, 0-based. Returns count required. */
size_t arjun_shim_cur_sampl(ArjunShim *s, uint32_t *buf, size_t cap);
/* Backbone literals discovered at the minimize stage (literals forced in every
 * model), DIMACS-signed, in the INPUT variable space. Returns count required. */
size_t arjun_shim_backbone(ArjunShim *s, int32_t *buf, size_t cap);
/* Equivalence literals discovered at the minimize stage, as a FLAT list of
 * pairs: buf = [a0,b0, a1,b1, ...] where each (a,b) encodes a≡b in Arjun's
 * get_all_binary_xors() polarity (DIMACS-signed, INPUT var space). Returns the
 * total int count required (= 2 * number of pairs). */
size_t arjun_shim_eq_lits(ArjunShim *s, int32_t *buf, size_t cap);
/* Redundant/learnt clauses Arjun's internal (patched CaDiCaL / oracle) solver
 * derived during the heavy simplify stage, harvested off the current
 * SimplifiedCNF's `red_clauses` (SimplifiedCNF::get_red_clauses()). Flattened
 * exactly like arjun_shim_cur_clauses — each clause 0-terminated, DIMACS
 * (1-based signed) in the CURRENT (reduced/renumbered) variable space, the SAME
 * numbering as arjun_shim_cur_clauses, so the Rust-side reduced-formula mapping
 * applies to both identically (they read the one `s->cur` object). Returns total
 * ints required; writes only if cap >= required.
 *
 * SOUNDNESS: red_clauses are clauses the simplify pipeline PROVED redundant
 * w.r.t. the (reduced) formula — implied by it (reduced ⊨ C). Populated only
 * when the oracle passes run with SimpConf::oracle_vivify_get_learnts (its
 * default `true`; the shim leaves it untouched unless the oracle is disabled, in
 * which case red_clauses is simply empty). Caps below bound FFI traffic; over-
 * length or over-count clauses are silently skipped (a smaller harvest is always
 * sound). */
size_t arjun_shim_red_clauses(ArjunShim *s, int32_t *buf, size_t cap);

/* INPUT-variable -> CURRENT(reduced)-literal correspondence, read straight off
 * the checkpoint's own `SimplifiedCNF::get_orig_to_new_var()`. Flat PAIRS:
 * buf = [orig0, newlit0, orig1, newlit1, ...] where `orig` is the DIMACS
 * (1-based, always POSITIVE) id of a variable of the formula that was fed in via
 * arjun_shim_new_vars/add_clause, and `newlit` is a DIMACS-signed (1-based)
 * literal in the CURRENT reduced numbering — the SAME numbering as
 * arjun_shim_cur_clauses / arjun_shim_cur_sampl / arjun_shim_red_clauses.
 * Returns the total int count required (= 2 * number of entries).
 *
 * SEMANTICS — the sign carries meaning. `+n` means input var `orig` IS reduced
 * var `n`; `-n` means input var `orig` is the NEGATION of reduced var `n`
 * (Arjun's equivalent-literal replacement can flip polarity, so dropping the
 * sign silently corrupts any model lifted back through this map).
 *
 * COVERAGE — Arjun maintains this map itself, from the identity installed by
 * `new_vars()` onward:
 *   - `get_cnf` (every simplify pass) recomposes it through CryptoMiniSat's
 *     replacement table, DROPPING input vars that became backbone-assigned or
 *     were BVE-eliminated: they have no reduced counterpart, so they simply do
 *     not appear here.
 *   - `fix_mapping_after_renumber` keeps exactly ONE input var per reduced var
 *     when several collapse together, so the entries are injective on the
 *     reduced side.
 *   - the `elim_to_file` renumbering epilogue rewrites the reduced side in
 *     lock-step with the clauses.
 *   - SBVA-introduced reduced variables are NOT in the map at all (SBVA replaces
 *     the clause set and raises nVars without touching the map), i.e. a reduced
 *     variable that appears as no entry's `newlit` was created by the reduction
 *     and has no input counterpart.
 * None of that is gated on Arjun's AIG/synthesis mode, which this shim never
 * enables. */
size_t arjun_shim_orig_to_new(ArjunShim *s, int32_t *buf, size_t cap);

/* Multiplier as a NUL-terminated decimal string (exact, arbitrary precision):
 * original_count = reduced_count * multiplier. Returns strlen required (excl NUL). */
size_t arjun_shim_cur_multiplier(ArjunShim *s, char *buf, size_t cap);

#ifdef __cplusplus
}
#endif

#endif /* VITRI_ARJUN_SHIM_H */
