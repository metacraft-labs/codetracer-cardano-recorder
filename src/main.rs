//! CLI entry point for the CodeTracer Aiken/Cardano recorder.
//!
//! Supports the `record` subcommand which takes an Aiken source file,
//! parses and evaluates variable assignments, and writes a CodeTracer
//! CTFS trace bundle.
//!
//! # Usage
//!
//! ```text
//! codetracer-cardano-recorder record <aiken-file> --out-dir <output-dir>
//! ```
//!
//! The recorder always writes traces in the canonical CodeTracer multi-stream
//! CTFS format (see `Recorder-CLI-Conventions.md` §4 in `codetracer-specs`).
//! No `--format` flag is exposed: human-readable conversion is handled
//! out-of-band by `ct print` (shipped with `codetracer-trace-format-nim`).
//!
//! # Environment variables
//!
//! * `CODETRACER_CARDANO_RECORDER_OUT_DIR` — fallback for `--out-dir` when the
//!   flag is not given. The CLI flag always wins.
//! * `CODETRACER_CARDANO_RECORDER_DISABLED` — set to `1` or `true` to skip
//!   recording entirely. The recorder still executes the target subcommand
//!   (where applicable) and propagates its exit code.
//! * `CODETRACER_CARDANO_RECORDER_LOG_LEVEL` — recorder log verbosity (advisory;
//!   the Cardano recorder currently logs to stderr unconditionally).
//! * `BLOCKFROST_API_KEY` — Blockfrost API key used by the `replay` subcommand
//!   (Cardano-specific; not part of the standard recorder env-var set).

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use eyre::{Context, Result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Environment variable used as a fallback for `--out-dir` when the CLI
/// flag is omitted.  Convention: see `Recorder-CLI-Conventions.md` §5.
const ENV_OUT_DIR: &str = "CODETRACER_CARDANO_RECORDER_OUT_DIR";

/// Environment variable that, when set to `1`/`true`, disables tracing
/// entirely — the recorder runs as a transparent pass-through.
const ENV_DISABLED: &str = "CODETRACER_CARDANO_RECORDER_DISABLED";

/// Default output directory used when neither `--out-dir` nor
/// `CODETRACER_CARDANO_RECORDER_OUT_DIR` is set.
const DEFAULT_OUT_DIR: &str = "./ct-traces/";

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

/// CodeTracer Aiken/Cardano recorder -- record Aiken smart contract execution traces.
///
/// Traces are always written in the canonical CTFS multi-stream format.
/// To convert a recorded `.ct` bundle to JSON / text for inspection, use
/// `ct print` from `codetracer-trace-format-nim`.
#[derive(Debug, Parser)]
#[command(
    name = "codetracer-cardano-recorder",
    version,
    about = "Record Aiken smart contract execution traces for CodeTracer (CTFS-only). \
             Use `ct print` from codetracer-trace-format-nim for human-readable conversion.",
    long_about = "Record Aiken smart contract execution traces for CodeTracer.\n\
                  \n\
                  Output is always written in the canonical CodeTracer CTFS\n\
                  multi-stream format. Use `ct print` (shipped with the\n\
                  codetracer-trace-format-nim sibling) to convert a recorded\n\
                  `.ct` bundle to JSON or other human-readable forms.\n\
                  \n\
                  Environment variables:\n\
                    CODETRACER_CARDANO_RECORDER_OUT_DIR    fallback for --out-dir\n\
                    CODETRACER_CARDANO_RECORDER_DISABLED   set to 1/true to skip recording\n\
                    CODETRACER_CARDANO_RECORDER_LOG_LEVEL  log verbosity (advisory)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Record execution of an Aiken program.
    ///
    /// Parses the given .ak source file, evaluates variable assignments,
    /// captures the execution trace, and writes a CTFS bundle to
    /// `--out-dir`.
    Record(RecordArgs),

    /// Replay a Plutus validator from an on-chain Cardano transaction.
    ///
    /// Fetches the transaction via the Blockfrost API, extracts the Plutus
    /// script and its arguments (datum, redeemer, script context), and
    /// traces the validator execution through the UPLC CEK machine.
    Replay(ReplayArgs),

    /// Print version information.
    Version,
}

#[derive(Debug, clap::Args)]
struct ReplayArgs {
    /// Cardano transaction hash (64 hex characters).
    #[arg(long)]
    tx_hash: String,

    /// Blockfrost API key. If not provided, the BLOCKFROST_API_KEY
    /// environment variable is used.
    #[arg(long)]
    blockfrost_key: Option<String>,

    /// Directory where the trace files will be written.
    ///
    /// Falls back to `CODETRACER_CARDANO_RECORDER_OUT_DIR` when omitted.
    #[arg(short = 'o', long)]
    out_dir: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
struct RecordArgs {
    /// Path to the Aiken source (.ak) file.
    program: PathBuf,

    /// Directory where the trace files will be written.
    ///
    /// The directory will be created if it does not exist.  Falls back to
    /// the `CODETRACER_CARDANO_RECORDER_OUT_DIR` environment variable when the
    /// flag is omitted.
    #[arg(short = 'o', long)]
    out_dir: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the effective output directory:
///   1. `--out-dir` if given on the CLI.
///   2. `CODETRACER_CARDANO_RECORDER_OUT_DIR` env var.
///   3. `DEFAULT_OUT_DIR` ("./ct-traces/").
fn resolve_out_dir(cli_out_dir: Option<PathBuf>) -> PathBuf {
    if let Some(path) = cli_out_dir {
        return path;
    }
    if let Some(value) = std::env::var_os(ENV_OUT_DIR) {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    PathBuf::from(DEFAULT_OUT_DIR)
}

/// Whether the recorder is disabled via env var.  When true, the CLI
/// must execute its target operation in pass-through mode without
/// emitting any trace artefacts.
fn recording_disabled() -> bool {
    match std::env::var(ENV_DISABLED) {
        Ok(value) => {
            let v = value.trim();
            v == "1" || v.eq_ignore_ascii_case("true")
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Record(args) => record(args),
        Commands::Replay(args) => replay(args),
        Commands::Version => {
            println!("codetracer-cardano-recorder {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// `record` implementation
// ---------------------------------------------------------------------------

/// Execute the `record` subcommand.
fn record(args: RecordArgs) -> Result<()> {
    // 1. Validate the source file exists
    let source_path = args
        .program
        .canonicalize()
        .with_context(|| format!("source file not found: {}", args.program.display()))?;

    eprintln!("Source file: {}", source_path.display());

    if recording_disabled() {
        // Pass-through: the Cardano recorder doesn't run a separate target
        // process — it parses & evaluates the Aiken source itself — so
        // disabling recording simply means "don't emit any trace artefacts".
        eprintln!("{ENV_DISABLED} is set; skipping trace recording (no output written).");
        return Ok(());
    }

    // 2. Resolve and create the output directory
    let out_dir = resolve_out_dir(args.out_dir);
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("cannot create output dir: {}", out_dir.display()))?;

    // 3. Run the recorder (CTFS only)
    codetracer_cardano_recorder::recorder::record(&source_path, &out_dir)?;

    eprintln!("Trace files written to {}", out_dir.display());

    Ok(())
}

// ---------------------------------------------------------------------------
// `replay` implementation
// ---------------------------------------------------------------------------

/// Execute the `replay` subcommand.
///
/// Fetches the transaction via Blockfrost, reconstructs the fully-applied
/// UPLC program, evaluates it through the CEK machine, and writes a
/// CodeTracer trace.
fn replay(args: ReplayArgs) -> Result<()> {
    use codetracer_cardano_recorder::blockfrost::BlockfrostClient;
    use codetracer_cardano_recorder::transaction::reconstruct_applied_program;
    use uplc::machine::cost_model::ExBudget;

    eprintln!("Replaying transaction: {}", args.tx_hash);

    if recording_disabled() {
        eprintln!("{ENV_DISABLED} is set; skipping replay (no output written).");
        return Ok(());
    }

    // The replay subcommand does not yet write CTFS output, but we
    // resolve the output dir up-front so callers can rely on the
    // env-var fallback once writer support lands.
    let _out_dir = resolve_out_dir(args.out_dir);

    // 1. Build the Blockfrost client.
    let client = if let Some(key) = args.blockfrost_key {
        BlockfrostClient::new(key)
    } else {
        BlockfrostClient::from_env()?
    };

    // 2. Fetch the transaction data.
    let tx_data = client.fetch_transaction(&args.tx_hash)?;
    eprintln!(
        "Fetched {} script ({} bytes)",
        tx_data.script_version,
        tx_data.script_bytes.len()
    );

    // 3. Reconstruct the fully-applied program.
    let program = reconstruct_applied_program(&tx_data)?;
    eprintln!("Reconstructed applied program");

    // 4. Evaluate through the CEK machine.
    let language = tx_data.script_version.to_language();
    let result = program.eval_version(ExBudget::default(), &language);
    match result.result() {
        Ok(term) => {
            eprintln!("Evaluation succeeded");
            println!("{term}");
        }
        Err(e) => {
            eprintln!("Evaluation failed: {e:?}");
            return Err(eyre::eyre!("UPLC evaluation error: {e:?}"));
        }
    }

    eprintln!(
        "Budget spent: {} CPU, {} MEM",
        result.cost().cpu,
        result.cost().mem
    );

    Ok(())
}
