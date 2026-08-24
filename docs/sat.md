# The SAT solver

vitri statically links a SAT solver, and publishes it as
[`vitri::sat`](https://tractables.github.io/vitri/vitri/sat/index.html). A
consumer that needs a SAT solver in the same process uses this one.

## One CaDiCaL per process

The solver is CaDiCaL, in the copy vendored under `vendor/arjun/upstream/` and
built from source by `build.rs`. The Arjun preprocessing stack is built against
that same copy, so a vitri process holds exactly one CaDiCaL.

Two CaDiCaL builds in one process do not coexist. They export the same
`CaDiCaL::` symbols, so static linking resolves every call to whichever archive
the linker reached first — while each build's own headers are already compiled
into the struct layouts its callers use. Nothing warns. The program links, runs,
and corrupts its heap the first time a call crosses the seam. Link order decides
which build wins; it does not stop one from winning.

Adding a solver crate beside vitri is therefore not an option, which is why the
solver is public rather than private.

`Cargo.toml` declares `links = "vitri_arjun"`. The key names the native library
`build.rs` produces, and Cargo permits one package with a given `links` value per
dependency graph. That reservation covers the whole vendored stack — CaDiCaL,
Arjun and CryptoMiniSat — so a second crate declaring it turns the collision into
a resolve-time error instead of a corrupted heap.

## The handle

[`CaDiCal`](https://tractables.github.io/vitri/vitri/sat/struct.CaDiCal.html)
owns a solver and frees it on drop. It is neither `Send` nor `Sync`.

`CaDiCal::new` returns `Option`: an allocation failure arrives as a value, and
nothing in this crate exits the calling process.

The surface is the incremental interface — `add` a literal at a time with `0`
ending a clause, `assume`, `constrain`, `solve`, then read the model with `val`
— plus `fixed`, `flippable`, `freeze`, `reserve`, `limit`, `simplify` and
`traverse_clauses`. It is the set vitri's own preprocessing needed, not a
general solver abstraction, and it does not try to be one.

```rust
use vitri::sat::{CaDiCal, Status};

let mut solver = CaDiCal::new().expect("a solver");
// (a ∨ b) ∧ (¬a ∨ b)
for lit in [1, 2, 0, -1, 2, 0] {
    solver.add(lit);
}
assert_eq!(solver.solve(), Status::Satisfiable);
assert!(solver.val(2) > 0);
```

`freeze` is what preserves a model count across inprocessing: a frozen variable
is never eliminated by bounded variable elimination or blocked clause
elimination. A caller that will ask about a variable after `simplify` freezes it
first.

## Bounding a search

`Terminator` is asked, between rounds and phases, whether to stop.
`WallClockTerminator` is the deadline case.

Attach one with `Bounded`, which connects the terminator for as long as the
guard lives and disconnects it on drop. That is the whole reason the guard
exists: no path out of the bounded region — an early return, a `?`, or an unwind
— can leave the solver holding a pointer to a terminator that is gone. `Bounded`
dereferences to the solver, so the bounded region reads exactly as it would
without it.

```rust
use std::time::Duration;
use vitri::sat::{Bounded, CaDiCal, WallClockTerminator};

let mut solver = CaDiCal::new().expect("a solver");
let status = Bounded::new(&mut solver, WallClockTerminator::new(Duration::from_secs(5))).solve();
```

`solve` honours the terminator strictly. `simplify` polls it between rounds and
phases, so a budget bounds it closely enough to prevent a runaway pass but not
to the millisecond.

A search that stops early answers `Status::Unknown`. So does one that hits a
`limit`.

## Reading the search

`redundant` and `irredundant` count the clauses currently in the database: the
learnt ones, and the ones the caller gave. `traverse_clauses` visits the
irredundant clauses, and the callback returns `false` to stop.

`search_stats` reads CDCL counters — conflicts, decisions, search propagations,
restarts, learnt clauses, and the variables the decision loop scanned. They are
cumulative over the handle's lifetime, because an incremental solver keeps
accumulating across `solve` calls. A caller measuring one interval takes a
snapshot at each end and differences them with `SearchStats::since`, which
saturates at zero rather than wrapping.

`SearchStats` is `#[non_exhaustive]`. Its counters are append-only, so a counter
added in a later release is not a breaking change for a caller that constructs
one.

`score_of` reads a variable's current activity score, which is a property of the
variable rather than of a literal — the two literals over it read the same.

It means something only after a search that accumulated some. On an untouched
solver every variable reads the same initial value, and so does one whose search
was short: CaDiCaL keeps variable activity in one of two schemes and alternates
between them, and this reads only one of the two. Read the result as a signal
about which variables *this* search found contentious. It is not a stable
property of the formula, and two runs that took different search paths will
disagree.

Those two accessors read state CaDiCaL's public class does not expose, so
`build.rs` compiles a small translation unit that includes CaDiCaL's internal
header, and the vendored header grants it access. `THIRD-PARTY.md` records the
modification, and [`building.md`](building.md) covers what that costs a build.

## What it is not

vitri does not publish a solver-independent interface, and adding one is not
planned. The type is CaDiCaL, the process holds one of it, and a consumer that
wants a different solver runs it in a different process.
