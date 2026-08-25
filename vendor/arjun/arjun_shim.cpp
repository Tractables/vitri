/* arjun_shim.cpp — implementation of the C ABI declared in arjun_shim.h.
 * See that header for the soundness rationale (multiplier travels with the
 * SimplifiedCNF; every stage boundary is a sound checkpoint). */
#include "arjun_shim.h"
#include "arjun.h"

#include <memory>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>
#include <cerrno>
#include <climits>
#include <cstdint>
#include <cstring>
#include <cstdlib>
#include <cstdio>

using ArjunNS::Arjun;
using ArjunNS::SimplifiedCNF;
using ArjunNS::SimpConf;
using ArjunNS::FGenMpz;
using ArjunNS::FGenMpq;
using CMSat::Field;
using CMSat::FieldGen;
using CMSat::Lit;

// A `VITRI_*` numeric value, in full. `strtoul`/`strtol` with a null end pointer
// stop at the first character they cannot use and report the digits before it,
// so `xyz` reads as 0 and `12x` as 12 — a configuration nobody asked for,
// applied silently. Everything the value contains has to be part of the number.
//
// The Rust side rejects a malformed value before it builds a shim, so a value
// that fails here means the two sides disagree; refusing is the only answer that
// cannot run the wrong configuration.
static bool parse_whole_number(const char *v, long lo, long hi, long *out) {
    if (v == nullptr || *v == '\0') return false;
    char *end = nullptr;
    errno = 0;
    const long parsed = strtol(v, &end, 10);
    if (errno != 0 || end == v || *end != '\0') return false;
    if (parsed < lo || parsed > hi) return false;
    *out = parsed;
    return true;
}

struct ArjunShim {
    std::unique_ptr<FieldGen> fg;
    std::unique_ptr<Arjun> arjun;
    std::unique_ptr<SimplifiedCNF> cur;  // most-reduced checkpoint so far
    uint32_t verb = 0;
    // Lite-config backbone/probing budget (conflicts) for the heavy simplify
    // stage's SimpConf. -1 = Arjun default (unlimited); set via
    // arjun_shim_set_backbone_max_confl. Default keeps the full path unchanged.
    int64_t backbone_max_confl = -1;
    // Heavy simplify stage's oracle effort scalar (SimpConf::oracle_mult). < 0
    // = unset -> leave SimpConf's default (1.0), so the full path is unchanged.
    // Set via arjun_shim_set_oracle_mult; bounds the oracle's SAT work linearly.
    double oracle_mult = -1.0;
    // Backbone + equivalence info harvested at the minimize stage, in the
    // INPUT variable space (minimize runs with set_renumber(0) and filters to
    // var() < nVars(), so these literals index the fed formula directly — no
    // renumber translation needed). DIMACS-signed (1-based). `eq_dimacs` is a
    // flat list of literal PAIRS (each consecutive 2 ints is one a≡b pair, in
    // Arjun's get_all_binary_xors() polarity convention).
    std::vector<int32_t> backbone_dimacs;
    std::vector<int32_t> eq_dimacs;

    // `weighted` selects the field generator: FGenMpz (integer, unweighted) or FGenMpq
    // (rational, --weighted / arjun_bin --mode 1). For the weighted field we also
    // flip the SimplifiedCNF's weighted flag up-front so set_lit_weight is legal
    // (it exit()s if the cnf is not weighted). Everything downstream (the stages,
    // the multiplier readback) is field-agnostic and unchanged.
    // `seed` seeds Arjun's internal RNG. Every seed yields a sound reduction and
    // a different one, re-rolling everything downstream. The caller passes
    // Arjun's own default (42) when it has no opinion, which is byte-identical
    // to not setting it at all.
    ArjunShim(bool weighted, uint32_t seed)
        : fg(weighted ? std::unique_ptr<FieldGen>(new FGenMpq)
                      : std::unique_ptr<FieldGen>(new FGenMpz)),
          arjun(new Arjun),
          cur(std::make_unique<SimplifiedCNF>(fg)) {
        arjun->set_verb(0);
        arjun->set_seed(seed);
        if (weighted) cur->set_weighted(true);
    }

};

// CMSat::Lit (0-based var + sign) -> DIMACS (1-based, signed).
static inline int32_t lit_to_dimacs(const Lit &l) {
    int32_t d = (int32_t)(l.var() + 1u);
    return l.sign() ? -d : d;
}

extern "C" {

// A failed construction is reported by the null return; there is no shim yet, so
// there is no verbosity setting to consult and nothing to write to stderr on
// behalf of a caller that may want none.
ArjunShim *arjun_shim_new(uint32_t seed) {
    try {
        return new ArjunShim(false, seed);
    } catch (...) {
        return nullptr;
    }
}

ArjunShim *arjun_shim_new_weighted(uint32_t seed) {
    try {
        return new ArjunShim(true, seed);
    } catch (...) {
        return nullptr;
    }
}

void arjun_shim_free(ArjunShim *s) { delete s; }

void arjun_shim_new_vars(ArjunShim *s, uint32_t n) { s->cur->new_vars(n); }

void arjun_shim_add_clause(ArjunShim *s, const int32_t *lits, size_t n) {
    std::vector<Lit> cl;
    cl.reserve(n);
    for (size_t i = 0; i < n; i++) {
        int32_t l = lits[i];
        uint32_t var = (uint32_t)(l < 0 ? -l : l) - 1u;  // DIMACS 1-based -> 0-based
        cl.push_back(Lit(var, l < 0));
    }
    s->cur->add_clause(cl);
}

void arjun_shim_set_sampl(ArjunShim *s, const uint32_t *vars0, size_t n) {
    std::vector<uint32_t> v(vars0, vars0 + n);
    s->cur->set_sampl_vars(v);
}

void arjun_shim_set_verb(ArjunShim *s, uint32_t verb) {
    s->verb = verb;
    s->arjun->set_verb(verb);
}

void arjun_shim_set_backbone_max_confl(ArjunShim *s, int64_t max_confl) {
    s->backbone_max_confl = max_confl;
}

void arjun_shim_set_oracle_mult(ArjunShim *s, double mult) {
    s->oracle_mult = mult;
}

void arjun_shim_set_deadline_ms(ArjunShim *s, int64_t ms_from_now) {
    // Straight through to the patched Arjun, which stores it as an absolute
    // CLOCK_MONOTONIC instant and pushes it down into CryptoMiniSat (oracle)
    // and CadiBack (backbone) — the two places the budget is actually spent.
    // Negative clears it. See the header for the soundness contract.
    s->arjun->set_deadline(ms_from_now < 0 ? -1.0 : (double)ms_from_now / 1000.0);
}

int arjun_shim_stage_minimize_indep(ArjunShim *s, int all_indep) {
    try {
        // Use the *_info variant: it performs the identical in-place minimize
        // (run_minimize_indep_info calls run_minimize_indep) AND returns the
        // backbone + equivalence literals the minimize round discovered. We
        // harvest them here (input var space) so the caller can seed a sibling
        // race lane's solver/diagram with constraints that lane never sees.
        ArjunNS::Arjun::IndepInfo info =
            s->arjun->standalone_minimize_indep_info(*s->cur, all_indep != 0);
        s->backbone_dimacs.clear();
        s->backbone_dimacs.reserve(info.backbone.size());
        for (const auto &l : info.backbone) s->backbone_dimacs.push_back(lit_to_dimacs(l));
        s->eq_dimacs.clear();
        s->eq_dimacs.reserve(info.eq_lits.size() * 2);
        for (const auto &p : info.eq_lits) {
            s->eq_dimacs.push_back(lit_to_dimacs(p.first));
            s->eq_dimacs.push_back(lit_to_dimacs(p.second));
        }
        return 0;
    } catch (const std::exception& e) {
        if (s->verb > 0) fprintf(stderr, "[arjun-shim] stage_minimize_indep: %s\n", e.what());
        return 1;
    } catch (...) {
        if (s->verb > 0) fprintf(stderr, "[arjun-shim] stage_minimize_indep: unknown failure\n");
        return 1;
    }
}

int arjun_shim_stage_simplify(ArjunShim *s, int all_indep, int oracle_enabled, int no_sbva, int no_bve) {
    try {
        // Match arjun_bin's CLI `do_minimize()`: run the full elim-to-file
        // pipeline (extend-indep + autarky + BCE + SBVA + renumber + simplify),
        // not the bare get_simplified_cnf (BVE only). The default ElimToFileConf
        // mirrors the CLI defaults (do_extend_indep=true, do_autarky=true,
        // do_renumber=true, num_sbva_steps=1000, do_bce=false); only all_indep
        // is set per-call. elim_to_file rewrites *cur in place and updates the
        // travelling multiplier, so the (clauses, multiplier) checkpoint stays
        // sound — the count is preserved exactly (differential-tested).
        Arjun::ElimToFileConf etof;
        etof.all_indep = all_indep != 0;
        // Disable SBVA in the reduction. SBVA (structured bounded variable
        // addition) can produce a reduced formula that compiles far worse than
        // the no-SBVA reduction on some instances. num_sbva_steps=0 is
        // count-preserving (differential-tested). The decision is entirely the
        // caller's: the `no_sbva` arg carries VITRI_ARJUN_SBVA (resolved
        // Rust-side, in one place, by `decompose::arjun_sbva_skip`) OR a retry
        // after a memory blow-up. This side reads no environment.
        if (no_sbva) {
            etof.num_sbva_steps = 0;
        }
        SimpConf sc;
        // Lite-config backbone/probing budget (conflicts). Default -1 equals
        // SimpConf's own default, so the full-config path is byte-identical when
        // arjun_shim_set_backbone_max_confl was never called. Count-preserving.
        sc.backbone_max_confl = s->backbone_max_confl;
        // Oracle effort scalar. Default -1 (unset) leaves SimpConf's own default
        // (1.0), so the full-config path is byte-identical when
        // arjun_shim_set_oracle_mult was never called. Count-preserving at any
        // value (see the header): a smaller mult only lets the oracle prove fewer
        // clause removals before its mems budget aborts the pass, yielding a
        // larger-but-exact reduction. Threads into BOTH get_simplified_cnf passes
        // of elim_to_file (simp_conf2 copies it), bounding total oracle work.
        if (s->oracle_mult >= 0.0) sc.oracle_mult = s->oracle_mult;
        // VITRI_ARJUN_NO_BVE: disable bounded variable elimination entirely so
        // functionally-defined gate variables SURVIVE into the reduced CNF instead
        // of being resolved away, for a consumer that can exploit them.
        // Count-preserving — BVE off only keeps MORE vars/clauses, never changes
        // the count. Propagates to both elim_to_file simplify passes (do_bve is
        // honored regardless of the per-pass bve_grow override).
        // Per-call `no_bve` OR the process-global env override; the per-call arg
        // lets one reduce disable BVE without forcing every reduce to.
        if (no_bve || getenv("VITRI_ARJUN_NO_BVE")) sc.do_bve = false;
        // VITRI_ARJUN_BVE_GROW=N: clamp the BVE clause-growth budget to N (default
        // iter1=0, iter2=6). N=0 = eliminate a var only when it does NOT increase the
        // clause count. Affects pass 1 only; pass 2 hardcodes grow=0 in the arjun lib.
        if (const char *e = getenv("VITRI_ARJUN_BVE_GROW")) {
            long g = 0;
            if (!parse_whole_number(e, 0, (long)INT_MAX, &g)) {
                if (s->verb > 0)
                    fprintf(stderr,
                            "[arjun-shim] VITRI_ARJUN_BVE_GROW is not a clause-growth budget\n");
                return 1;
            }
            sc.bve_grow_iter1 = (int)g;
            sc.bve_grow_iter2 = (int)g;
        }
        // The oracle passes (oracle_extra/vivify/sparsify) dominate this stage and
        // can run far past the budget without a poll site to interrupt them
        // (seconds on hard instances against well under one for the rest of the
        // stage). `oracle_enabled` lets the caller gate them on the remaining
        // budget: keep them when there is runway to finish, since they account for
        // much of the reduction, and drop them when little budget remains so the
        // cheap part (BVE+SBVA+autarky) still returns a sound result in time.
        // VITRI_ARJUN_NO_ORACLE force-disables them regardless.
        if (oracle_enabled == 0 || getenv("VITRI_ARJUN_NO_ORACLE")) {
            sc.oracle_extra = false;
            sc.oracle_vivify = false;
            sc.oracle_vivify_get_learnts = false;
            sc.oracle_sparsify = false;
        }
        s->arjun->standalone_elim_to_file(*s->cur, etof, sc);
        return 0;
    } catch (const std::exception& e) {
        if (s->verb > 0) fprintf(stderr, "[arjun-shim] stage_simplify: %s\n", e.what());
        return 1;
    } catch (...) {
        if (s->verb > 0) fprintf(stderr, "[arjun-shim] stage_simplify: unknown failure\n");
        return 1;
    }
}

uint32_t arjun_shim_cur_nvars(ArjunShim *s) { return s->cur->nVars(); }

size_t arjun_shim_cur_nclauses(ArjunShim *s) { return s->cur->get_clauses().size(); }

size_t arjun_shim_cur_clauses(ArjunShim *s, int32_t *buf, size_t cap) {
    const auto &cls = s->cur->get_clauses();
    size_t need = 0;
    for (const auto &cl : cls) need += cl.size() + 1;  // +1 for the 0 terminator
    if (cap < need || buf == nullptr) return need;
    size_t k = 0;
    for (const auto &cl : cls) {
        for (const auto &l : cl) {
            int32_t lit = (int32_t)(l.var() + 1);
            buf[k++] = l.sign() ? -lit : lit;
        }
        buf[k++] = 0;
    }
    return need;
}

size_t arjun_shim_cur_sampl(ArjunShim *s, uint32_t *buf, size_t cap) {
    const auto &sv = s->cur->get_sampl_vars();
    if (cap < sv.size() || buf == nullptr) return sv.size();
    std::memcpy(buf, sv.data(), sv.size() * sizeof(uint32_t));
    return sv.size();
}

size_t arjun_shim_backbone(ArjunShim *s, int32_t *buf, size_t cap) {
    const auto &bb = s->backbone_dimacs;
    if (cap < bb.size() || buf == nullptr) return bb.size();
    std::memcpy(buf, bb.data(), bb.size() * sizeof(int32_t));
    return bb.size();
}

size_t arjun_shim_eq_lits(ArjunShim *s, int32_t *buf, size_t cap) {
    const auto &eq = s->eq_dimacs;  // flat pairs: [a0,b0, a1,b1, ...]
    if (cap < eq.size() || buf == nullptr) return eq.size();
    std::memcpy(buf, eq.data(), eq.size() * sizeof(int32_t));
    return eq.size();
}

// FFI-traffic caps for the redundant/learnt-clause harvest (single source of
// truth). A learnt clause longer than this is skipped (long learnts prune
// little and cost the most to ship/replay); the total count is bounded so a
// pathological red-clause DB can't flood the buffer. Both are sound to apply:
// dropping any implied clause only means less pruning, never a wrong answer.
static const size_t RED_CLAUSE_MAX_LEN = 8;       // literals per clause
static const size_t RED_CLAUSE_MAX_TOTAL = 50000; // clauses harvested

size_t arjun_shim_red_clauses(ArjunShim *s, int32_t *buf, size_t cap) {
    const auto &cls = s->cur->get_red_clauses();
    // First pass: size the (capped) selection. Encoding mirrors
    // arjun_shim_cur_clauses exactly — each kept clause contributes its lits
    // plus a 0 terminator.
    size_t need = 0, kept = 0;
    for (const auto &cl : cls) {
        if (kept >= RED_CLAUSE_MAX_TOTAL) break;
        if (cl.size() > RED_CLAUSE_MAX_LEN) continue;  // skip over-length
        need += cl.size() + 1;
        kept++;
    }
    if (cap < need || buf == nullptr) return need;
    size_t k = 0, kept2 = 0;
    for (const auto &cl : cls) {
        if (kept2 >= RED_CLAUSE_MAX_TOTAL) break;
        if (cl.size() > RED_CLAUSE_MAX_LEN) continue;
        for (const auto &l : cl) {
            int32_t lit = (int32_t)(l.var() + 1);  // SAME numbering as cur_clauses
            buf[k++] = l.sign() ? -lit : lit;
        }
        buf[k++] = 0;
        kept2++;
    }
    return need;
}

size_t arjun_shim_orig_to_new(ArjunShim *s, int32_t *buf, size_t cap) {
    // `get_orig_to_new_var()` is std::map<uint32_t /*orig var, 0-based*/,
    // CMSat::Lit /*current lit*/>, maintained by Arjun across every stage (see
    // the header for the coverage contract). Emit it as DIMACS flat pairs so the
    // Rust side needs no knowledge of CMSat::Lit's bit layout.
    const auto &m = s->cur->get_orig_to_new_var();
    size_t need = 0;
    for (const auto &kv : m) {
        if (kv.second == CMSat::lit_Undef) continue;  // no reduced counterpart
        need += 2;
    }
    if (cap < need || buf == nullptr) return need;
    size_t k = 0;
    for (const auto &kv : m) {
        if (kv.second == CMSat::lit_Undef) continue;
        buf[k++] = (int32_t)(kv.first + 1u);        // orig var, 1-based positive
        buf[k++] = lit_to_dimacs(kv.second);        // reduced lit, DIMACS-signed
    }
    return need;
}

size_t arjun_shim_cur_multiplier(ArjunShim *s, char *buf, size_t cap) {
    std::ostringstream os;
    s->cur->get_multiplier_weight()->display(os);
    std::string str = os.str();
    if (cap == 0 || buf == nullptr) return str.size();
    size_t n = str.size() < cap - 1 ? str.size() : cap - 1;
    std::memcpy(buf, str.data(), n);
    buf[n] = '\0';
    return str.size();
}

void arjun_shim_set_lit_weight(ArjunShim *s, int32_t lit, const char *weight_str) {
    uint32_t var = (uint32_t)(lit < 0 ? -lit : lit) - 1u;  // DIMACS 1-based -> 0-based
    Lit l(var, lit < 0);
    // Parse via the field's own parser (FMpq::parse: decimal, rational num/den,
    // integer, scientific) — the exact path the DIMACS `c p weight` reader uses.
    // Field::parse expects the DIMACS weight TOKEN `<weight> 0` (a trailing 0
    // line-terminator), so append " 0"; without it parse() still sets the value
    // correctly but prints a spurious "expected 0 at the end" diagnostic.
    std::unique_ptr<Field> w(s->fg->zero());
    std::string tok(weight_str);
    tok += " 0";
    w->parse(tok, 0);
    s->cur->set_lit_weight(l, w);
}

void arjun_shim_clean_sampl(ArjunShim *s) {
    s->cur->start_with_clean_sampl_vars();
}

size_t arjun_shim_lit_weight(ArjunShim *s, int32_t lit, char *buf, size_t cap) {
    uint32_t var = (uint32_t)(lit < 0 ? -lit : lit) - 1u;
    Lit l(var, lit < 0);
    std::unique_ptr<Field> w = s->cur->get_lit_weight(l);
    std::ostringstream os;
    w->display(os);
    std::string str = os.str();
    if (cap == 0 || buf == nullptr) return str.size();
    size_t n = str.size() < cap - 1 ? str.size() : cap - 1;
    std::memcpy(buf, str.data(), n);
    buf[n] = '\0';
    return str.size();
}

}  // extern "C"
