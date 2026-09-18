//! DIMACS CNF text ↔ [`CnfFormula`] plus the [`CnfMeta`] the competition
//! meta-comment lines declare.
//!
//! One reader and one writer, so the line kinds one accepts and the other emits
//! are decided together. One error type. The range rule every id-bearing
//! construct is held to lives here too ([`WidestId`]), so a new construct is
//! wired into it rather than given a check of its own.

use std::io::BufRead;

use num_bigint::BigInt;
use num_rational::BigRational;

use crate::error::VitriError;

use super::weights::LiteralWeight;
use super::{Clause, CnfFormula, CnfMeta, Literal, Mode, ShowSet, Space, WeightTable};

/// An exact rational as `"numerator/denominator"` — the inverse of
/// [`parse_weight`], and the spelling every written weight uses.
/// `BigRational` keeps itself in lowest terms with a positive denominator, so
/// this is canonical: two equal weights always produce the same string.
pub(crate) fn rational_string(r: &BigRational) -> String {
    format!("{}/{}", r.numer(), r.denom())
}

/// Parse a weight token into an exact rational — the spellings a DIMACS
/// `c p weight` line uses: a fraction `a/b`, a decimal with an optional
/// `e`/`E` exponent, or a plain integer. Surrounding whitespace is ignored.
///
/// Exact throughout: a competition instance's weights are rationals, and
/// rounding one through an `f64` changes the count it produces. A caller that
/// reads weights from somewhere other than a CNF file — its own configuration,
/// a command line — parses them here so its weights and this crate's agree.
///
/// # Errors
///
/// [`VitriError::Input`] naming the token that could not be read.
pub fn parse_rational_weight(s: &str) -> Result<BigRational, VitriError> {
    parse_weight(s).map_err(VitriError::input)
}

/// Parse an exact rational weight token: a fraction `a/b`, a decimal
/// `[-]d.ddd` (optionally with a `e`/`E` exponent), or a plain integer.
/// Exactness is required for competition precision-category-A; no `f64`
/// rounding.
pub(crate) fn parse_weight(s: &str) -> Result<BigRational, String> {
    let s = s.trim();
    if let Some((n, d)) = s.split_once('/') {
        let num: BigInt = n
            .trim()
            .parse()
            .map_err(|_| format!("invalid weight numerator: {s}"))?;
        let den: BigInt = d
            .trim()
            .parse()
            .map_err(|_| format!("invalid weight denominator: {s}"))?;
        if den.sign() == num_bigint::Sign::NoSign {
            return Err(format!("zero denominator in weight: {s}"));
        }
        return Ok(BigRational::new(num, den));
    }
    // Decimal with optional exponent: mantissa[e±exp].
    let (mantissa, exp) = match s.split_once(['e', 'E']) {
        Some((m, e)) => (
            m,
            e.trim()
                .parse::<i64>()
                .map_err(|_| format!("invalid weight exponent: {s}"))?,
        ),
        None => (s, 0i64),
    };
    let neg = mantissa.starts_with('-');
    let mant = mantissa.trim_start_matches(['+', '-']);
    let (int_part, frac_part) = match mant.split_once('.') {
        Some((i, f)) => (i, f),
        None => (mant, ""),
    };
    let digits: String = format!("{int_part}{frac_part}");
    let digits = if digits.is_empty() {
        "0".to_string()
    } else {
        digits
    };
    let mut num: BigInt = digits
        .parse()
        .map_err(|_| format!("invalid weight value: {s}"))?;
    if neg {
        num = -num;
    }
    // value = num × 10^(exp − frac_len).
    let scale = exp - frac_part.len() as i64;
    let ten = || BigInt::from(10);
    if scale >= 0 {
        num *= num_traits::pow(ten(), scale as usize);
        Ok(BigRational::from_integer(num))
    } else {
        let den = num_traits::pow(ten(), (-scale) as usize);
        Ok(BigRational::new(num, den))
    }
}

/// The widest variable id the file named, and which line named it.
///
/// The `p cnf` header declares the variable count, and every id written
/// anywhere in the file — a clause literal, a `c p show` variable, a `c p
/// weight` literal — must fall inside it. Nothing downstream re-checks: a
/// clause literal past the end indexes past the per-variable tables the
/// preprocessing builds, and a show variable past the end reaches the vendored
/// sampling-set check, which ends the process. So this is caught here, once,
/// for every construct that carries an id.
///
/// Checked after the whole file is read, not at each site: the declared count
/// isn't known until the `p` line, and meta-comment lines aren't required to
/// come after it — real MCC track-4 instances write `c t` and `c p show`
/// above the header. Only the widest id is kept — it decides whether anything
/// is out of range.
#[derive(Clone, Copy)]
struct WidestId {
    /// What named it, in the words the message uses: `clause literal`,
    /// `show var`, `weight literal`.
    kind: &'static str,
    /// The id exactly as the file spells it, sign included.
    written: i64,
    /// The 1-based line the id was written on.
    line: usize,
}

impl WidestId {
    /// Record `written` — a variable id or a literal, as the file spells it —
    /// as named by a `kind` construct on line `line`.
    ///
    /// The entry point for every id-bearing construct the parser accepts: a new
    /// one is wired in here rather than given a range check of its own.
    fn note(widest: &mut Option<WidestId>, kind: &'static str, written: i64, line: usize) {
        let wider = match widest {
            Some(w) => written.unsigned_abs() > w.written.unsigned_abs(),
            None => true,
        };
        if wider {
            *widest = Some(WidestId {
                kind,
                written,
                line,
            });
        }
    }

    /// The one range rule: an id is in range when `1 <= |id| <= num_vars`. `0`
    /// never reaches here — it terminates a clause and a show set rather than
    /// naming a variable.
    fn check(widest: Option<WidestId>, num_vars: u32) -> Result<(), String> {
        match widest {
            Some(w) if w.written.unsigned_abs() > u64::from(num_vars) => Err(format!(
                "line {}: {} {} exceeds the declared variable count {num_vars}",
                w.line, w.kind, w.written,
            )),
            _ => Ok(()),
        }
    }
}

/// Close `current_clause` and add it to `clauses`, normalized: literals sorted
/// by variable, exact duplicates dropped, and the whole clause dropped when a
/// variable occurs in both polarities — a tautology, which every assignment
/// satisfies. `current_clause` is left empty for the next clause either way.
///
/// The one place a parsed clause is built, so a `0`-terminated clause and a
/// final clause whose `0` the file omits are closed the same way.
fn close_clause(current_clause: &mut Vec<Literal>, clauses: &mut Vec<Clause>) {
    clauses.extend(Clause::normalized(std::mem::take(current_clause)));
}

impl CnfFormula {
    /// Parse a DIMACS CNF file into the clause set and the [`CnfMeta`] its MCC
    /// `c t` / `c p show` / `c p weight` meta-comment lines declare.
    ///
    /// One entry point, one error type, whether or not the caller cares about
    /// the metadata. For a plain MC instance (no such lines) the meta is
    /// `CnfMeta::default()`.
    ///
    /// Recognized line types:
    /// - `c p show <v1> … 0` — the show set (accumulated across lines, sorted+deduped)
    /// - `c t {mc,wmc,pmc,pwmc}` — declared count type
    /// - `c p weight <lit> <w> 0` — literal weight (exact rational)
    /// - `c ...` — other comment (skipped)
    /// - `p cnf <vars> <clauses>` — problem header (required)
    /// - `w ...` — PMC weight line (skipped)
    /// - `%` — SATLIB EOF marker (stops parsing)
    /// - Integer tokens separated by whitespace, `0`-terminated — clause data
    ///
    /// A `0` with no literals before it — a bare `0` line, or the second of two
    /// in a row — is the empty clause: constant false, so the formula is UNSAT
    /// and its count is 0. It is kept, not skipped. A final clause whose
    /// terminating `0` the file omits is accepted, normalized like any other.
    ///
    /// The header clause count is advisory (extra clauses are accepted). The
    /// header variable count is not: every id the file names — clause literal,
    /// show variable, weight literal — must satisfy `1 <= |id| <= vars`, and a
    /// file that names one above it is rejected, naming the id, the count and
    /// the line. Non-integer tokens in clause data are rejected with an error.
    ///
    /// The metadata is expressed over original DIMACS variable ids; callers on
    /// the projected/weighted path must remap it as preprocessing renumbers
    /// variables.
    ///
    /// # Errors
    ///
    /// [`VitriError::Input`] naming the line and what is wrong with it.
    pub fn from_dimacs<R: BufRead>(reader: R) -> Result<(Self, CnfMeta), VitriError> {
        Self::parse_dimacs(reader).map_err(VitriError::input)
    }

    /// The parse itself, reporting a plain sentence. Private: the sentence
    /// becomes a [`VitriError`] at the one public entry point above.
    fn parse_dimacs<R: BufRead>(reader: R) -> Result<(Self, CnfMeta), String> {
        // The declared variable count, and whether the header was there at all:
        // `p cnf 0 1` declares no variables, which is not the same as a file
        // that declares nothing.
        let mut header: Option<u32> = None;
        let mut clauses = Vec::new();
        let mut current_clause: Vec<Literal> = Vec::new();
        let mut line_num = 0usize;
        let mut meta_lines = MetaLines::default();
        // See [`WidestId`].
        let mut widest: Option<WidestId> = None;

        for line in reader.lines() {
            let line = line.map_err(|e| e.to_string())?;
            let line = line.trim();
            line_num += 1;

            // Do not stop at the declared clause count: benchmark generators
            // (e.g. Bayesian network encodings) often append extra clauses —
            // evidence/observation unit clauses — beyond the header without
            // updating it.

            if line.is_empty() {
                continue;
            }

            match line.as_bytes()[0] {
                b'c' => {
                    meta_lines.read_comment(line, line_num, &mut widest)?;
                    continue;
                }
                b'%' => break, // SATLIB EOF marker
                // PMC weight line (prefixed format). Discarded whole — the
                // variable it names never enters the formula or the metadata —
                // so it carries no id to range-check, and rejecting one would
                // refuse a file that parses correctly today.
                b'w' => continue,
                b'p' => {
                    header = Some(read_problem_line(line, line_num, &mut clauses)?);
                    continue;
                }
                _ => {}
            }

            // Weight line detection: some PMC files use bare floats (no `w` prefix)
            // as weight data after clauses. If the first token on a line contains a
            // decimal point, treat the entire line as a weight line and skip it.
            if let Some(first) = line.split_whitespace().next()
                && first.contains('.')
            {
                continue;
            }

            read_clause_line(
                line,
                line_num,
                &mut current_clause,
                &mut clauses,
                &mut widest,
            )?;
        }

        // A final clause whose terminating `0` the file omits — some writers
        // leave it off. Closed on the same path as a terminated one.
        if !current_clause.is_empty() {
            close_clause(&mut current_clause, &mut clauses);
        }

        let num_vars = match header {
            Some(declared) => declared,
            // An empty file parses to an empty formula, as it always has. A
            // file with clauses and no header does not: nothing says which
            // variable space they are written over.
            None if clauses.is_empty() => 0,
            None => return Err("Missing problem line".to_string()),
        };

        // Every id the file named, against the count the header declared. Ahead
        // of the conversions in [`MetaLines::into_meta`], both of which assume
        // ids that fit.
        WidestId::check(widest, num_vars)?;

        let meta = meta_lines.into_meta(num_vars)?;
        Ok((CnfFormula::from_parts(num_vars, clauses), meta))
    }
}

/// The MCC meta-comment lines read so far, held as written. Both conversions
/// into indexed form subtract one from a written id, and the weight table sizes
/// itself from one, so they wait in [`MetaLines::into_meta`] until every id is
/// known to fit the declared count.
#[derive(Default)]
struct MetaLines {
    /// Stays `None` until a `c t` line names a track, so that `c t mc` and a
    /// file carrying no such line remain distinguishable — see
    /// [`CnfMeta::declared_track`].
    track: Option<Mode>,
    /// Show variables as written, accumulated across `c p show` lines.
    show: Vec<i64>,
    /// Distinguishes "no `c p show` line" from "`c p show 0`" — a legitimate
    /// empty declaration, see [`CnfMeta::declared_show_vars`].
    saw_show: bool,
    /// `(written literal, weight)` per `c p weight` line, in file order.
    weight_lines: Vec<(i32, BigRational)>,
}

impl MetaLines {
    /// Read one `c` line. `c t`, `c p show` and `c p weight` land here; every
    /// other comment is skipped.
    fn read_comment(
        &mut self,
        line: &str,
        line_num: usize,
        widest: &mut Option<WidestId>,
    ) -> Result<(), String> {
        let toks: Vec<&str> = line.split_whitespace().collect();
        match toks.as_slice() {
            ["c", "t", ty] => {
                self.track = Some(
                    Mode::parse_track(ty)
                        .ok_or_else(|| format!("line {line_num}: unknown problem type: {ty}"))?,
                );
            }
            ["c", "p", "show", rest @ ..] => {
                self.saw_show = true;
                self.read_show_line(rest, line_num, widest)?;
            }
            ["c", "p", "weight", lit, w, ..] => {
                // Trailing token (a `0` line terminator) is ignored.
                let (l, val) = read_weight_line(lit, w, line_num)?;
                WidestId::note(widest, "weight literal", i64::from(l), line_num);
                self.weight_lines.push((l, val));
            }
            _ => {}
        }
        Ok(())
    }

    /// Read the variables a `c p show … 0` line declares, as written. The `0`
    /// ends the line; a negative id is rejected here rather than at the range
    /// check, which reports widths.
    fn read_show_line(
        &mut self,
        rest: &[&str],
        line_num: usize,
        widest: &mut Option<WidestId>,
    ) -> Result<(), String> {
        for t in rest {
            let v: i64 = t
                .parse()
                .map_err(|_| format!("line {line_num}: invalid show var: {t:?}"))?;
            if v == 0 {
                break;
            }
            if v < 0 {
                return Err(format!("line {line_num}: negative show var: {v}"));
            }
            WidestId::note(widest, "show var", v, line_num);
            self.show.push(v);
        }
        Ok(())
    }

    /// Convert to [`CnfMeta`], every id now known to be within `1..=num_vars`.
    fn into_meta(self, num_vars: u32) -> Result<CnfMeta, String> {
        let show_vars = if self.saw_show {
            // Narrowing to the written width cannot truncate an id the range
            // check accepted.
            let ids: Vec<u32> = self.show.into_iter().map(|v| v as u32).collect();
            Some(ShowSet::from_dimacs_ids(&ids).map_err(|e| e.to_string())?)
        } else {
            None
        };
        let weights = if self.weight_lines.is_empty() {
            None
        } else {
            Some(
                WeightTable::from_dimacs_pairs(self.weight_lines, num_vars)
                    .map_err(|e| e.to_string())?,
            )
        };
        CnfMeta::from_parts(num_vars, self.track, show_vars, weights).map_err(|e| e.to_string())
    }
}

/// Read the `p cnf <vars> <clauses>` header, returning the declared variable
/// count. The clause count only sizes `clauses`: extra clauses are accepted.
fn read_problem_line(
    line: &str,
    line_num: usize,
    clauses: &mut Vec<Clause>,
) -> Result<u32, String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 || parts[1] != "cnf" {
        return Err(format!("line {line_num}: invalid problem line: {line}"));
    }
    let num_vars = parts[2]
        .parse()
        .map_err(|_| format!("line {}: invalid variable count: {}", line_num, parts[2]))?;
    let nc: usize = parts[3]
        .parse()
        .map_err(|_| format!("line {}: invalid clause count: {}", line_num, parts[3]))?;
    clauses.reserve(nc);
    Ok(num_vars)
}

/// Read the literal and the exact rational of a `c p weight <lit> <w> [0]`
/// line.
fn read_weight_line(lit: &str, w: &str, line_num: usize) -> Result<(i32, BigRational), String> {
    let l: i32 = lit
        .parse()
        .map_err(|_| format!("line {line_num}: invalid weight literal: {lit:?}"))?;
    if l == 0 {
        return Err(format!("line {line_num}: weight on literal 0"));
    }
    let val = parse_weight(w).map_err(|e| format!("line {line_num}: {e}"))?;
    Ok((l, val))
}

/// Read one line of clause data: whitespace-separated DIMACS literals, each `0`
/// closing the clause read so far.
fn read_clause_line(
    line: &str,
    line_num: usize,
    current_clause: &mut Vec<Literal>,
    clauses: &mut Vec<Clause>,
    widest: &mut Option<WidestId>,
) -> Result<(), String> {
    for token in line.split_whitespace() {
        let val: i32 = token
            .parse()
            .map_err(|_| format!("line {line_num}: invalid token in clause data: {token:?}"))?;
        if val == 0 {
            // Closes the clause read so far, the empty one included: a `0` with
            // no literals before it is the empty clause — constant false, so
            // the formula is UNSAT and its count is 0. Reading past it drops
            // the one clause no assignment satisfies, and nothing downstream
            // can re-derive a contradiction that was never read.
            close_clause(current_clause, clauses);
        } else {
            WidestId::note(widest, "clause literal", i64::from(val), line_num);
            current_clause.push(Literal::from(val));
        }
    }
    Ok(())
}

/// The MCC meta-comment lines a written CNF carries, so the file describes the
/// problem it belongs to rather than depending on a sibling JSON.
#[derive(Debug)]
pub(crate) struct DimacsHeader<'a, S: Space> {
    /// The `c t <track>` token, or `None` for a bare DIMACS file.
    pub track: Option<&'a str>,
    /// The `c p show` set, over the space `S` this file is written in.
    pub show: Option<&'a ShowSet<S>>,
    /// The `c p weight` lines, literals in this file's own space.
    pub weights: Option<&'a [LiteralWeight]>,
}

impl<S: Space> Default for DimacsHeader<'_, S> {
    fn default() -> Self {
        Self {
            track: None,
            show: None,
            weights: None,
        }
    }
}

/// Write `formula` to `w` as DIMACS, carrying `header`'s meta-comment lines.
///
/// THE DIMACS writer for this crate — one emitter, one line order. The `p cnf`
/// header comes FIRST and the meta-comments after it: that order is what Arjun's
/// own DIMACS parser requires, and standard DIMACS readers ignore comments
/// wherever they appear — the reader above included, which takes the two in
/// either order because real competition instances write them either way.
///
/// Nothing is flushed here; the bundle's file sink owns the writer and flushes
/// it while it can still report the failure against the file's name.
pub(crate) fn write_dimacs<S: Space, W: std::io::Write>(
    formula: &CnfFormula,
    header: &DimacsHeader<'_, S>,
    mut w: W,
) -> std::io::Result<()> {
    // The empty clause has no portable DIMACS spelling: it would be written as a
    // lone `0` line, which most readers take for a clause terminator or a SATLIB
    // end marker rather than the contradiction it is. Every producer must
    // therefore convert a refutation into an explicit `x ∧ ¬x` (see
    // `PreprocessRecord::unsat`) before it reaches this writer, and this guard is
    // what keeps a future one from skipping that step and shipping a bundle whose
    // UNSAT re-parses as a nonzero count somewhere else.
    debug_assert!(
        !formula.is_refuted(),
        "refusing to write the empty clause — DIMACS cannot express it, so the file \
         would re-parse as satisfiable; emit an explicit contradiction instead",
    );
    emit_problem_line(&mut w, formula)?;
    emit_meta_lines(&mut w, header)?;
    emit_clause_lines(&mut w, formula)
}

/// `p cnf <vars> <clauses>` — the declared universe and the clause count.
fn emit_problem_line<W: std::io::Write>(w: &mut W, formula: &CnfFormula) -> std::io::Result<()> {
    writeln!(w, "p cnf {} {}", formula.num_vars, formula.clauses.len())
}

/// The meta-comment lines a header carries, in the order the readers this crate
/// writes for expect them.
fn emit_meta_lines<W: std::io::Write, S: Space>(
    w: &mut W,
    header: &DimacsHeader<'_, S>,
) -> std::io::Result<()> {
    if let Some(t) = header.track {
        writeln!(w, "c t {t}")?;
    }
    if let Some(show) = header.show {
        write!(w, "c p show")?;
        for v in show.as_dimacs() {
            write!(w, " {v}")?;
        }
        writeln!(w, " 0")?;
    }
    if let Some(weights) = header.weights {
        for weight in weights {
            writeln!(w, "c p weight {} {} 0", weight.literal, weight.weight)?;
        }
    }
    Ok(())
}

/// The clause body: one clause per line, literals space-separated, `0` ending
/// each line. THE literal encoding for this crate's output — every writer above
/// reaches it, so a clause is spelled the same way wherever it is written.
fn emit_clause_lines<W: std::io::Write>(w: &mut W, formula: &CnfFormula) -> std::io::Result<()> {
    for clause in &formula.clauses {
        for (i, lit) in clause.literals.iter().enumerate() {
            if i > 0 {
                write!(w, " ")?;
            }
            write!(w, "{}", lit.to_dimacs())?;
        }
        writeln!(w, " 0")?;
    }
    Ok(())
}

impl CnfFormula {
    /// Write this formula as DIMACS: the `p cnf` header line, then the clauses.
    ///
    /// The inverse of [`CnfFormula::from_dimacs`] — re-parsing what this writes
    /// gives the same formula back. No meta-comment lines are written; the
    /// bundle writer is what emits a run's track, show set and weight table
    /// alongside a formula.
    ///
    /// `num_vars` is written as declared, not as a count of the variables the
    /// clauses still mention. A formula whose universe is wider than its clauses
    /// has models the narrower universe does not, and the header line is where
    /// that is said.
    ///
    /// A formula holding the empty clause has no portable DIMACS spelling: the
    /// empty clause writes as a lone `0` line, which readers take for a
    /// terminator rather than the contradiction it is. A caller writing out a
    /// refutation writes an explicit `x ∧ ¬x` instead.
    ///
    /// # Errors
    ///
    /// Whatever `w` returns. Nothing is flushed here — a buffered writer is the
    /// caller's to flush, while it still has somewhere to report the failure.
    pub fn write_dimacs<W: std::io::Write>(&self, w: &mut W) -> std::io::Result<()> {
        emit_problem_line(w, self)?;
        emit_clause_lines(w, self)
    }

    /// The clause body alone, without the `p cnf` line: one clause per line,
    /// literals 1-based and signed, `0` ending each line.
    ///
    /// For a caller writing its own preamble — a `c p show` declaration of its
    /// own, a replay header, a diagnostic cube — that wants the literal encoding
    /// to have one definition rather than a fresh one at every such site.
    ///
    /// # Errors
    ///
    /// Whatever `w` returns.
    pub fn write_dimacs_clauses<W: std::io::Write>(&self, w: &mut W) -> std::io::Result<()> {
        emit_clause_lines(w, self)
    }
}
