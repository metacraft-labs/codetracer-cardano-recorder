//! CLI entry point for the CodeTracer Aiken/Cardano recorder.
//!
//! Supports the `record` subcommand which takes an Aiken source file,
//! parses and evaluates variable assignments, and writes CodeTracer trace
//! output files.
//!
//! # Usage
//!
//! ```text
//! codetracer-cardano-recorder record <aiken-file> \
//!     --out-dir <output-dir> \
//!     [--format binary|json]
//! ```

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use codetracer_trace_writer::TraceEventsFileFormat;
use eyre::{Context, Result};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

/// CodeTracer Aiken/Cardano recorder -- record Aiken smart contract execution traces.
#[derive(Debug, Parser)]
#[command(
    name = "codetracer-cardano-recorder",
    version,
    about = "Record Aiken smart contract execution traces for CodeTracer"
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
    /// captures the execution trace, and writes CodeTracer trace files
    /// to `--out-dir`.
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

#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    Binary,
    Json,
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
    #[arg(short = 'o', long, default_value = "./ct-traces/")]
    out_dir: PathBuf,

    /// Output format for the trace data.
    #[arg(short = 'f', long, default_value = "binary")]
    format: OutputFormat,
}

#[derive(Debug, clap::Args)]
struct RecordArgs {
    /// Path to the Aiken source (.ak) file.
    program: PathBuf,

    /// Directory where the trace files will be written.
    ///
    /// The directory will be created if it does not exist.
    #[arg(short = 'o', long, default_value = "./ct-traces/")]
    out_dir: PathBuf,

    /// Output format for the trace data.
    #[arg(short = 'f', long, default_value = "binary")]
    format: OutputFormat,
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

    let format = match args.format {
        OutputFormat::Binary => TraceEventsFileFormat::Binary,
        OutputFormat::Json => TraceEventsFileFormat::Json,
    };

    // 2. Create the output directory
    let out_dir = &args.out_dir;
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("cannot create output dir: {}", out_dir.display()))?;

    // 3. Run the recorder
    codetracer_cardano_recorder::recorder::record(&source_path, out_dir, format)?;

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
