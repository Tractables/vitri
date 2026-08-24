// C ABI over CaDiCaL's C++ `Solver`, for the Rust preprocessing path.
//
// CaDiCaL ships a C API (`ccadical.h`), but it is missing five entry points we
// need -- `simplify(rounds)`, `flippable`, `reserve`, `traverse_clauses` and
// `disconnect_terminator` -- and its opaque `CCaDiCaL *` is a wrapper struct
// rather than a `Solver *`, so reaching past it would couple us to that
// wrapper's layout. This shim goes straight at the C++ class instead, exposing
// exactly the surface `vitri::preprocess` calls and nothing else.
//
// It deliberately mirrors `arjun_shim.h`: one opaque handle, plain C types, and
// callbacks passed as (function pointer, state) pairs so Rust can hand over a
// `&mut T` without CaDiCaL knowing anything about Rust types.

#ifndef VITRI_CADICAL_SHIM_H
#define VITRI_CADICAL_SHIM_H

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct cadical_shim_solver cadical_shim_solver;

// Return codes from `solve` / `simplify`, matching CaDiCaL's own convention.
#define CADICAL_SHIM_UNKNOWN 0
#define CADICAL_SHIM_SATISFIABLE 10
#define CADICAL_SHIM_UNSATISFIABLE 20

// Returns true to ask CaDiCaL to stop.
typedef bool (*cadical_shim_terminate_cb) (void *state);

// Receives one clause as a pointer/length pair. Returns false to stop
// traversal early.
typedef bool (*cadical_shim_clause_cb) (void *state, const int *lits, int len);

cadical_shim_solver *cadical_shim_new (void);
void cadical_shim_delete (cadical_shim_solver *);

// Clause construction: pass literals then 0 to close the clause.
void cadical_shim_add (cadical_shim_solver *, int lit);

void cadical_shim_assume (cadical_shim_solver *, int lit);
void cadical_shim_constrain (cadical_shim_solver *, int lit);

int cadical_shim_solve (cadical_shim_solver *);
int cadical_shim_simplify (cadical_shim_solver *, int rounds);

int cadical_shim_val (cadical_shim_solver *, int lit);
int cadical_shim_fixed (cadical_shim_solver *, int lit);
bool cadical_shim_flippable (cadical_shim_solver *, int lit);

// Frozen variables are never eliminated by BVE/BCE. That is what makes this
// preprocessing model-count preserving, so the Rust side freezes every
// occurring variable before simplifying.
void cadical_shim_freeze (cadical_shim_solver *, int lit);

// Forces the initial decision phase of a variable.
void cadical_shim_phase (cadical_shim_solver *, int lit);

void cadical_shim_reserve (cadical_shim_solver *, int min_max_var);
bool cadical_shim_limit (cadical_shim_solver *, const char *name, int val);

// Visits the irredundant clauses. Returns false if the callback stopped it.
bool cadical_shim_traverse_clauses (cadical_shim_solver *,
                                    cadical_shim_clause_cb, void *state);

// `state` must stay valid until disconnected or the solver is deleted.
void cadical_shim_connect_terminator (cadical_shim_solver *,
                                      cadical_shim_terminate_cb, void *state);
void cadical_shim_disconnect_terminator (cadical_shim_solver *);

// Clause-database sizes, straight off CaDiCaL's public class.
long long cadical_shim_redundant (cadical_shim_solver *);
long long cadical_shim_irredundant (cadical_shim_solver *);

// The two accessors that reach past the public class, defined in
// `cadical_internal_stats.cpp`.

// Current VSIDS activity of `lit`'s variable.
double cadical_shim_score_of (cadical_shim_solver *, int lit);

// Fills `n` CDCL search counters, in the slot order
// `cadical_internal_stats.cpp` documents, zeroing any slot it does not have.
void cadical_shim_search_stats (cadical_shim_solver *, long long *out,
                                unsigned long n);

#ifdef __cplusplus
}
#endif

#endif // VITRI_CADICAL_SHIM_H
