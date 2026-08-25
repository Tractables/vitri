//! The C boundary to the Arjun shim: the `extern "C"` declarations mirroring
//! `vendor/arjun/arjun_shim.h`, and [`ArjunLib`], the safe owner that is the
//! only way the rest of this crate reaches them.
//!
//! # Safety
//!
//! Every `unsafe` call below goes into the shim, so they share one argument;
//! the per-site comments add only what is specific to a call.
//!
//! * **The handle.** `ArjunLib::raw` is what a shim constructor returned, and
//!   the two constructors are the only way to build the struct — each rejects a
//!   null before wrapping it, so every later call has a live handle. It is
//!   released exactly once, in `Drop`; the struct is neither `Copy` nor `Clone`,
//!   so no second owner can free it or call through it afterwards, and its raw
//!   pointer field keeps the type off `Send`/`Sync`, so the handle stays on the
//!   thread that built it.
//! * **Buffers passed in** are described by a pointer and a length taken from
//!   the same live slice in the same expression. The shim copies their contents
//!   into its own containers during the call and keeps no pointer to them, so
//!   nothing has to outlive the call.
//! * **Buffers read back** follow the shim's probe-then-fill protocol: called
//!   with a null buffer and capacity 0 a getter writes nothing and returns the
//!   length it needs; called again with a buffer of at least that length it
//!   fills it. Nothing can invalidate that length in between, because the
//!   getters take `&self` while every call that advances the checkpoint takes
//!   `&mut self`. A capacity too small for the answer is refused rather than
//!   overrun: the list getters write nothing at all, and the two string getters
//!   truncate and NUL-terminate inside the capacity they were handed — which is
//!   why their buffer is sized `need + 1` rather than `need`.
//! * **Exceptions.** The entry points that can fail — both constructors and
//!   both stages — catch every C++ exception at the boundary and report the
//!   failure through their return value, so none crosses back into Rust. The
//!   others do bounded work on already-built state, where the one throwing
//!   failure left is allocation failure. An exception there would cross an
//!   `extern "C"` boundary that does not permit unwinding; in practice it takes
//!   the process down, and since a reduction runs in a forked child by default,
//!   the parent reports that as a failed reduction and falls back to the
//!   unreduced formula.

use crate::cnf::{Clause, CnfFormula, Literal, Reduced};
use crate::error::VitriError;
use crate::preprocess::VarMap;
use std::os::raw::{c_char, c_int};
use std::time::Instant;

#[allow(non_camel_case_types)]
mod ffi {
    use std::os::raw::{c_char, c_int};

    #[repr(C)]
    pub(super) struct ArjunShim {
        _private: [u8; 0],
    }

    // SAFETY: hand-written mirror of the C declarations in
    // `vendor/arjun/arjun_shim.h`, which is vendored here and compiled by
    // `build.rs` from that same header — the two sides move together. Each
    // signature spells the header's own types: `c_int` for its `int`, the
    // exact-width Rust integer for its `<stdint.h>` types; the plain-Rust
    // surface is the safe wrapper below.
    unsafe extern "C" {
        pub(super) fn arjun_shim_new(seed: u32) -> *mut ArjunShim;
        pub(super) fn arjun_shim_new_weighted(seed: u32) -> *mut ArjunShim;
        pub(super) fn arjun_shim_free(s: *mut ArjunShim);
        pub(super) fn arjun_shim_set_lit_weight(
            s: *mut ArjunShim,
            lit: i32,
            weight_str: *const c_char,
        );
        pub(super) fn arjun_shim_clean_sampl(s: *mut ArjunShim);
        pub(super) fn arjun_shim_lit_weight(
            s: *mut ArjunShim,
            lit: i32,
            buf: *mut c_char,
            cap: usize,
        ) -> usize;
        pub(super) fn arjun_shim_new_vars(s: *mut ArjunShim, n: u32);
        pub(super) fn arjun_shim_add_clause(s: *mut ArjunShim, lits: *const i32, n: usize);
        pub(super) fn arjun_shim_set_sampl(s: *mut ArjunShim, vars0: *const u32, n: usize);
        pub(super) fn arjun_shim_set_backbone_max_confl(s: *mut ArjunShim, max_confl: i64);
        pub(super) fn arjun_shim_set_oracle_mult(s: *mut ArjunShim, mult: f64);
        pub(super) fn arjun_shim_set_deadline_ms(s: *mut ArjunShim, ms_from_now: i64);
        pub(super) fn arjun_shim_stage_minimize_indep(s: *mut ArjunShim, all_indep: c_int)
        -> c_int;
        pub(super) fn arjun_shim_stage_simplify(
            s: *mut ArjunShim,
            all_indep: c_int,
            oracle_enabled: c_int,
            no_sbva: c_int,
            no_bve: c_int,
        ) -> c_int;
        pub(super) fn arjun_shim_cur_nvars(s: *mut ArjunShim) -> u32;
        pub(super) fn arjun_shim_cur_clauses(s: *mut ArjunShim, buf: *mut i32, cap: usize)
        -> usize;
        pub(super) fn arjun_shim_cur_sampl(s: *mut ArjunShim, buf: *mut u32, cap: usize) -> usize;
        pub(super) fn arjun_shim_backbone(s: *mut ArjunShim, buf: *mut i32, cap: usize) -> usize;
        pub(super) fn arjun_shim_eq_lits(s: *mut ArjunShim, buf: *mut i32, cap: usize) -> usize;
        pub(super) fn arjun_shim_red_clauses(s: *mut ArjunShim, buf: *mut i32, cap: usize)
        -> usize;
        pub(super) fn arjun_shim_orig_to_new(s: *mut ArjunShim, buf: *mut i32, cap: usize)
        -> usize;
        pub(super) fn arjun_shim_cur_multiplier(
            s: *mut ArjunShim,
            buf: *mut c_char,
            cap: usize,
        ) -> usize;
    }
}

/// Safe owner of an in-process Arjun simplification state. The internal
/// `SimplifiedCNF` always holds the most-reduced-so-far checkpoint; every getter
/// reads that checkpoint, so reading after any stage (or after a stage that
/// failed) is sound.
pub(in crate::preprocess) struct ArjunLib {
    raw: *mut ffi::ArjunShim,
    /// Set by [`Self::set_deadline`]; read by [`Self::deadline_armed`].
    deadline_armed: bool,
}

impl ArjunLib {
    /// Take ownership of a freshly constructed shim, `None` for the null the
    /// constructors return when the allocation failed.
    fn from_raw(raw: *mut ffi::ArjunShim) -> Option<Self> {
        (!raw.is_null()).then_some(ArjunLib {
            raw,
            deadline_armed: false,
        })
    }

    /// Construct an unweighted (integer count) Arjun shim, seeding Arjun's own
    /// randomization with `seed`. `None` if the underlying allocation fails.
    pub(in crate::preprocess) fn new(seed: u32) -> Option<Self> {
        // SAFETY: a plain value argument with no precondition; a construction
        // the shim could not complete comes back as null, which `from_raw`
        // checks before wrapping it.
        Self::from_raw(unsafe { ffi::arjun_shim_new(seed) })
    }

    /// Weighted (rational, FGenMpq) field — the WMC path, equivalent to
    /// upstream Arjun's `--mode 1`. The travelling multiplier is a rational; literal
    /// weights are ingested via [`Self::set_lit_weight`].
    pub(in crate::preprocess) fn new_weighted(seed: u32) -> Option<Self> {
        // SAFETY: as `new` above — no precondition, null on failure.
        Self::from_raw(unsafe { ffi::arjun_shim_new_weighted(seed) })
    }

    /// Set a literal's weight (decimal or rational `num/den` string). Only valid
    /// on a weighted shim. Set BOTH polarities of a var with their explicit
    /// weights to reproduce upstream Arjun's per-literal ingest exactly.
    ///
    /// # Errors
    ///
    /// `weight_str` carries an interior NUL, so no C string spells it. Feeding
    /// the shim an empty weight instead would silently change the count.
    pub(in crate::preprocess) fn set_lit_weight(
        &mut self,
        lit: Literal,
        weight_str: &str,
    ) -> Result<(), VitriError> {
        let dimacs = lit.to_dimacs();
        let c = std::ffi::CString::new(weight_str).map_err(|_| {
            VitriError::input(format!(
                "the weight for literal {dimacs} cannot be passed to the shim: {weight_str:?}"
            ))
        })?;
        // SAFETY: live handle (§ Safety). `c` is NUL-terminated, outlives the
        // call, and the shim parses it into its own field value without keeping
        // the pointer.
        unsafe { ffi::arjun_shim_set_lit_weight(self.raw, dimacs, c.as_ptr()) };
        Ok(())
    }

    /// Replicate upstream Arjun's no-`c p show` branch: all vars into the sampling +
    /// opt-sampling sets (full weighted count). Pair with `all_indep=true`.
    pub(in crate::preprocess) fn clean_sampl(&mut self) {
        // SAFETY: live handle (§ Safety); no arguments to validate.
        unsafe { ffi::arjun_shim_clean_sampl(self.raw) };
    }

    /// Current per-literal weight as a rational/decimal string (current reduced
    /// numbering). Returns "1" for a var with no explicit weight.
    ///
    /// # Errors
    ///
    /// The shim handed back bytes that are not text. An empty string in their
    /// place would read as "no weight" and silently change the count.
    pub(in crate::preprocess) fn lit_weight_decimal(&self, lit: i32) -> Result<String, VitriError> {
        // SAFETY: live handle (§ Safety); this getter carries the literal
        // alongside it, and the read-back protocol is `read_c_string`'s.
        self.read_c_string(|buf, cap| unsafe {
            ffi::arjun_shim_lit_weight(self.raw, lit, buf, cap)
        })
        .map_err(|e| {
            VitriError::input(format!(
                "the weight reported for literal {lit} is not text: {e}"
            ))
        })
    }

    /// Cap the Puura backbone/probing effort inside the heavy simplify stage to
    /// `max_confl` conflicts (`SimpConf::backbone_max_confl`). `-1` = Arjun's
    /// default (unlimited), so leaving it unset keeps the full-config path
    /// byte-identical. Count-preserving (bounds search effort only).
    pub(in crate::preprocess) fn set_backbone_max_confl(&mut self, max_confl: i64) {
        unsafe { ffi::arjun_shim_set_backbone_max_confl(self.raw, max_confl) };
    }

    /// Bound the heavy stage's oracle SAT work by scaling `SimpConf::oracle_mult`
    /// (default 1.0). Arjun's oracle budgets its per-pass effort as a fixed
    /// constant × `oracle_mult` "mems", so this is a linear scalar on total
    /// oracle work: `mult=0.3` ⇒ roughly a third of the worst-case oracle budget.
    /// `< 0` leaves Arjun's default untouched. Count-preserving at any value: a
    /// smaller mult only lets the oracle prove fewer clause removals before its
    /// mems budget aborts the pass (larger-but-exact reduction).
    pub(in crate::preprocess) fn set_oracle_mult(&mut self, mult: f64) {
        unsafe { ffi::arjun_shim_set_oracle_mult(self.raw, mult) };
    }

    /// Arm Arjun's own wall-clock deadline at `deadline`, so a stage stops
    /// there and returns instead of running to completion.
    ///
    /// Every other bound in the Arjun stack counts operations (mems /
    /// conflicts / steps), and the wall-clock cost per operation varies by
    /// orders of magnitude across instances, so none of them bounds elapsed
    /// time — this is the one that does.
    ///
    /// Sound at any value: the deadline gates optional work only, so a stage
    /// cut short yields a less-reduced but exactly count-preserving formula,
    /// and Arjun's finalization (renumber + sampling-set cleanup + CNF
    /// read-back) always runs — see `vendor/arjun/arjun_shim.h`. Call once
    /// before the stages; it is absolute from the moment of the call. An
    /// already-passed deadline arms 0 ms, which stops at the first check
    /// rather than disabling the deadline.
    pub(in crate::preprocess) fn set_deadline(&mut self, deadline: Instant) {
        let ms = crate::budget::remaining(deadline).as_millis();
        // SAFETY: live handle (§ Safety). The millisecond count is clamped to
        // `i64::MAX` before the cast, so it cannot wrap negative — which the shim
        // reads as "no deadline at all" rather than as one already passed.
        unsafe { ffi::arjun_shim_set_deadline_ms(self.raw, ms.min(i64::MAX as u128) as i64) };
        self.deadline_armed = true;
    }

    /// Whether [`Self::set_deadline`] has been called on this handle, i.e. whether
    /// a stage that stops past the deadline stopped *cooperatively* (layer 1) or
    /// simply ran long. [`classify_budget`] needs the distinction: only an armed
    /// deadline can produce a DEADLINE-CUT return.
    pub(in crate::preprocess) fn deadline_armed(&self) -> bool {
        self.deadline_armed
    }

    /// Allocate `n` new fresh variables in the shim's formula.
    pub(in crate::preprocess) fn new_vars(&mut self, n: u32) {
        unsafe { ffi::arjun_shim_new_vars(self.raw, n) };
    }

    /// Add a clause in DIMACS form (1-based, signed, no trailing 0).
    pub(in crate::preprocess) fn add_clause_dimacs(&mut self, lits: &[i32]) {
        // SAFETY: live handle (§ Safety); pointer and length describe one live
        // slice, and the shim copies the literals into its own vector.
        unsafe { ffi::arjun_shim_add_clause(self.raw, lits.as_ptr(), lits.len()) };
    }

    /// Sampling (independent-support) set, 0-based variable ids.
    pub(in crate::preprocess) fn set_sampl(&mut self, vars0: &[u32]) {
        // SAFETY: live handle (§ Safety); pointer and length describe one live
        // slice, which the shim copies.
        unsafe { ffi::arjun_shim_set_sampl(self.raw, vars0.as_ptr(), vars0.len()) };
    }

    /// Cheap stage: independent-support minimization (in place). Returns true on
    /// success; on failure the checkpoint is left at the last good state.
    pub(in crate::preprocess) fn stage_minimize_indep(&mut self, all_indep: bool) -> bool {
        // SAFETY: live handle (§ Safety). The stage catches every C++ exception
        // and reports failure as a nonzero return, so none crosses back here.
        unsafe { ffi::arjun_shim_stage_minimize_indep(self.raw, all_indep as c_int) == 0 }
    }

    /// Heavy stage: the full `elim_to_file` pipeline (extend-indep + autarky +
    /// BCE + SBVA + renumber + BVE/oracle simplify), matching upstream Arjun's CLI.
    /// `all_indep` must match the value passed to [`Self::stage_minimize_indep`].
    /// `oracle` gates the expensive oracle passes (see the shim header): disable
    /// them when little budget remains so the stage can't overrun it.
    /// `no_sbva` disables SBVA (count-preserving) — the OOM-triggered revert.
    /// `no_bve` disables BVE (count-preserving) — keeps gate variables in the
    /// reduced formula for a consumer that can exploit them.
    pub(in crate::preprocess) fn stage_simplify(
        &mut self,
        all_indep: bool,
        oracle: bool,
        no_sbva: bool,
        no_bve: bool,
    ) -> bool {
        // SAFETY: live handle (§ Safety). As with the minimize stage, every C++
        // exception is caught inside the shim and reported as a nonzero return.
        unsafe {
            ffi::arjun_shim_stage_simplify(
                self.raw,
                all_indep as c_int,
                oracle as c_int,
                no_sbva as c_int,
                no_bve as c_int,
            ) == 0
        }
    }

    /// The shim's probe-then-fill protocol for a list getter (§ Safety), in
    /// one place: probe with a null buffer for the length, then hand back
    /// exactly that many elements. Every list getter has this signature, which
    /// is why one function serves all six.
    fn read_list<T: Copy + Default>(
        &self,
        get: unsafe extern "C" fn(*mut ffi::ArjunShim, *mut T, usize) -> usize,
    ) -> Vec<T> {
        // SAFETY: live handle (§ Safety). The probe writes nothing and reports
        // the length; the refill is handed a buffer of exactly that length, and
        // nothing between the two calls can change it, since both take `&self`.
        unsafe {
            let need = get(self.raw, std::ptr::null_mut(), 0);
            let mut buf = vec![T::default(); need];
            get(self.raw, buf.as_mut_ptr(), buf.len());
            buf
        }
    }

    /// The same protocol for a text getter, whose buffer is one byte longer
    /// than the probe's answer because the shim NUL-terminates inside the
    /// capacity it is handed. `get` calls one shim getter with `(buf, cap)`;
    /// the two text getters differ in what they carry beside the handle.
    ///
    /// # Errors
    ///
    /// The bytes written are not UTF-8. The caller names what it was reading.
    fn read_c_string(
        &self,
        get: impl Fn(*mut c_char, usize) -> usize,
    ) -> Result<String, std::string::FromUtf8Error> {
        let need = get(std::ptr::null_mut(), 0);
        let mut buf = vec![0u8; need + 1];
        get(buf.as_mut_ptr().cast::<c_char>(), buf.len());
        if let Some(nul) = buf.iter().position(|&b| b == 0) {
            buf.truncate(nul);
        }
        String::from_utf8(buf)
    }

    /// Variable count of the current (most-reduced-so-far) checkpoint.
    pub(in crate::preprocess) fn cur_nvars(&self) -> u32 {
        // SAFETY: live handle (§ Safety); reads the checkpoint's variable count.
        unsafe { ffi::arjun_shim_cur_nvars(self.raw) }
    }

    /// The current checkpoint's clauses, flattened DIMACS with a 0 after each.
    pub(in crate::preprocess) fn cur_clauses_dimacs(&self) -> Vec<i32> {
        self.read_list(ffi::arjun_shim_cur_clauses)
    }

    /// Redundant/learnt clauses harvested from the current checkpoint's
    /// `red_clauses` (clauses Arjun's oracle simplify proved implied by the
    /// reduced formula), as per-clause DIMACS `i32` lists in the same
    /// (reduced/renumbered) numbering as [`Self::cur_clauses_dimacs`] /
    /// [`Self::cur_formula`] — both read the one `s->cur`. Length/total caps
    /// are enforced in the shim. Empty when the oracle passes did not run.
    pub(in crate::preprocess) fn red_clauses(&self) -> Vec<Vec<i32>> {
        let buf = self.read_list(ffi::arjun_shim_red_clauses);
        if buf.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut cur = Vec::new();
        for &val in &buf {
            if val == 0 {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push(val);
            }
        }
        out
    }

    /// The current checkpoint's sampling (show/independent-support) set, 0-based
    /// var IDs in the same numbering as [`Self::cur_clauses_dimacs`] /
    /// [`Self::cur_formula`] — both read from the one `s->cur` SimplifiedCNF,
    /// rewritten in lock-step by the elim_to_file renumber. The projected
    /// analogue of [`Self::backbone`]/[`Self::eq_lits`].
    pub(in crate::preprocess) fn cur_sampl(&self) -> Vec<u32> {
        self.read_list(ffi::arjun_shim_cur_sampl)
    }

    /// Backbone literals discovered at the minimize stage (forced in every
    /// model), as [`Literal`]s in the INPUT var space (0-based `VarId`).
    pub(in crate::preprocess) fn backbone(&self) -> Vec<Literal> {
        self.read_list(ffi::arjun_shim_backbone)
            .into_iter()
            .map(Literal::from)
            .collect()
    }

    /// Equivalence literal pairs discovered at the minimize stage, as `(a, b)`
    /// [`Literal`]s in the INPUT var space encoding `a ≡ b`.
    pub(in crate::preprocess) fn eq_lits(&self) -> Vec<(Literal, Literal)> {
        self.read_list(ffi::arjun_shim_eq_lits)
            .chunks_exact(2)
            .map(|p| (Literal::from(p[0]), Literal::from(p[1])))
            .collect()
    }

    /// INPUT variable → reduced literal, read off the current checkpoint's own
    /// `orig_to_new_var` (see the shim header for Arjun's coverage contract).
    ///
    /// The source space is the formula fed via
    /// [`Self::new_vars`]/[`Self::add_clause_dimacs`] (`input_num_vars`
    /// entries), the target space is [`Self::cur_formula`]'s numbering;
    /// [`VarMap`] states the encoding.
    pub(in crate::preprocess) fn orig_to_new_lits(
        &self,
        input_num_vars: u32,
    ) -> VarMap<Reduced, Reduced> {
        let buf = self.read_list(ffi::arjun_shim_orig_to_new);
        let mut map = vec![None; input_num_vars as usize];
        if buf.is_empty() {
            return VarMap::from_entries(map);
        }
        for pair in buf.chunks_exact(2) {
            let (orig, new_lit) = (pair[0], pair[1]);
            // Defensive: Arjun's `new_var()` would append entries keyed beyond
            // the fed variable count. No stage this shim drives does that, but a
            // key we cannot name in the input space must never be silently
            // written somewhere else.
            if orig < 1 || new_lit == 0 {
                continue;
            }
            let idx = (orig as u32 - 1) as usize;
            if idx < map.len() {
                map[idx] = Some(new_lit);
            }
        }
        VarMap::from_entries(map)
    }

    /// Multiplier as an exact decimal string: original = reduced * multiplier.
    ///
    /// # Errors
    ///
    /// The shim handed back bytes that are not text. An empty string in their
    /// place would carry no multiplier at all and silently change the count.
    pub(in crate::preprocess) fn cur_multiplier_decimal(&self) -> Result<String, VitriError> {
        // SAFETY: live handle (§ Safety); the read-back protocol is
        // `read_c_string`'s.
        self.read_c_string(|buf, cap| unsafe { ffi::arjun_shim_cur_multiplier(self.raw, buf, cap) })
            .map_err(|e| VitriError::input(format!("the count multiplier is not text: {e}")))
    }

    /// Build the current checkpoint into a [`CnfFormula`]. Variable ids are
    /// 0-based, as everywhere else in this crate; Arjun hands them out 1-based,
    /// which [`Literal::from`] converts.
    pub(in crate::preprocess) fn cur_formula(&self) -> CnfFormula {
        let flat = self.cur_clauses_dimacs();
        let mut clauses = Vec::new();
        let mut declared = self.cur_nvars();
        let mut lits = Vec::new();
        for &val in &flat {
            if val == 0 {
                let max_var = lits
                    .iter()
                    .map(|l: &Literal| l.var.to_dimacs() as u32)
                    .max()
                    .unwrap_or(0);
                if max_var > declared {
                    declared = max_var;
                }
                clauses.push(Clause::new(std::mem::take(&mut lits)));
            } else {
                lits.push(Literal::from(val));
            }
        }
        CnfFormula {
            num_vars: declared,
            clauses,
        }
    }
}

impl Drop for ArjunLib {
    fn drop(&mut self) {
        // SAFETY: the handle (§ Safety); this is the one release it describes.
        unsafe { ffi::arjun_shim_free(self.raw) };
    }
}

/// How the two presence-only shim switches read, quoted in their error message.
const PRESENCE_ONLY_FORM: &str = "set to any value to turn the pass off, or left \
                                  unset to leave it on — an off-looking value still \
                                  turns it off, so it is refused rather than obeyed";

/// The spellings a presence-only switch will not obey. Its vocabulary is
/// otherwise open — being set at all turns the pass off — so what it needs is
/// the refused list, not an accepted one.
const OFF_LOOKING_FORMS: &[&str] = &["", "0", "off", "false"];

/// Whether a presence-only switch's value reads as "off", and so is one it
/// refuses rather than obeys. The pure half of the check in
/// [`validate_shim_env`].
pub(in crate::preprocess) fn reads_as_off(value: &str) -> bool {
    OFF_LOOKING_FORMS
        .iter()
        .any(|form| crate::env::is_form(value, form))
}

/// What `VITRI_ARJUN_BVE_GROW` accepts, quoted in its error message. Both bounds
/// are the vendored reader's own: it takes nothing below zero, because the
/// growth budget it clamps is a count of extra clauses, and nothing above
/// `INT_MAX`, because it keeps that count in a C `int`.
const BVE_GROW_FORM: &str = "a clause-growth budget, a whole number from 0 to 2147483647";

/// Parses the `VITRI_ARJUN_BVE_GROW` value — the pure half of the check in
/// [`validate_shim_env`], so the accepted set can be read back without
/// mutating the process environment.
///
/// The return type carries the ceiling: `i32` is the C `int` the budget ends up
/// in, so a value past the top fails to parse here for the reason it is refused
/// there, and the bound has one spelling rather than two that can drift apart.
pub(in crate::preprocess) fn bve_grow_value(value: Option<&str>) -> Result<i32, VitriError> {
    let grow = crate::env::parse_value("VITRI_ARJUN_BVE_GROW", value, 0, BVE_GROW_FORM)?;
    if grow < 0 {
        return Err(VitriError::env(
            "VITRI_ARJUN_BVE_GROW",
            format!("must be {BVE_GROW_FORM}; got {grow}"),
        ));
    }
    Ok(grow)
}

/// Check the `VITRI_*` variables the shim itself reads.
///
/// Checked here, in the parent, before any shim is built: the shim can only
/// refuse to start or fail a stage, which reaches the caller as "Arjun found
/// nothing" rather than as "this variable is set to something it cannot mean".
///
/// Each check below must accept no more than its `getenv` on the other side
/// does: a value that gets past here and is then refused there costs a whole
/// stage with nothing said on any channel.
///
/// # Errors
///
/// [`VitriError::Env`] naming the offending variable.
pub(in crate::preprocess) fn validate_shim_env() -> Result<(), VitriError> {
    bve_grow_value(crate::env::env_raw("VITRI_ARJUN_BVE_GROW", BVE_GROW_FORM)?.as_deref())?;
    for var in ["VITRI_ARJUN_NO_BVE", "VITRI_ARJUN_NO_ORACLE"] {
        if let Some(value) = crate::env::env_raw(var, PRESENCE_ONLY_FORM)?
            && reads_as_off(&value)
        {
            return Err(VitriError::env(
                var,
                format!("must be {PRESENCE_ONLY_FORM}; got {value:?}"),
            ));
        }
    }
    Ok(())
}
