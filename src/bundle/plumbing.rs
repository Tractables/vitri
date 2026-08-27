//! Shared plumbing: the DIMACS and vtree writers, the stage configuration, and
//! the small helpers all three chains reach for.

use super::*;

use crate::cnf::{DimacsHeader, Original, Reduced, ShowSet, WeightTable, Weights, write_dimacs};
use crate::dot;
use crate::vtree::Vtree;

// ── Shared plumbing ───────────────────────────────────────────────────────────

/// The bundle for an instance `clauses` refutes, or `None` when it carries no
/// refutation.
///
/// Every stage that can derive the empty clause asks here, and the check is
/// repeated after each of them because the empty clause is the one thing DIMACS
/// cannot spell: writing it emits a lone `0` line, which parsers (including this
/// crate's, as a SATLIB end marker) read as anything but a contradiction,
/// silently turning UNSAT into a nonzero count. The answer is the synthetic
/// contradiction [`unsat_bundle`] builds, whichever chain asked.
///
/// `show_vars_reduced_dimacs` is the projected chains' declared set: a
/// refutation bundle is a contradiction over the ORIGINAL variable space, so its
/// "reduced" formula is renumbered from the input by nothing at all and the
/// declared set already reads over it.
pub(super) fn refuted(
    clauses: &[Clause],
    num_vars: u32,
    mode: Mode,
    show_vars_reduced_dimacs: Option<ShowSet<Reduced>>,
    stages: StageReport,
    telemetry: PreprocessTelemetry,
) -> Option<PreprocessBundle> {
    crate::cnf::contains_empty_clause(clauses)
        .then(|| unsat_bundle(num_vars, mode, show_vars_reduced_dimacs, stages, telemetry))
}

/// The bundle for an instance preprocessing proved UNSAT: a two-unit-clause
/// contradiction over the original variable space, the identity map, and no
/// lift. Count 0 both before and after, so the lift identity holds trivially —
/// and, unlike the empty clause, it survives a DIMACS write/read round trip.
pub(super) fn unsat_bundle(
    num_vars: u32,
    mode: Mode,
    show_vars_reduced_dimacs: Option<ShowSet<Reduced>>,
    stages: StageReport,
    telemetry: PreprocessTelemetry,
) -> PreprocessBundle {
    debug_assert!(num_vars >= 1, "an UNSAT instance has at least one variable");
    let x = VarId(0);
    PreprocessBundle {
        reduced: CnfFormula {
            num_vars,
            clauses: vec![
                Clause::new(vec![Literal::new(x, true)]),
                Clause::new(vec![Literal::new(x, false)]),
            ],
        },
        record: PreprocessRecord {
            // Present iff the mode is `compile`, exactly as on the satisfiable
            // path. Nothing was eliminated here — the reduced formula is a
            // contradiction over the original variable space — so the identity
            // is the truthful map, and it keeps the field's presence rule a
            // property of the mode rather than of the outcome.
            original_to_reduced_dimacs: matches!(mode, Mode::Compile)
                .then(|| OriginalMap::identity(num_vars)),
            unsat: true,
            show_vars_reduced_dimacs,
            // Nothing was eliminated, so there is nothing to lift back —
            // whatever the mode, and whatever the weights. `reduced_weights`
            // stays absent with it: a refuted instance has count 0 whatever the
            // weights are, so a table would be decoration on an answer that is
            // already known.
            ..PreprocessRecord::new(
                mode,
                RecordLift::neutral(),
                num_vars,
                VarMap::identity(num_vars),
            )
        },
        learnt_clauses_reduced_dimacs: Vec::new(),
        stages,
        // A refutation is counted 0 before and after, so there is nothing to
        // lift and no stage that earned any of it.
        count_lift: CountLift::default(),
        telemetry,
        arjun_input: None,
        independent_support_reduced: None,
    }
}

/// The stage configuration for an export preprocessing run: the shared
/// [`SimplifyConfig::for_purpose`] base (which owns the sound stage ceiling)
/// plus [`RunConfig::simplify`](crate::config::RunConfig::simplify), which may
/// reduce work inside that ceiling but cannot enable a stage the contract bans.
///
/// `purpose` is the caller's CONTRACT, and it is the only thing that decides
/// which stages run — a chain names its contract and takes the stage list that
/// comes with it, so there is no per-mode stage arithmetic here to get wrong.
///
/// `stages.simplify == false` is expressed on that SAME base rather than by
/// bypassing the call: [`SimplifyPrefix::Disabled`](crate::preprocess::simplify::SimplifyPrefix::Disabled)
/// suppresses preprocessing and `keep_all_vars = true` suppresses every
/// variable-eliminating tail stage. `simplify()` then returns an identity
/// `SimplifiedFormula` — one code path, one set of defaults.
///
/// The only per-call difference is that `WeightedCount` FREEZES every
/// unequal-weight variable out of DVE, so that every elimination DVE does make
/// is one a scalar can pay for.
pub(super) fn preprocess_config(
    config: &RunConfig,
    purpose: SimplifyPurpose,
    orig_w: &Weights<Original>,
) -> SimplifyConfig {
    if !config.stages.simplify {
        return SimplifyConfig {
            prefix: crate::preprocess::simplify::SimplifyPrefix::Disabled,
            deadline: config.deadline,
            ..SimplifyConfig::for_purpose(purpose, /*keep_all_vars=*/ true)
        };
    }
    let mut resolved = SimplifyConfig {
        prefix: match config.simplify.backbone_budget_ms {
            Some(budget_ms) => crate::preprocess::simplify::SimplifyPrefix::Backbone {
                budget_ms,
                equivalence_budget_ms: config.simplify.equivalence_budget_ms,
            },
            None => crate::preprocess::simplify::SimplifyPrefix::EqIter,
        },
        deadline: config.deadline,
        frozen_vars: if purpose == SimplifyPurpose::WeightedCount {
            orig_w.unequal_vars()
        } else {
            rustc_hash::FxHashSet::default()
        },
        ..SimplifyConfig::for_purpose(purpose, /*keep_all_vars=*/ false)
    };
    // The purpose's stage set is the soundness ceiling. Public policy can turn
    // count-only work down or off, never turn it on for `Function`.
    if resolved.stages.gates {
        resolved.stages.gates = config.simplify.detect_gates;
    }
    if resolved.stages.dve.is_some() {
        resolved.stages.dve = config.simplify.dve.map(|dve| DveBudget {
            rounds: dve.rounds,
            budget_ms: dve.budget_ms,
        });
    }
    resolved
}

/// The weight table the file declares, when the mode counts under one:
/// `None` for an unweighted mode, whose count ignores any table the file
/// happens to carry, and for a weighted mode over a file that declares none.
///
/// The one place that pairing is decided. What a chain then does with the
/// table differs on purpose, and the two readings are below: a count needs a
/// weight for every literal, while Arjun's projected entry point must be told
/// only about the ones the file wrote down.
pub(super) fn weight_table(meta: &CnfMeta, mode: Mode) -> Option<&WeightTable> {
    mode.is_weighted()
        .then(|| meta.declared_weights())
        .flatten()
}

/// The instance's literal weights over `num_vars`, unspecified literals
/// defaulting to 1 (the MCC convention); all-ones for an unweighted mode, where
/// every weighted formula in this crate degenerates to its integer counterpart.
pub(super) fn original_weights(meta: &CnfMeta, num_vars: usize, mode: Mode) -> Weights<Original> {
    weight_table(meta, mode).map_or_else(|| Weights::uniform(num_vars), |t| t.resolve(num_vars))
}

/// Create `dir` and any missing parent of it.
///
/// Every directory a bundle needs is created through here, so the failure names
/// the directory and the action it was doing in one voice — and one file, not
/// eight, decides what that voice is.
pub(super) fn ensure_dir(dir: &Path) -> Result<(), VitriError> {
    std::fs::create_dir_all(dir).map_err(|e| VitriError::io(dir, "create", &e))
}

/// Write `contents` to `path`, replacing whatever was there.
///
/// The counterpart of [`ensure_dir`] for the files themselves — every one a
/// bundle emits except `reduced.cnf`, whose writer has a format to enforce as
/// well.
pub(super) fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), VitriError> {
    std::fs::write(path, contents).map_err(|e| VitriError::io(path, "write", &e))
}

/// `value` as the pretty JSON a bundle's own `.json` files are written in.
///
/// Serialization of these types cannot fail: every field is a plain owned value
/// with a derived or hand-written impl that only ever writes, and the writer is
/// a `String`. So there is nothing here for a caller to handle, and no
/// half-written file to explain — the panic would be a bug in this crate's own
/// types.
pub(super) fn to_json_pretty<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("bundle serialization is infallible")
}

impl PreprocessRecord {
    /// Serialize to a pretty JSON string.
    pub fn to_json_string(&self) -> String {
        to_json_pretty(self)
    }

    /// The header lines `reduced.cnf` must carry to describe itself: the track,
    /// the reduced-space show set and the reduced-space weights.
    ///
    /// No `c t` line under `compile`: a `c t` line names a competition track, and
    /// writing `c t compile` would produce a file this crate's own parser rejects.
    /// The mode is in `preprocess.json`'s `mode` either way.
    pub(crate) fn dimacs_header(&self) -> DimacsHeader<'_, Reduced> {
        DimacsHeader {
            track: (self.mode != Mode::Compile).then_some(self.mode.token()),
            show: self.show_vars_reduced_dimacs.as_ref(),
            weights: self.reduced_weights.as_deref(),
        }
    }
}

impl PreprocessBundle {
    /// Write `reduced.cnf` and `preprocess.json` into `dir`, creating it if
    /// needed.
    ///
    /// # Errors
    ///
    /// [`VitriError::Io`] naming the file or directory that could not be
    /// written.
    pub fn write_to_dir(&self, dir: &Path) -> Result<BundlePaths, VitriError> {
        ensure_dir(dir)?;
        let reduced_cnf = dir.join(REDUCED_CNF_NAME);
        let record = dir.join(PREPROCESS_RECORD_NAME);
        write_dimacs(&self.reduced, &self.record.dimacs_header(), &reduced_cnf)?;
        write_file(&record, self.record.to_json_string())?;
        Ok(BundlePaths {
            reduced_cnf,
            record,
        })
    }
}

/// Refuse a build that was not made from `reduced`.
///
/// The vtree, the split and the formula are three separate arguments a caller
/// pairs by hand, and the writers below index the formula by clause and by
/// variable id on the strength of that pairing. What they need is what is
/// checked: every clause a component claims exists, and no two components claim
/// one variable — which together are what makes the manifest name every reduced
/// variable exactly once.
fn check_build_belongs(build: &VtreeBuild, reduced: &CnfFormula) -> Result<(), VitriError> {
    if build.vtree.num_leaves() != reduced.num_vars {
        return Err(VitriError::mismatch(format!(
            "vtree has {} leaves but the formula has {} variables; \
             the build does not belong to this formula",
            build.vtree.num_leaves(),
            reduced.num_vars,
        )));
    }
    let Some(comps) = build.components.as_deref() else {
        return Ok(());
    };
    // Which component claimed each variable, so the second claim on one can
    // name both.
    let mut claimed_by: Vec<Option<usize>> = vec![None; reduced.num_vars as usize];
    for (index, cv) in comps.iter().enumerate() {
        for &ci in &cv.clause_indices {
            let Some(clause) = reduced.clauses.get(ci) else {
                return Err(VitriError::mismatch(format!(
                    "component {index} claims clause {ci} but the formula has {} clauses; \
                     the build does not belong to this formula",
                    reduced.clauses.len(),
                )));
            };
            for lit in &clause.literals {
                let Some(slot) = claimed_by.get_mut(lit.var.idx()) else {
                    return Err(VitriError::mismatch(format!(
                        "clause {ci} names variable {} but the formula declares {} variables",
                        lit.var.to_dimacs(),
                        reduced.num_vars,
                    )));
                };
                match *slot {
                    Some(other) if other != index => {
                        return Err(VitriError::mismatch(format!(
                            "components {other} and {index} both claim variable {}; \
                             the build does not belong to this formula",
                            lit.var.to_dimacs(),
                        )));
                    }
                    _ => *slot = Some(index),
                }
            }
        }
    }
    Ok(())
}

impl VtreeBuild {
    /// Write the vtree half of a bundle into `dir`, creating it if needed:
    /// `vtree.vtree` ([`VTREE_NAME`]), its Graphviz picture when one was asked
    /// for, and the component manifest with the per-component and candidate
    /// files ([`components::write_components`]).
    ///
    /// The counterpart of [`PreprocessBundle::write_to_dir`], and the other half
    /// of what [`VitriRun::write_to_dir`](crate::VitriRun::write_to_dir) writes:
    /// a caller that built a vtree without preprocessing anything exports it
    /// through here rather than reconstructing the file names, the `.dot` naming
    /// convention and the manifest.
    ///
    /// `reduced` is the formula this vtree was built over and `show` its show
    /// set, both as [`components::write_components`] takes them — the pictures
    /// and the per-component scores are read off that pair.
    ///
    /// # Errors
    ///
    /// [`VitriError::Io`] naming the file or directory that could not be
    /// written, and [`VitriError::Mismatch`] for a build that does not belong to
    /// `reduced`.
    pub fn write_to_dir(
        &self,
        dir: &Path,
        reduced: &CnfFormula,
        show: Option<&ShowSet<Reduced>>,
        options: components::ComponentWriteOptions,
    ) -> Result<VtreeFiles, VitriError> {
        // First, so a build that does not belong to `reduced` leaves the
        // caller's directory as it found it.
        check_build_belongs(self, reduced)?;
        ensure_dir(dir)?;
        // The whole-formula vtree's picture, against the formula it was built
        // over and that formula's own show set — the same mask selection scored
        // on.
        let show_mask = show.map(|s| s.mask(reduced.num_vars));
        let dot = DotFor::when(options.dot, reduced, show_mask.as_ref());
        let (vtree, vtree_dot) = write_vtree_files(dir.join(VTREE_NAME), &self.vtree, dot)?;

        // The component manifest is written whatever the split turned out to be
        // — one entry pointing at the files above when the formula is connected
        // — so a consumer reads `components.json` unconditionally.
        let (manifest, paths) = components::write_components(dir, reduced, self, show, options)?;
        assert!(
            components::manifest_matches_vtree(&manifest, &self.vtree),
            "the component manifest and the emitted whole-formula vtree describe different \
             variable spaces",
        );
        Ok(VtreeFiles {
            vtree,
            dot: vtree_dot,
            components: ComponentFiles { manifest, paths },
        })
    }
}

/// The CNF a vtree about to be written serves, carried to the point where its
/// `.dot` sibling is produced. `None` at a call site means no picture is wanted;
/// this exists so no writer has to re-derive a component's formula.
#[derive(Clone, Copy)]
pub(super) struct DotFor<'a> {
    pub formula: &'a CnfFormula,
    /// show-set mask over `formula`'s variables, or `None` when unprojected.
    pub show_mask: Option<&'a crate::cnf::ShowMask>,
}

impl<'a> DotFor<'a> {
    /// The request for a picture of `formula`, or `None` when `wanted` says no
    /// picture was asked for — every caller of [`write_vtree_files`] decides
    /// that the same way, and the vtree file is written either way.
    pub(super) fn when(
        wanted: bool,
        formula: &'a CnfFormula,
        show_mask: Option<&'a crate::cnf::ShowMask>,
    ) -> Option<Self> {
        wanted.then_some(DotFor { formula, show_mask })
    }
}

/// Write one vtree and, when a picture was asked for, its `.dot` sibling beside
/// it — the whole-formula vtree, a component's, and a retained candidate's all
/// go out this way, so a file that appears in a manifest is a file this
/// function wrote.
pub(super) fn write_vtree_files(
    path: PathBuf,
    vtree: &Vtree,
    dot: Option<DotFor<'_>>,
) -> Result<(PathBuf, Option<PathBuf>), VitriError> {
    write_file(&path, vtree.to_vtree_text())?;
    let dot_path = match dot {
        // The picture is the vtree file's sibling: the same path with a `.dot`
        // extension, so naming one in a manifest names the other.
        Some(d) => {
            let dot_path = path.with_extension("dot");
            let ann = dot::annotate_from_cnf(vtree, d.formula, d.show_mask);
            write_file(&dot_path, dot::vtree_to_dot(vtree, Some(&ann)))?;
            Some(dot_path)
        }
        None => None,
    };
    Ok((path, dot_path))
}
