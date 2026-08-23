#include "cadical_shim.h"

#include <cadical.hpp>

#include <new>
#include <vector>

namespace {

// Adapts CaDiCaL's abstract Terminator to a C callback. CaDiCaL polls this
// between rounds/phases, so a deadline expressed here bounds a runaway pass
// without needing to interrupt the solver from another thread.
struct FnTerminator final : public CaDiCaL::Terminator {
  cadical_shim_terminate_cb cb;
  void *state;
  FnTerminator (cadical_shim_terminate_cb c, void *s) : cb (c), state (s) {}
  bool terminate () override { return cb (state); }
};

// Adapts CaDiCaL's abstract ClauseIterator to a C callback. The vector is
// handed over as a pointer/length pair and is only valid for the call.
struct FnClauseIterator final : public CaDiCaL::ClauseIterator {
  cadical_shim_clause_cb cb;
  void *state;
  FnClauseIterator (cadical_shim_clause_cb c, void *s) : cb (c), state (s) {}
  bool clause (const std::vector<int> &c) override {
    return cb (state, c.data (), static_cast<int> (c.size ()));
  }
};

// The handle owns the terminator adapter so the Rust side only has to keep its
// own callback state alive, not a second C++ object.
struct Wrapper {
  CaDiCaL::Solver solver;
  FnTerminator *terminator = nullptr;
  ~Wrapper () { delete terminator; }
};

inline Wrapper *unwrap (cadical_shim_solver *s) {
  return reinterpret_cast<Wrapper *> (s);
}

} // namespace

extern "C" {

cadical_shim_solver *cadical_shim_new (void) {
  return reinterpret_cast<cadical_shim_solver *> (new (std::nothrow) Wrapper ());
}

void cadical_shim_delete (cadical_shim_solver *s) { delete unwrap (s); }

void cadical_shim_add (cadical_shim_solver *s, int lit) {
  unwrap (s)->solver.add (lit);
}

void cadical_shim_assume (cadical_shim_solver *s, int lit) {
  unwrap (s)->solver.assume (lit);
}

void cadical_shim_constrain (cadical_shim_solver *s, int lit) {
  unwrap (s)->solver.constrain (lit);
}

int cadical_shim_solve (cadical_shim_solver *s) {
  return unwrap (s)->solver.solve ();
}

int cadical_shim_simplify (cadical_shim_solver *s, int rounds) {
  return unwrap (s)->solver.simplify (rounds);
}

int cadical_shim_val (cadical_shim_solver *s, int lit) {
  return unwrap (s)->solver.val (lit);
}

int cadical_shim_fixed (cadical_shim_solver *s, int lit) {
  return unwrap (s)->solver.fixed (lit);
}

bool cadical_shim_flippable (cadical_shim_solver *s, int lit) {
  return unwrap (s)->solver.flippable (lit);
}

void cadical_shim_freeze (cadical_shim_solver *s, int lit) {
  unwrap (s)->solver.freeze (lit);
}

void cadical_shim_phase (cadical_shim_solver *s, int lit) {
  unwrap (s)->solver.phase (lit);
}

void cadical_shim_reserve (cadical_shim_solver *s, int min_max_var) {
  unwrap (s)->solver.reserve (min_max_var);
}

bool cadical_shim_limit (cadical_shim_solver *s, const char *name, int val) {
  return unwrap (s)->solver.limit (name, val);
}

bool cadical_shim_traverse_clauses (cadical_shim_solver *s,
                                    cadical_shim_clause_cb cb, void *state) {
  FnClauseIterator it (cb, state);
  return unwrap (s)->solver.traverse_clauses (it);
}

void cadical_shim_connect_terminator (cadical_shim_solver *s,
                                      cadical_shim_terminate_cb cb,
                                      void *state) {
  Wrapper *w = unwrap (s);
  // Replacing an existing terminator must not leak the old adapter.
  w->solver.disconnect_terminator ();
  delete w->terminator;
  w->terminator = new FnTerminator (cb, state);
  w->solver.connect_terminator (w->terminator);
}

void cadical_shim_disconnect_terminator (cadical_shim_solver *s) {
  Wrapper *w = unwrap (s);
  w->solver.disconnect_terminator ();
  delete w->terminator;
  w->terminator = nullptr;
}

} // extern "C"
