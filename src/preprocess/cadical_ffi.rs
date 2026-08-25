//! Safe wrapper over the CaDiCaL C shim (`vendor/arjun/cadical_shim.cpp`).
//!
//! There is exactly one CaDiCaL in the process: the copy the vendored Arjun
//! stack builds. Arjun, CryptoMiniSat and CadiBack all require meelgroup's fork
//! (it adds `get_eqiv_lits` / `traverse_red_clauses` / `conflicts`), and that
//! fork is upstream 2.1.3 plus those additions -- every option default is
//! identical and no pre-existing function body changed -- so our preprocessing
//! runs on it too rather than linking a second, stock copy.
//!
//! Keeping it to one copy is what lets the whole stack link statically: two
//! same-version CaDiCaLs would share COMDAT groups (vtables, libstdc++ template
//! instantiations) that a static merge cannot safely separate.
//!
//! # Safety
//!
//! Two facts are shared by the `unsafe` blocks below; the per-site comments add
//! only what is specific to a call.
//!
//! * **The handle.** `CaDiCal::handle` is what the shim's constructor returned,
//!   null-checked at the one place it is called, so every later call has a live
//!   solver. It is released exactly once, in `Drop`; the struct is neither
//!   `Copy` nor `Clone`, so no second owner can release it or call through it
//!   afterwards, and the raw pointer field keeps the type off `Send`/`Sync`.
//! * **Callback state.** The two trampolines receive a pointer that was a
//!   `&mut T` on the Rust side, cast back to the type the trampoline was
//!   instantiated for. For the clause iterator the borrow spans exactly the
//!   traversal call, so the compiler enforces the lifetime; for the terminator
//!   it does not, which is why `connect_terminator` is an `unsafe fn` that
//!   states the obligation instead. Neither trampoline can unwind into C++: a
//!   panic in a Rust `extern "C"` function aborts at the boundary.

use std::ffi::{CStr, c_int, c_void};

use crate::diagnostics::diag;

mod ffi {
    use std::ffi::{c_char, c_double, c_int, c_longlong, c_ulong, c_void};

    /// The solver the shim hands back, opaque exactly as its header declares
    /// it: an incomplete type nothing outside the shim can build, read or
    /// measure.
    ///
    /// Zero-sized rather than an alias for `c_void`, so that a pointer to one
    /// is its OWN type: under an alias a solver handle and the `*mut c_void`
    /// of a callback state are the same type, and the signatures below take
    /// one of each.
    #[repr(C)]
    pub(super) struct Solver {
        _private: [u8; 0],
    }

    pub(super) type TerminateCb = extern "C" fn(state: *mut c_void) -> bool;
    pub(super) type ClauseCb =
        extern "C" fn(state: *mut c_void, lits: *const c_int, len: c_int) -> bool;

    // SAFETY: hand-written mirror of the C declarations in
    // `vendor/arjun/cadical_shim.h`, which is vendored here and compiled by
    // `build.rs` from that same header — the two sides move together. Each
    // signature spells the header's own types; the plain-Rust surface is the
    // safe wrapper above.
    unsafe extern "C" {
        pub(super) fn cadical_shim_new() -> *mut Solver;
        pub(super) fn cadical_shim_delete(s: *mut Solver);
        pub(super) fn cadical_shim_add(s: *mut Solver, lit: c_int);
        pub(super) fn cadical_shim_assume(s: *mut Solver, lit: c_int);
        pub(super) fn cadical_shim_constrain(s: *mut Solver, lit: c_int);
        pub(super) fn cadical_shim_solve(s: *mut Solver) -> c_int;
        pub(super) fn cadical_shim_simplify(s: *mut Solver, rounds: c_int) -> c_int;
        pub(super) fn cadical_shim_val(s: *mut Solver, lit: c_int) -> c_int;
        pub(super) fn cadical_shim_fixed(s: *mut Solver, lit: c_int) -> c_int;
        pub(super) fn cadical_shim_flippable(s: *mut Solver, lit: c_int) -> bool;
        pub(super) fn cadical_shim_freeze(s: *mut Solver, lit: c_int);
        pub(super) fn cadical_shim_reserve(s: *mut Solver, min_max_var: c_int);
        pub(super) fn cadical_shim_limit(s: *mut Solver, name: *const c_char, val: c_int) -> bool;
        pub(super) fn cadical_shim_traverse_clauses(
            s: *mut Solver,
            cb: ClauseCb,
            state: *mut c_void,
        ) -> bool;
        pub(super) fn cadical_shim_connect_terminator(
            s: *mut Solver,
            cb: TerminateCb,
            state: *mut c_void,
        );
        pub(super) fn cadical_shim_disconnect_terminator(s: *mut Solver);
        pub(super) fn cadical_shim_redundant(s: *mut Solver) -> c_longlong;
        pub(super) fn cadical_shim_irredundant(s: *mut Solver) -> c_longlong;
        pub(super) fn cadical_shim_score_of(s: *mut Solver, lit: c_int) -> c_double;
        pub(super) fn cadical_shim_search_stats(s: *mut Solver, out: *mut c_longlong, n: c_ulong);
    }
}

/// Result of [`CaDiCal::solve`] / [`CaDiCal::simplify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The call stopped before deciding — a limit, or a terminator that fired.
    Unknown,
    /// A model exists; [`CaDiCal::val`] reads it.
    Satisfiable,
    /// No model exists under the assumptions the call was made with.
    Unsatisfiable,
}

impl Status {
    fn from_raw(v: c_int) -> Self {
        match v {
            10 => Status::Satisfiable,
            20 => Status::Unsatisfiable,
            _ => Status::Unknown,
        }
    }
}

/// Asked between rounds/phases; return `true` to stop the solver.
///
/// Attach one with [`Bounded`], which is what keeps the solver from holding a
/// pointer to a terminator that has gone away.
pub trait Terminator {
    /// Whether the solver should stop now.
    fn terminated(&mut self) -> bool;
}

/// Receives each clause during [`CaDiCal::traverse_clauses`]; return `false`
/// to stop the traversal.
pub trait ClauseIterator {
    /// One clause, as literals in DIMACS numbering. Return `false` to stop.
    fn clause(&mut self, clause: &[i32]) -> bool;
}

extern "C" fn terminate_trampoline<T: Terminator>(state: *mut c_void) -> bool {
    // SAFETY: callback state (§ Safety). `connect_terminator`'s obligation is
    // what keeps `state` valid, and this runs only from inside a solver call,
    // so the caller cannot be touching `T` at the same time.
    unsafe { (*(state as *mut T)).terminated() }
}

extern "C" fn clause_trampoline<I: ClauseIterator>(
    state: *mut c_void,
    lits: *const c_int,
    len: c_int,
) -> bool {
    // The EMPTY clause is a real thing to traverse — CaDiCaL holds one for a
    // formula it has proved unsatisfiable — and `std::vector::data()` is
    // permitted to return null when the vector is empty. `from_raw_parts`
    // requires a non-null, aligned pointer even for a zero-length slice, so the
    // empty case must never reach it: that is UB, and the debug-build
    // precondition check turns it into a non-unwinding abort.
    let slice: &[c_int] = if len <= 0 || lits.is_null() {
        &[]
    } else {
        // SAFETY: the shim passes the clause vector's own buffer, valid for the
        // duration of this call and holding `len` initialized `int`s.
        unsafe { std::slice::from_raw_parts(lits, len as usize) }
    };
    // SAFETY: callback state (§ Safety) — the `&mut I` that `traverse_clauses`
    // borrows for the whole traversal, so nothing else can reach `I` while
    // this runs.
    unsafe { (*(state as *mut I)).clause(slice) }
}

/// An owned CaDiCaL solver.
///
/// The incremental interface — [`add`](Self::add), [`assume`](Self::assume),
/// [`solve`](Self::solve), [`val`](Self::val) — plus the inprocessing and
/// introspection hooks vitri's own preprocessing needed. It is not a general
/// solver abstraction and does not try to be.
///
/// Neither `Send` nor `Sync`: the handle is a raw pointer into a C++ object
/// with no internal synchronisation.
pub struct CaDiCal {
    handle: *mut ffi::Solver,
}

/// Report that [`CaDiCal::new`] came back empty, and what `stage` gives up as a
/// result. The reason is this constructor's to tell, not each caller's: a
/// caller that reconstructs it from what it knows today would keep saying
/// "allocation failed" after a second failure cause appeared here.
pub(super) fn note_solver_unavailable(stage: &str, consequence: &str) {
    diag!("[{stage}] no CaDiCaL solver — {consequence}");
}

impl CaDiCal {
    /// A fresh solver, or `None` when the shim could not allocate one.
    ///
    /// This is a library path, so an allocation that failed comes back as a
    /// value the caller decides about — every caller here has a "this stage
    /// found nothing" answer — rather than ending the calling program.
    pub fn new() -> Option<Self> {
        // SAFETY: no arguments and no precondition. The shim allocates with
        // `new (std::nothrow)`, so a failure arrives as null rather than as an
        // exception, and the check below is what stops a null reaching any of
        // the calls that dereference it.
        let handle = unsafe { ffi::cadical_shim_new() };
        if handle.is_null() {
            None
        } else {
            Some(Self { handle })
        }
    }

    /// Add a literal to the current clause; `0` closes it.
    pub fn add(&mut self, lit: i32) {
        unsafe { ffi::cadical_shim_add(self.handle, lit) }
    }

    /// Assume `lit` true for the next [`solve`](Self::solve) only.
    pub fn assume(&mut self, lit: i32) {
        unsafe { ffi::cadical_shim_assume(self.handle, lit) }
    }

    /// Add a literal to the constraint clause for the next
    /// [`solve`](Self::solve); `0` closes it. Unlike an assumption the
    /// constraint is a clause, so it holds if any of its literals does.
    pub fn constrain(&mut self, lit: i32) {
        unsafe { ffi::cadical_shim_constrain(self.handle, lit) }
    }

    /// Solve under the current assumptions and constraint.
    ///
    /// A search stopped by an attached terminator, or by a
    /// [`limit`](Self::limit) it reached, answers [`Status::Unknown`]; the
    /// terminator is honoured strictly.
    pub fn solve(&mut self) -> Status {
        Status::from_raw(unsafe { ffi::cadical_shim_solve(self.handle) })
    }

    /// Run `rounds` of inprocessing without searching for a model. Frozen
    /// variables survive it; see [`freeze`](Self::freeze).
    ///
    /// An attached terminator is polled between rounds and phases rather than
    /// continuously, so a budget bounds this closely enough to stop a runaway
    /// pass but not to the millisecond.
    pub fn simplify(&mut self, rounds: i32) -> Status {
        Status::from_raw(unsafe { ffi::cadical_shim_simplify(self.handle, rounds) })
    }

    /// The value of `lit` in the model of the last satisfiable solve:
    /// positive for true, negative for false.
    pub fn val(&mut self, lit: i32) -> i32 {
        unsafe { ffi::cadical_shim_val(self.handle, lit) }
    }

    /// Whether `lit` is fixed at the root: `1` true, `-1` false, `0` neither.
    pub fn fixed(&mut self, lit: i32) -> i32 {
        unsafe { ffi::cadical_shim_fixed(self.handle, lit) }
    }

    /// Whether `lit`'s value in the current model can be flipped and still
    /// satisfy every clause.
    pub fn flippable(&mut self, lit: i32) -> bool {
        unsafe { ffi::cadical_shim_flippable(self.handle, lit) }
    }

    /// Frozen variables are never BVE/BCE-eliminated, which is what preserves
    /// the model count across preprocessing. A caller that will ask about a
    /// variable after [`simplify`](Self::simplify) freezes it first.
    pub fn freeze(&mut self, lit: i32) {
        unsafe { ffi::cadical_shim_freeze(self.handle, lit) }
    }

    /// Pre-size the solver for `min_max_var` variables, so adding them does
    /// not reallocate repeatedly.
    pub fn reserve(&mut self, min_max_var: i32) {
        unsafe { ffi::cadical_shim_reserve(self.handle, min_max_var) }
    }

    /// Set a named limit (`c"conflicts"`). Returns false if CaDiCaL does not
    /// know the name.
    ///
    /// The name is a `CStr` because every one this crate sets is a literal:
    /// taking the C form directly is one fewer allocation per call — the
    /// probe loop sets a limit before every probe — and there is no run-time
    /// name left to reject for an interior NUL.
    pub fn limit(&mut self, name: &CStr, val: i32) -> bool {
        // SAFETY: live handle (§ Safety). `name` is NUL-terminated and outlives
        // the call; CaDiCaL compares it against its own table and keeps no
        // pointer to it.
        unsafe { ffi::cadical_shim_limit(self.handle, name.as_ptr(), val) }
    }

    /// Visit the irredundant clauses. Returns false if `it` stopped early.
    pub fn traverse_clauses<I: ClauseIterator>(&mut self, it: &mut I) -> bool {
        // SAFETY: live handle (§ Safety). The state pointer is the `&mut I` this
        // call borrows for its whole duration, and the trampoline is instantiated
        // for that same `I`, so the cast inside it restores the original type.
        // The shim builds its iterator adapter on the stack for the call only, so
        // nothing outlives the borrow.
        unsafe {
            ffi::cadical_shim_traverse_clauses(
                self.handle,
                clause_trampoline::<I>,
                it as *mut I as *mut c_void,
            )
        }
    }

    /// Number of redundant (learnt) clauses currently in the database.
    pub fn redundant(&self) -> i64 {
        // SAFETY: live handle (§ Safety). A read-only query on the public class.
        unsafe { ffi::cadical_shim_redundant(self.handle) }
    }

    /// Number of irredundant (original) clauses currently in the database.
    pub fn irredundant(&self) -> i64 {
        // SAFETY: live handle (§ Safety). A read-only query on the public class.
        unsafe { ffi::cadical_shim_irredundant(self.handle) }
    }

    /// The solver's current activity score for `lit`'s variable — a property of
    /// the variable, so the two literals over it read the same.
    ///
    /// Meaningful only after a search that accumulated some: on an untouched
    /// solver every variable reads the same initial value. So does one whose
    /// search was short, because CaDiCaL keeps variable activity in one of two
    /// schemes and alternates between them, and this reads only one of the two.
    /// Read the result as a signal about which variables *this* search found
    /// contentious — it is not a stable property of the formula, and two runs
    /// that take different search paths will disagree.
    pub fn score_of(&self, lit: i32) -> f64 {
        // SAFETY: live handle (§ Safety). The accessor reads one score off the
        // solver's internal state and keeps nothing.
        unsafe { ffi::cadical_shim_score_of(self.handle, lit) }
    }

    /// CDCL search counters, read off the solver's internal statistics.
    ///
    /// Cumulative over this handle's lifetime — an incremental solver keeps
    /// accumulating across [`solve`](Self::solve) calls — so a caller measuring
    /// one interval takes two snapshots and differences them with
    /// [`SearchStats::since`].
    pub fn search_stats(&self) -> SearchStats {
        let mut slots = [0 as std::ffi::c_longlong; SearchStats::SLOTS];
        // SAFETY: live handle (§ Safety). `slots` is a live array of exactly
        // the length passed, and the accessor fills it and zeroes what it does
        // not have — see the slot-order contract in
        // `vendor/arjun/cadical_internal_stats.cpp`.
        unsafe {
            ffi::cadical_shim_search_stats(
                self.handle,
                slots.as_mut_ptr(),
                SearchStats::SLOTS as std::ffi::c_ulong,
            )
        };
        SearchStats::from_slots(slots)
    }

    /// Hand CaDiCaL a callback that asks `t` whether to stop.
    ///
    /// Private: [`Bounded`] is the only caller, and its `Drop` is what
    /// discharges the obligation below.
    ///
    /// # Safety
    ///
    /// This stores a raw pointer to `t` inside the solver and does not borrow
    /// it for the lifetime of that pointer. The caller must keep `t` alive, in
    /// place, and unaliased until [`Self::disconnect_terminator`]. Letting `t`
    /// move or fall out of scope while still connected leaves CaDiCaL calling
    /// through a dangling pointer — and dropping the solver is not enough,
    /// since locals drop in reverse declaration order and a terminator
    /// declared after its solver goes first.
    unsafe fn connect_terminator<T: Terminator>(&mut self, t: &mut T) {
        // SAFETY: the caller's obligation above is what keeps the state pointer
        // valid for as long as the solver holds it; the trampoline is
        // instantiated for the same `T` the pointer came from, so its cast
        // restores the original type.
        unsafe {
            ffi::cadical_shim_connect_terminator(
                self.handle,
                terminate_trampoline::<T>,
                t as *mut T as *mut c_void,
            )
        }
    }

    fn disconnect_terminator(&mut self) {
        // SAFETY: live handle (§ Safety). Disconnecting is what ends the solver's
        // hold on the terminator state, and it is sound whether or not one is
        // connected — the shim clears an already-empty slot.
        unsafe { ffi::cadical_shim_disconnect_terminator(self.handle) }
    }
}

/// A snapshot of the CDCL search counters. See [`CaDiCal::search_stats`].
///
/// Every field is cumulative over the solver's lifetime;
/// [`since`](SearchStats::since) turns two snapshots into the work done between
/// them.
///
/// `#[non_exhaustive]`: the accessor's slot order is append-only, so a counter
/// added later must not be a breaking change for a caller that constructs one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchStats {
    /// Conflicts encountered.
    pub conflicts: i64,
    /// Decisions taken.
    pub decisions: i64,
    /// Propagations performed during search, not during inprocessing.
    pub propagations: i64,
    /// Restarts performed.
    pub restarts: i64,
    /// Clauses learnt.
    pub learned_clauses: i64,
    /// Variables the decision loop scanned looking for an unassigned one.
    pub searched: i64,
}

impl SearchStats {
    /// How many slots the accessor is asked for, derived from the fields below
    /// rather than written twice: the C side fills what the caller asks for and
    /// zeroes the rest, so this number and the field count must be the same or
    /// a field silently reads zero.
    pub(crate) const SLOTS: usize = 6;

    /// Read the accessor's slots in the order
    /// `vendor/arjun/cadical_internal_stats.cpp` documents. The one place that
    /// order is spelled on this side.
    pub(crate) fn from_slots(v: [std::ffi::c_longlong; Self::SLOTS]) -> Self {
        SearchStats {
            conflicts: v[0],
            decisions: v[1],
            propagations: v[2],
            restarts: v[3],
            learned_clauses: v[4],
            searched: v[5],
        }
    }

    /// The work done since `earlier`, field by field.
    ///
    /// Saturates at zero rather than wrapping, so differencing two snapshots in
    /// the wrong order reports no work rather than a huge one.
    pub fn since(self, earlier: Self) -> Self {
        SearchStats {
            conflicts: self.conflicts.saturating_sub(earlier.conflicts).max(0),
            decisions: self.decisions.saturating_sub(earlier.decisions).max(0),
            propagations: self
                .propagations
                .saturating_sub(earlier.propagations)
                .max(0),
            restarts: self.restarts.saturating_sub(earlier.restarts).max(0),
            learned_clauses: self
                .learned_clauses
                .saturating_sub(earlier.learned_clauses)
                .max(0),
            searched: self.searched.saturating_sub(earlier.searched).max(0),
        }
    }
}

/// A solver with a terminator attached for as long as the guard lives.
///
/// The guard owns the terminator and disconnects it on drop, so the obligation
/// that attaching one carries is discharged structurally: no path out of the
/// bounded region — early return, `?`, or unwind — can leave CaDiCaL holding a
/// pointer to a terminator that is gone. It is the only way to attach one.
/// Deref reaches the solver, so the bounded region is written against the guard
/// exactly as it would be against the solver.
pub struct Bounded<'s, T: Terminator> {
    solver: &'s mut CaDiCal,
    /// Boxed so the address CaDiCaL holds survives the guard itself being
    /// moved out of the constructor. Never read again — it exists to be kept
    /// alive, and to be freed after `Drop` has disconnected it.
    _term: Box<T>,
}

impl<'s, T: Terminator> Bounded<'s, T> {
    /// Connect `term` to `solver` for as long as the guard lives.
    pub fn new(solver: &'s mut CaDiCal, term: T) -> Self {
        let mut term = Box::new(term);
        // SAFETY: the box keeps `term` at one address for the whole life of the
        // guard, the guard is its only owner (nothing else can read it), and
        // `Drop` disconnects before the box is freed — field drops run after
        // `drop`, so the pointer is out of the solver first.
        unsafe { solver.connect_terminator(&mut *term) };
        Bounded {
            solver,
            _term: term,
        }
    }
}

impl<T: Terminator> Drop for Bounded<'_, T> {
    fn drop(&mut self) {
        self.solver.disconnect_terminator();
    }
}

impl<T: Terminator> std::ops::Deref for Bounded<'_, T> {
    type Target = CaDiCal;
    fn deref(&self) -> &CaDiCal {
        &*self.solver
    }
}

impl<T: Terminator> std::ops::DerefMut for Bounded<'_, T> {
    fn deref_mut(&mut self) -> &mut CaDiCal {
        &mut *self.solver
    }
}

impl Drop for CaDiCal {
    fn drop(&mut self) {
        // SAFETY: the handle (§ Safety); this is the one release it describes.
        unsafe { ffi::cadical_shim_delete(self.handle) }
    }
}
