//! Writing a run out: the destination sink, the bundle and vtree writers, and
//! the path and file types they report.

use super::*;

use std::io::Write;

use crate::cnf::write_dimacs;
use crate::dot;
use crate::vtree::Vtree;

/// One file of a bundle held in memory, as [`VitriRun::to_files`] returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleFile {
    /// The file's path relative to the bundle directory, `/`-separated: the
    /// name [`VitriRun::write_to_dir`] gives it, such as `reduced.cnf` or
    /// `components/comp000.vtree`.
    pub path: String,
    /// The bytes [`VitriRun::write_to_dir`] writes to that path.
    pub contents: Vec<u8>,
}

/// Write `files`, as [`VitriRun::to_files`] returned them, under `dir`, the
/// way [`VitriRun::write_to_dir`] would have: directories are created as
/// needed, a file already at one of the paths is replaced, and any other file
/// under `dir` is left alone.
///
/// # Errors
///
/// [`VitriError::Io`] naming the file or directory that could not be written.
pub fn write_files(dir: &Path, files: &[BundleFile]) -> Result<(), VitriError> {
    let mut sink = Sink::at(dir);
    for file in files {
        let parent = file.path.rsplit_once('/').map_or("", |(parent, _)| parent);
        sink.dir(parent)?;
        sink.file(&file.path, |w| w.write_all(&file.contents))?;
    }
    Ok(())
}

/// Paths written by [`PreprocessBundle::write_to_dir`].
#[derive(Debug)]
pub struct BundlePaths {
    /// Where `reduced.cnf` ([`REDUCED_CNF_NAME`]) landed inside the target directory.
    pub reduced_cnf: PathBuf,
    /// Where `preprocess.json` ([`PREPROCESS_RECORD_NAME`]) landed inside the target directory.
    pub record: PathBuf,
}

impl VitriRun {
    /// Write every file this run can name into `dir`: the two halves,
    /// [`PreprocessBundle::write_to_dir`] then
    /// [`VtreeBuild::write_to_dir`](crate::component::VtreeBuild::write_to_dir),
    /// in that order.
    ///
    /// A run that built no vtree writes the bundle alone — there is no vtree to
    /// point a manifest at.
    ///
    /// # Errors
    ///
    /// [`VitriError::Io`] naming the file or directory that could not be
    /// written, and [`VitriError::Mismatch`] for a build that does not belong
    /// to this run's formula.
    pub fn write_to_dir(
        &self,
        dir: &Path,
        options: components::ComponentWriteOptions,
    ) -> Result<RunPaths, VitriError> {
        self.write_to(&mut Sink::at(dir), options)
    }

    /// Every file [`Self::write_to_dir`] writes, held in memory instead: the
    /// same paths and the same bytes, in the order they are written.
    ///
    /// # Errors
    ///
    /// [`VitriError::Mismatch`] for a build that does not belong to this run's
    /// formula.
    pub fn to_files(
        &self,
        options: components::ComponentWriteOptions,
    ) -> Result<RunFiles, VitriError> {
        let mut files = Vec::new();
        let paths = self.write_to(&mut Sink::in_memory(&mut files), options)?;
        Ok(RunFiles { files, paths })
    }

    /// [`Self::write_to_dir`] into either destination a [`Sink`] names.
    pub(crate) fn write_to(
        &self,
        sink: &mut Sink<'_>,
        options: components::ComponentWriteOptions,
    ) -> Result<RunPaths, VitriError> {
        let bundle = self.preprocessed.write_to(sink)?;
        let Some(build) = self.built() else {
            return Ok(RunPaths {
                bundle,
                vtree: None,
            });
        };
        let vtree = build.write_to(
            sink,
            &self.preprocessed.reduced,
            self.preprocessed.record.show_vars_reduced_dimacs.as_ref(),
            options,
        )?;
        Ok(RunPaths {
            bundle,
            vtree: Some(vtree),
        })
    }
}

/// What [`VitriRun::to_files`] produced.
#[derive(Debug)]
pub struct RunFiles {
    /// Every file of the bundle, in the order it was written.
    pub files: Vec<BundleFile>,
    /// What [`VitriRun::write_to_dir`] reports for the same run — the component
    /// manifest included — with every path relative to the bundle directory.
    pub paths: RunPaths,
}

/// What [`VitriRun::write_to_dir`] wrote.
#[derive(Debug)]
pub struct RunPaths {
    /// `reduced.cnf` and `preprocess.json`, which every run writes.
    pub bundle: BundlePaths,
    /// The vtree files, absent for a run that built no vtree — the same
    /// distinction [`RunVtree`] draws, and drawn once here rather than repeated
    /// across the files that arrive together.
    pub vtree: Option<VtreeFiles>,
}

/// What the vtree half of a run wrote: the vtree itself, its picture, and the
/// component split underneath it.
#[derive(Debug)]
pub struct VtreeFiles {
    /// Where `vtree.vtree` ([`VTREE_NAME`]) landed.
    pub vtree: PathBuf,
    /// Its Graphviz picture, present only when
    /// [`ComponentWriteOptions::dot`](components::ComponentWriteOptions::dot)
    /// asked for one.
    pub dot: Option<PathBuf>,
    /// The component manifest and the files it names, written whatever the
    /// split turned out to be.
    pub components: ComponentFiles,
}

/// The component split as written: what the manifest says, and where its files
/// landed.
#[derive(Debug)]
pub struct ComponentFiles {
    /// The manifest, as written to `components.json`.
    pub manifest: components::ComponentsManifest,
    /// Where the manifest and the files it names landed.
    pub paths: components::ComponentPaths,
}

/// Create `dir` and any missing parent of it.
///
/// Every directory a bundle needs is created through here, so the failure names
/// the directory and the action it was doing in one voice — and one file, not
/// eight, decides what that voice is.
pub(super) fn ensure_dir(dir: &Path) -> Result<(), VitriError> {
    std::fs::create_dir_all(dir).map_err(|e| VitriError::io(dir, "create", &e))
}

/// Where a bundle's files go: a directory on disk, or a list held in memory.
///
/// Every writer in this module and in [`components`] writes through one, so the
/// two destinations receive the same bytes from the same serializers, and each
/// file's name is decided once whichever destination it is. The sink also keeps
/// the order it wrote them in, which is the order a run reports its files.
pub(crate) struct Sink<'a> {
    to: Destination<'a>,
    written: Vec<String>,
}

enum Destination<'a> {
    /// Files under this directory, which is created as needed.
    Dir(&'a Path),
    /// Files appended here in the order they are written.
    Memory(&'a mut Vec<BundleFile>),
}

impl<'a> Sink<'a> {
    /// Files under `root`, which is created as needed.
    pub(crate) fn at(root: &'a Path) -> Self {
        Sink {
            to: Destination::Dir(root),
            written: Vec::new(),
        }
    }

    /// Files appended to `files`, in the order they are written.
    pub(crate) fn in_memory(files: &'a mut Vec<BundleFile>) -> Self {
        Sink {
            to: Destination::Memory(files),
            written: Vec::new(),
        }
    }

    /// Every file written through this sink, in order, each relative to the
    /// bundle root and `/`-separated: the names a run reports whichever
    /// destination it wrote to.
    pub(crate) fn written(&self) -> &[String] {
        &self.written
    }

    /// Make sure the directory `rel` exists below the sink's root; `""` is the
    /// root itself. In memory there is nothing to create.
    pub(crate) fn dir(&mut self, rel: &str) -> Result<(), VitriError> {
        match &self.to {
            Destination::Dir(root) => ensure_dir(&root.join(rel)),
            Destination::Memory(_) => Ok(()),
        }
    }

    /// Write the file `rel` — relative to the sink's root, `/`-separated — with
    /// the bytes `emit` produces, replacing whatever was there.
    ///
    /// Returns where the file landed: its path on disk, or `rel` itself for a
    /// sink in memory.
    pub(crate) fn file(
        &mut self,
        rel: &str,
        emit: impl FnOnce(&mut dyn Write) -> std::io::Result<()>,
    ) -> Result<PathBuf, VitriError> {
        let path = match &mut self.to {
            Destination::Dir(root) => {
                let path = root.join(rel);
                let written = std::fs::File::create(&path).and_then(|file| {
                    let mut w = std::io::BufWriter::new(file);
                    emit(&mut w)?;
                    // Dropping the writer would flush too, and discard the error
                    // a full disk reports there, leaving a truncated file and an
                    // `Ok`. Flush while the failure can still be returned.
                    w.flush()
                });
                written.map_err(|e| VitriError::io(&path, "write", &e))?;
                path
            }
            Destination::Memory(files) => {
                let mut contents = Vec::new();
                emit(&mut contents).map_err(|e| VitriError::io(rel, "write", &e))?;
                files.push(BundleFile {
                    path: rel.to_string(),
                    contents,
                });
                PathBuf::from(rel)
            }
        };
        self.written.push(rel.to_string());
        Ok(path)
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
        self.write_to(&mut Sink::at(dir))
    }

    /// [`Self::write_to_dir`] into either destination a [`Sink`] names.
    pub(super) fn write_to(&self, sink: &mut Sink<'_>) -> Result<BundlePaths, VitriError> {
        sink.dir("")?;
        let reduced_cnf = sink.file(REDUCED_CNF_NAME, |w| {
            write_dimacs(&self.reduced, &self.record.dimacs_header(), w)
        })?;
        let record = sink.file(PREPROCESS_RECORD_NAME, |w| {
            w.write_all(self.record.to_json_string().as_bytes())
        })?;
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
/// checked: every clause a component claims exists, no two components claim one
/// variable (which together are what makes the manifest name every reduced
/// variable exactly once), each component vtree spans the variables its clauses
/// cut out, and its `local_to_outer` is the numbering those clauses give.
///
/// Every check is here and none in the writers, so nothing is written before
/// the whole pairing has been accepted.
fn check_build_belongs(build: &VtreeBuild, reduced: &CnfFormula) -> Result<(), VitriError> {
    if build.vtree.num_leaves() != reduced.num_vars() {
        return Err(VitriError::mismatch(format!(
            "vtree has {} leaves but the formula has {} variables; \
             the build does not belong to this formula",
            build.vtree.num_leaves(),
            reduced.num_vars(),
        )));
    }
    let Some(comps) = build.components.as_deref() else {
        return Ok(());
    };
    // Which component claimed each variable, so the second claim on one can
    // name both.
    let mut claimed_by: Vec<Option<usize>> = vec![None; reduced.num_vars() as usize];
    for (index, cv) in comps.iter().enumerate() {
        // Before `component_vars` below, which indexes the formula by these.
        for &ci in &cv.clause_indices {
            let Some(clause) = reduced.clauses().get(ci) else {
                return Err(VitriError::mismatch(format!(
                    "component {index} claims clause {ci} but the formula has {} clauses; \
                     the build does not belong to this formula",
                    reduced.clauses().len(),
                )));
            };
            for lit in &clause.literals {
                let Some(slot) = claimed_by.get_mut(lit.var.idx()) else {
                    return Err(VitriError::mismatch(format!(
                        "clause {ci} names variable {} but the formula declares {} variables",
                        lit.var.to_dimacs(),
                        reduced.num_vars(),
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

        let local_to_outer = reduced.component_vars(&cv.clause_indices);
        if cv.vtree.num_leaves() != local_to_outer.len() as u32 {
            return Err(VitriError::mismatch(format!(
                "component {index} vtree has {} leaves but its CNF has {} variables; \
                 the build does not belong to this formula",
                cv.vtree.num_leaves(),
                local_to_outer.len(),
            )));
        }
        if cv.local_to_outer != local_to_outer {
            let disagreement = cv
                .local_to_outer
                .iter()
                .zip(&local_to_outer)
                .position(|(named, cut)| named != cut);
            let detail = match disagreement {
                Some(local) => format!(
                    "local {} is named as variable {} and the clauses put variable {} there",
                    VarId::from_idx(local).to_dimacs(),
                    cv.local_to_outer[local].to_dimacs(),
                    local_to_outer[local].to_dimacs(),
                ),
                None => format!(
                    "the map names {} variables and the clauses cut out {}",
                    cv.local_to_outer.len(),
                    local_to_outer.len(),
                ),
            };
            return Err(VitriError::mismatch(format!(
                "component {index}'s local-to-outer map is not the numbering its clauses \
                 give ({detail}); the build does not belong to this formula",
            )));
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
        self.write_to(&mut Sink::at(dir), reduced, show, options)
    }

    /// [`Self::write_to_dir`] into either destination a [`Sink`] names.
    pub(super) fn write_to(
        &self,
        sink: &mut Sink<'_>,
        reduced: &CnfFormula,
        show: Option<&ShowSet<Reduced>>,
        options: components::ComponentWriteOptions,
    ) -> Result<VtreeFiles, VitriError> {
        // First, so a build that does not belong to `reduced` leaves the
        // caller's directory as it found it.
        check_build_belongs(self, reduced)?;
        sink.dir("")?;
        // The whole-formula vtree's picture, against the formula it was built
        // over and that formula's own show set — the same mask selection scored
        // on.
        let show_mask = show.map(|s| s.mask(reduced.num_vars()));
        let dot = DotFor::when(options.dot, reduced, show_mask.as_ref());
        let (vtree, vtree_dot) = write_vtree_files(sink, VTREE_NAME, &self.vtree, dot)?;

        // The component manifest is written whatever the split turned out to be
        // — one entry pointing at the files above when the formula is connected
        // — so a consumer reads `components.json` unconditionally.
        let components = components::write_components_to(sink, reduced, self, show, options)?;
        assert!(
            components::manifest_matches_vtree(&components.manifest, &self.vtree),
            "the component manifest and the emitted whole-formula vtree describe different \
             variable spaces",
        );
        Ok(VtreeFiles {
            vtree,
            dot: vtree_dot,
            components,
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
    sink: &mut Sink<'_>,
    rel: &str,
    vtree: &Vtree,
    dot: Option<DotFor<'_>>,
) -> Result<(PathBuf, Option<PathBuf>), VitriError> {
    let path = sink.file(rel, |w| w.write_all(vtree.to_vtree_text().as_bytes()))?;
    let dot_path = match dot {
        // The picture is the vtree file's sibling: the same path with a `.dot`
        // extension, so naming one in a manifest names the other.
        Some(d) => {
            let dot_rel = Path::new(rel).with_extension("dot");
            let ann = dot::annotate_from_cnf(vtree, d.formula, d.show_mask);
            let text = dot::vtree_to_dot(vtree, Some(&ann));
            Some(sink.file(&dot_rel.to_string_lossy(), |w| w.write_all(text.as_bytes()))?)
        }
        None => None,
    };
    Ok((path, dot_path))
}
