//! `vitri` — turn a raw CNF into a reduced CNF plus a good vtree, for a
//! circuit compiler or model counter to consume.
//!
//! This binary is a thin shell. The flags parsed here become one
//! [`Request`](vitri::request::Request), the same value the language bindings
//! build, and the run itself is
//! [`vitri::request::prepare_to_dir`](vitri::request::prepare_to_dir): the one
//! entry point that preprocesses, builds the vtrees and writes the bundle.
//! Anything reachable from the command line is reachable from the API.
//!
//! See `docs/bundle.md` for the output-file contract, `docs/preprocessing.md`
//! for what the consumer is responsible for, and `docs/env.md` for the `VITRI_*`
//! variables this binary reads.

#[path = "cli_main/help.rs"]
mod help;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::exit;

use help::help;
use vitri::VitriError;
use vitri::bundle;
use vitri::bundle::components::ComponentWriteOptions;
use vitri::candidates;
use vitri::cnf::CnfFormula;
use vitri::config::RunConfig;
use vitri::decompose::SelectionCtx;
use vitri::request::{self, Request};

/// Which option a row of [`OPTIONS`] is.
///
/// The parser dispatches on this, exhaustively, so a row added without an arm
/// to answer it is a build error rather than a flag `--help` offers and the
/// parser rejects.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OptKey {
    OutDir,
    Mode,
    Vtree,
    BudgetMs,
    Components,
    Candidates,
    Dot,
    NoArjun,
    NoSimplify,
    Help,
    Version,
}

/// One option the command line takes: how it is spelled, whether it carries a
/// value, and — through [`OptKey::blurb`] — what `--help` says about it.
struct Opt {
    key: OptKey,
    /// The short spelling, for the options that have one.
    short: Option<&'static str>,
    /// The long spelling, which is also how a message names the option.
    long: &'static str,
    /// The value placeholder `--help` prints after the spellings; `None` for a
    /// switch that takes no value.
    value: Option<&'static str>,
}

/// Every option, in the order `--help` lists them.
///
/// Read twice: [`help`] renders the `OPTIONS:` block from it, and [`parse_argv`]
/// resolves each argument against it before dispatching. So an option that is
/// not here is neither offered nor accepted, and the help text cannot describe a
/// grammar the parser does not have.
const OPTIONS: &[Opt] = &[
    Opt {
        key: OptKey::OutDir,
        short: Some("-o"),
        long: "--out-dir",
        value: Some("<DIR>"),
    },
    Opt {
        key: OptKey::Mode,
        short: None,
        long: "--mode",
        value: Some("<MODE>"),
    },
    Opt {
        key: OptKey::Vtree,
        short: None,
        long: "--vtree",
        value: Some("<SPEC>"),
    },
    Opt {
        key: OptKey::BudgetMs,
        short: None,
        long: "--budget-ms",
        value: Some("<N>"),
    },
    Opt {
        key: OptKey::Components,
        short: None,
        long: "--components",
        value: Some("<MODE>"),
    },
    Opt {
        key: OptKey::Candidates,
        short: None,
        long: "--candidates",
        value: Some("<N>"),
    },
    Opt {
        key: OptKey::Dot,
        short: None,
        long: "--dot",
        value: None,
    },
    Opt {
        key: OptKey::NoArjun,
        short: None,
        long: "--no-arjun",
        value: None,
    },
    Opt {
        key: OptKey::NoSimplify,
        short: None,
        long: "--no-simplify",
        value: None,
    },
    Opt {
        key: OptKey::Help,
        short: Some("-h"),
        long: "--help",
        value: None,
    },
    Opt {
        key: OptKey::Version,
        short: Some("-V"),
        long: "--version",
        value: None,
    },
];

impl Opt {
    /// Whether `arg` is this option, under either spelling.
    fn matches(&self, arg: &str) -> bool {
        self.long == arg || self.short == Some(arg)
    }
}

/// The command line, split into where the files go, what else gets written
/// there, and how the run is configured. Everything in the last part is a
/// [`RunConfig`] or [`SelectionCtx`] field — those flags do not carry defaults of
/// their own, they edit the two configs whose production settings the tool
/// starts from. `write` is not among them: it shapes the OUTPUT, not the
/// preprocessing or the construction.
///
/// This is the program the `VITRI_*` research knobs are for, so it starts from
/// the env-filled configs rather than from `Default`; a caller EMBEDDING the
/// library gets `Default` and is never reconfigured behind its back by the
/// shell that launched it.
struct Args {
    input: PathBuf,
    out_dir: PathBuf,
    /// The writer's own options, as [`Request::write_options`] builds them, so
    /// the tool and a request asking for the same extras get the same files.
    write: ComponentWriteOptions,
    config: RunConfig,
    /// The construction knobs, already filled from the environment. Selection
    /// mode is decided later, from the reduced instance's own show set, on top
    /// of this value.
    selection: SelectionCtx,
}

/// The command line as typed, before anything outside it is consulted: which
/// flags appeared, and with what values.
///
/// A request field left `None` means the flag was not given, so whatever the
/// environment-filled configuration already holds stands. Keeping the two apart
/// is what lets `--help` answer while a `VITRI_*` variable in the caller's shell
/// is malformed: the loop that fills this reads nothing but `argv`.
#[derive(Default)]
struct Options {
    input: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    request: Request,
}

/// The argument grammar, as a function of the argument vector. `--help` and
/// `--version` are the two arguments whose whole effect is to print and exit,
/// so they do that here, out of a loop that has read no environment variable
/// yet; everything else comes back as a value, an error included.
///
/// The environment is read once the whole command line is in hand, and the
/// flags are applied on top of it. A `VITRI_*` variable is therefore reported
/// after — not instead of — a bad argument on the same line.
fn parse_argv(argv: &[String]) -> Result<Args, VitriError> {
    let mut opts = Options::default();

    let next = |i: &mut usize, flag: &str| -> Result<String, VitriError> {
        *i += 1;
        argv.get(*i)
            .cloned()
            .ok_or_else(|| VitriError::config(format!("{flag} needs a value")))
    };

    let mut i = 1;
    while i < argv.len() {
        let arg = argv[i].as_str();
        // The inventory decides what a spelling means; everything left over is
        // either the input CNF or a typo. A message about an option names it by
        // its long spelling, whichever one was typed.
        let Some(opt) = OPTIONS.iter().find(|o| o.matches(arg)) else {
            if arg.starts_with('-') {
                return Err(VitriError::config(format!("unknown option {arg:?}")));
            }
            if opts.input.replace(PathBuf::from(arg)).is_some() {
                return Err(VitriError::config("expected exactly one input CNF"));
            }
            i += 1;
            continue;
        };
        match opt.key {
            OptKey::Help => {
                print!("{}", help());
                exit(0);
            }
            OptKey::Version => {
                println!("vitri {}", request::VERSION);
                exit(0);
            }
            OptKey::OutDir => opts.out_dir = Some(PathBuf::from(next(&mut i, opt.long)?)),
            OptKey::Mode => {
                let v = next(&mut i, opt.long)?;
                opts.request.mode = Some(request::parse_mode(opt.long, &v)?);
            }
            OptKey::Vtree => opts.request.vtree = Some(next(&mut i, opt.long)?),
            OptKey::BudgetMs => {
                let v = next(&mut i, opt.long)?;
                opts.request.budget_ms = Some(request::parse_integer(opt.long, &v)?);
            }
            OptKey::Components => {
                let v = next(&mut i, opt.long)?;
                opts.request.components = Some(request::parse_components(opt.long, &v)?);
            }
            OptKey::Candidates => {
                let v = next(&mut i, opt.long)?;
                opts.request.candidates = Some(request::parse_integer(opt.long, &v)?);
            }
            // No inert-combination guard: every mode emits at least one vtree
            // when there is anything to build one over, so `--dot` always means
            // something.
            OptKey::Dot => opts.request.dot = true,
            // Whether the stage these turn off is one the run's mode even has
            // cannot be settled here — the mode may still be coming from the
            // instance's own headers — so the flag only records the request, and
            // `RunConfig` judges it against the mode that ends up running.
            OptKey::NoArjun => opts.request.arjun = Some(false),
            OptKey::NoSimplify => opts.request.simplify = Some(false),
        }
        i += 1;
    }

    // The whole command line is in hand, so the environment can be read: each
    // flag above EDITS the two env-filled configs, and a flag that was not
    // given leaves the variable's value — or the production default — in place.
    let mut config = RunConfig::from_env_defaults()?;
    let selection = SelectionCtx::plain().with_env_defaults()?;
    opts.request.apply_to(&mut config);

    // The inert/out-of-range combinations are the config's own to judge —
    // `--candidates` above a construction that builds one vtree, or above the
    // retention ceiling — so they are checked by the ONE validator a library
    // consumer also gets, not by a second set of rules spelled here.
    config.validate()?;

    Ok(Args {
        input: opts
            .input
            .ok_or_else(|| VitriError::config("no input CNF given"))?,
        out_dir: opts
            .out_dir
            .ok_or_else(|| VitriError::config("--out-dir is required"))?,
        write: opts.request.write_options(),
        config,
        selection,
    })
}

/// The one place a failure becomes a process exit. Everything below `run` —
/// argument parsing, the library, this binary's own file writes — reports by
/// returning a [`VitriError`], so there is a single message-and-status rule
/// instead of one per call site.
fn main() {
    // This is the crate's own binary, not a library consumer — opt in to the
    // library's diagnostic chatter (`vitri::diagnostics`, default quiet) so all
    // existing binary output is preserved.
    vitri::diagnostics::set_verbose(true);

    if let Err(e) = run() {
        eprintln!("error: {e}");
        if let Some(hint) = where_to_look(&e) {
            eprintln!("{hint}");
        }
        exit(exit_status(&e));
    }
}

/// 2 when the invocation itself is wrong — a bad argument, a flag combination
/// that would do nothing, a `VITRI_*` variable set to a value this crate cannot
/// use — and nothing ran. 1 when the invocation was fine and the work failed.
fn exit_status(e: &VitriError) -> i32 {
    match e {
        VitriError::Config { .. } | VitriError::Spec { .. } | VitriError::Env { .. } => 2,
        // A variant this binary does not know is reported as work that failed:
        // an error class it cannot recognize is not one it can blame on the
        // command line.
        _ => 1,
    }
}

/// The one-line pointer printed under the message, or `None` when the message
/// stands on its own.
///
/// Each error class is sent where its answer actually is. `--help` documents the
/// command line, so it answers a bad argument or an inert combination; it says
/// nothing about the `VITRI_*` variables, so an environment error points at the
/// file that lists them instead. A run that started and then failed has no
/// documentation to be sent to.
fn where_to_look(e: &VitriError) -> Option<&'static str> {
    match e {
        VitriError::Config { .. } | VitriError::Spec { .. } => Some("run `vitri --help` for usage"),
        VitriError::Env { .. } => Some("the supported variables are listed in docs/env.md"),
        _ => None,
    }
}

fn run() -> Result<(), VitriError> {
    let argv: Vec<String> = std::env::args().collect();
    let args = parse_argv(&argv)?;
    let started = std::time::Instant::now();

    let file = File::open(&args.input).map_err(|e| VitriError::io(&args.input, "open", &e))?;
    let reader = BufReader::new(file);
    let (formula, meta) = CnfFormula::from_dimacs(reader)
        .map_err(|e| VitriError::input(format!("parsing {}: {e}", args.input.display())))?;

    // ── The whole pipeline: preprocess, the vtree over what is left, and the
    // bundle on disk ────────────────────────────────────────────────────────
    let (summary, paths) = request::prepare_to_dir(
        &formula,
        &meta,
        &args.config,
        &args.selection,
        &args.out_dir,
        args.write,
    )?;

    print_run_report(
        &args,
        &summary,
        paths.vtree.as_ref().map(|v| &v.components.manifest),
    );
    print_written(&paths, &args.out_dir);
    println!("elapsed:      {} ms", started.elapsed().as_millis());
    Ok(())
}

/// The `s` a count needs: nothing for one of something, `s` for none or many.
fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// What a successful run found, from the [`Summary`](request::Summary) the
/// library already derived: what went in, what preprocessing left, the vtree
/// over it, and the component split underneath.
///
/// The three shapes a run can take get a `reduced:` and a `vtree:` line each.
/// Preprocessing can leave a formula to compile, resolve every variable —
/// forced, determined, or folded into the multiplier, so `count(reduced)` is 1
/// by definition and `count(original)` is the lift — or refute the instance, so
/// the count is 0. Only the first has a vtree; for the other two the record is
/// the whole answer, and emitting a vtree for either would describe work that
/// was not needed. The rest of the report is the same lines whichever shape it
/// is, with the sections that describe a vtree simply absent.
///
/// The candidate block is the one part not in the summary: it names each
/// component's runners-up, which only the manifest lists.
fn print_run_report(
    args: &Args,
    summary: &request::Summary,
    components: Option<&bundle::components::ComponentsManifest>,
) {
    println!(
        "input:        {} ({} vars, {} clauses, mode {})",
        args.input.display(),
        summary.input.variables,
        summary.input.clauses,
        summary.mode,
    );
    // A summary carries a vtree exactly when the run built one, so the vtree
    // itself decides the first arm and the status tells the other two apart.
    match (&summary.vtree, summary.status) {
        (Some(vtree), _) => {
            println!(
                "reduced:      {} vars, {} clauses  (count(original) = count(reduced) * {})",
                summary.reduced.variables, summary.reduced.clauses, summary.lift.factor,
            );
            println!(
                "vtree:        {} ({} leaves, {} nodes)",
                args.config.vtree_spec, vtree.leaves, vtree.nodes,
            );
        }
        (None, request::RunStatus::Refuted) => {
            println!("unsat:        preprocessing refuted the instance; count(original) = 0");
            println!(
                "reduced:      an explicit contradiction over {} vars",
                summary.reduced.variables,
            );
            println!("vtree:        none (the count is already 0)");
        }
        (None, _) => {
            println!(
                "reduced:      0 vars — fully resolved, count(original) = {}",
                summary.lift.factor,
            );
            println!("vtree:        none (no variables to build one over)");
        }
    }
    if let Some(manifest) = components {
        println!(
            "components:   {} ({} free variable{})",
            manifest.components.len(),
            manifest.free_vars_reduced_dimacs.len(),
            plural(manifest.free_vars_reduced_dimacs.len()),
        );
        // The candidate set, when one was asked for. Printed per component rather than
        // summed: how many DISTINCT vtrees a component's portfolio produced is the
        // number worth seeing — it is often below `--candidates` because specs
        // converge on the same tree, and it is zero for a component small enough
        // that the portfolio never ran on it.
        if candidates::retains_set(args.config.candidates) {
            match manifest.candidate_rank_metric {
                Some(metric) => {
                    let metric = metric.as_str();
                    println!("candidates:   ranked by {metric} (ascending — lower is better)")
                }
                None => {
                    println!(
                        "candidates:   no component was big enough to build a candidate set for"
                    )
                }
            }
            for (index, c) in manifest.components.iter().enumerate() {
                let n = c.vtree_candidates.len();
                let detail = if n == 0 {
                    "no candidate set — built directly, one candidate".to_string()
                } else {
                    format!(
                        "{n} distinct vtree{}: {}",
                        plural(n),
                        c.vtree_candidates
                            .iter()
                            .enumerate()
                            .map(|(rank, e)| format!("#{rank} {}", e.built_by.join("=")))
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                };
                println!("              component {index:03}: {detail}");
            }
        }
    }
}

/// Every file the run wrote, one per line under a single `wrote:` label.
///
/// The two directories a component split fills are named with a count rather
/// than listed: one line for a hundred component files is what a reader can use,
/// and the manifest above them is what names each.
fn print_written(paths: &bundle::RunPaths, out_dir: &std::path::Path) {
    let dir_line = |dir: &str, n: usize| {
        if n > 0 {
            println!("              {}/ ({n} files)", out_dir.join(dir).display());
        }
    };
    println!("wrote:        {}", paths.bundle.reduced_cnf.display());
    println!("              {}", paths.bundle.record.display());
    if let Some(vtree) = &paths.vtree {
        for p in std::iter::once(&vtree.vtree).chain(vtree.dot.iter()) {
            println!("              {}", p.display());
        }
        let comp_paths = &vtree.components.paths;
        println!("              {}", comp_paths.manifest.display());
        dir_line(bundle::components::COMPONENTS_DIR, comp_paths.files.len());
        dir_line(
            bundle::components::CANDIDATES_DIR,
            comp_paths.candidates.len(),
        );
    }
}
