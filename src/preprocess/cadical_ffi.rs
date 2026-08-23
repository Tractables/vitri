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
    use std::ffi::{c_char, c_int, c_void};

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
    }
}

/// Result of `solve()` / `simplify()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Status {
    Unknown,
    Satisfiable,
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
pub(super) trait Terminator {
    fn terminated(&mut self) -> bool;
}

/// Receives each clause during `traverse_clauses`; return `false` to stop.
pub(super) trait ClauseIterator {
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
pub(super) struct CaDiCal {
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
    pub(super) fn new() -> Option<Self> {
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
    pub(super) fn add(&mut self, lit: i32) {
        unsafe { ffi::cadical_shim_add(self.handle, lit) }
    }

    pub(super) fn assume(&mut self, lit: i32) {
        unsafe { ffi::cadical_shim_assume(self.handle, lit) }
    }

    pub(super) fn constrain(&mut self, lit: i32) {
        unsafe { ffi::cadical_shim_constrain(self.handle, lit) }
    }

    pub(super) fn solve(&mut self) -> Status {
        Status::from_raw(unsafe { ffi::cadical_shim_solve(self.handle) })
    }

    pub(super) fn simplify(&mut self, rounds: i32) -> Status {
        Status::from_raw(unsafe { ffi::cadical_shim_simplify(self.handle, rounds) })
    }

    pub(super) fn val(&mut self, lit: i32) -> i32 {
        unsafe { ffi::cadical_shim_val(self.handle, lit) }
    }

    pub(super) fn fixed(&mut self, lit: i32) -> i32 {
        unsafe { ffi::cadical_shim_fixed(self.handle, lit) }
    }

    pub(super) fn flippable(&mut self, lit: i32) -> bool {
        unsafe { ffi::cadical_shim_flippable(self.handle, lit) }
    }

    /// Frozen variables are never BVE/BCE-eliminated, which is what preserves
    /// the model count across preprocessing.
    pub(super) fn freeze(&mut self, lit: i32) {
        unsafe { ffi::cadical_shim_freeze(self.handle, lit) }
    }

    pub(super) fn reserve(&mut self, min_max_var: i32) {
        unsafe { ffi::cadical_shim_reserve(self.handle, min_max_var) }
    }

    /// Set a named limit (`c"conflicts"`). Returns false if CaDiCaL does not
    /// know the name.
    ///
    /// The name is a `CStr` because every one this crate sets is a literal:
    /// taking the C form directly is one fewer allocation per call — the
    /// probe loop sets a limit before every probe — and there is no run-time
    /// name left to reject for an interior NUL.
    pub(super) fn limit(&mut self, name: &CStr, val: i32) -> bool {
        // SAFETY: live handle (§ Safety). `name` is NUL-terminated and outlives
        // the call; CaDiCaL compares it against its own table and keeps no
        // pointer to it.
        unsafe { ffi::cadical_shim_limit(self.handle, name.as_ptr(), val) }
    }

    /// Visit the irredundant clauses. Returns false if `it` stopped early.
    pub(super) fn traverse_clauses<I: ClauseIterator>(&mut self, it: &mut I) -> bool {
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

/// A solver with a terminator attached for as long as the guard lives.
///
/// The guard owns the terminator and disconnects it on drop, so the obligation
/// [`CaDiCal::connect_terminator`] states is discharged structurally: no path
/// out of the probing region — early return, `?`, or unwind — can leave CaDiCaL
/// holding a pointer to a terminator that is gone. Deref reaches the solver, so
/// the bounded region is written against the guard exactly as it would be
/// against the solver.
pub(super) struct Bounded<'s, T: Terminator> {
    solver: &'s mut CaDiCal,
    /// Boxed so the address CaDiCaL holds survives the guard itself being
    /// moved out of the constructor. Never read again — it exists to be kept
    /// alive, and to be freed after `Drop` has disconnected it.
    _term: Box<T>,
}

impl<'s, T: Terminator> Bounded<'s, T> {
    pub(super) fn new(solver: &'s mut CaDiCal, term: T) -> Self {
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
