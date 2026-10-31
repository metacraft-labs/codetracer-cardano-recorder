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
//! * `BLOCKFROST_API_KEY` — Blockfrost API key, used only by `replay
//!   --via-blockfrost` (Cardano-specific; not part of the standard recorder
//!   env-var set). No lane in this workspace holds one, and that path cannot
//!   run a budget comparison in any case — see [`codetracer_cardano_recorder::
//!   blockfrost`]. The default `replay` path needs no credential.
//! * `CODETRACER_CARDANO_RECORDER_LIVE_CHAIN` — set to `1` or `true` to let the
//!   acquisition subcommands (`enumerate`, `provenance`, `eras`,
//!   `prestate-strategy`, `fixtures`), `replay` and `budget-oracle` reach the
//!   live chain. Unset, they read
//!   the committed Koios captures under `tests/fixtures/koios/` instead, and
//!   anything those captures do not cover is reported **NOT MEASURED** rather
//!   than skipped. Koios needs no API key, so `--live` costs nothing but
//!   network access; the default is offline so a run is reproducible.
//!
//! # The acquisition subcommands (CXP-ADA-2)
//!
//! `enumerate` counts blocks and transactions over a bounded height range and
//! prints the counts it measured. `provenance` reports, per script execution
//! and per requisite, whether the transaction carries the bytes or a state
//! read has to answer for them — the reading that retired this chain's
//! `self-contained` claim. `eras` enumerates the protocol-version boundaries
//! with each era's cost models and reports the acquisition floor and the
//! script-tracing floor separately, because they are different numbers.
//! `prestate-strategy` prints the declared `hydrated-from-node` token together
//! with the re-measurement that licenses it. `fixtures` recomputes every
//! committed capture's hash identity.
//!
//! # The budget oracle (CXP-ADA-1 deliverable 4)
//!
//! `replay` takes one transaction and `budget-oracle` takes a corpus or a
//! height range. Both reconstruct Plutus's `ScriptContext`, step every script
//! through the chain's own CEK machine under the cost model of the epoch the
//! transaction settled in, and compare each redeemer's budget against the
//! execution units the transaction's own witness set declares.
//!
//! `budget-oracle` prints the **distribution** rather than a verdict, because
//! the relation was measured and is not equality: the ledger validates only
//! that execution *fits inside* the declaration, so a submitter's round
//! over-estimate is an ordinary transaction and reads `UNDER`. `OVER` is
//! always a defect and fails the command.

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

    /// Enumerate blocks and transactions over a bounded height range.
    ///
    /// Prints the block and transaction counts measured in this run, together
    /// with the per-block cross-check between each block's own `tx_count` and
    /// the number of transaction rows actually enumerated.
    Enumerate(EnumerateArgs),

    /// Report where each script execution's requisites actually live.
    ///
    /// For every script execution in a transaction, says per requisite
    /// (script, datum, redeemer, inputs, script context) whether the bytes are
    /// carried in the transaction or whether a state read answers for them,
    /// and names the resolution path for the second.
    Provenance(ProvenanceArgs),

    /// Enumerate ledger-era boundaries, their cost models, and both floors.
    Eras(ErasArgs),

    /// Print the declared prestate strategy and the reading that licenses it.
    PrestateStrategy(ChainArgs),

    /// Replay a corpus of transactions and report the budget relation.
    ///
    /// For each transaction, every script is stepped through the chain's CEK
    /// machine under its own epoch's cost model and each redeemer's replayed
    /// budget is compared against the execution units the transaction's
    /// witness set declares. Prints the distribution — EQUAL / UNDER / OVER —
    /// and names every transaction that could not be measured.
    BudgetOracle(BudgetOracleArgs),

    /// Recompute the hash identity of every committed Koios capture.
    Fixtures,

    /// Print version information.
    Version,
}

/// Flags every acquisition subcommand shares.
#[derive(Debug, clap::Args)]
struct ChainArgs {
    /// Reach the live chain (Koios, no API key) instead of the committed
    /// captures.
    ///
    /// Also enabled by `CODETRACER_CARDANO_RECORDER_LIVE_CHAIN=1`.
    #[arg(long)]
    live: bool,
}

#[derive(Debug, clap::Args)]
struct EnumerateArgs {
    /// First block height, inclusive. Defaults to the captured window's start.
    #[arg(long)]
    from_height: Option<u64>,

    /// Last block height, inclusive. Defaults to the captured window's end.
    #[arg(long)]
    to_height: Option<u64>,

    #[command(flatten)]
    chain: ChainArgs,
}

#[derive(Debug, clap::Args)]
struct ProvenanceArgs {
    /// Cardano transaction hash (64 hex characters).
    ///
    /// Omit it to report every captured case.
    #[arg(long)]
    tx_hash: Option<String>,

    #[command(flatten)]
    chain: ChainArgs,
}

#[derive(Debug, clap::Args)]
struct ErasArgs {
    /// Epochs to read parameters for. Defaults to the captured boundary set.
    #[arg(long, value_delimiter = ',')]
    epochs: Option<Vec<u64>>,

    #[command(flatten)]
    chain: ChainArgs,
}

#[derive(Debug, clap::Args)]
struct BudgetOracleArgs {
    /// Transactions to replay. Repeatable.
    ///
    /// Omit to replay every captured case, which is the standing corpus.
    #[arg(long)]
    tx_hash: Vec<String>,

    /// Replay every script transaction in a height range instead.
    ///
    /// Needs `--live` unless the range is one the captures cover.
    #[arg(long)]
    from_height: Option<u64>,

    /// Last block height, inclusive.
    #[arg(long)]
    to_height: Option<u64>,

    #[command(flatten)]
    chain: ChainArgs,
}

#[derive(Debug, clap::Args)]
struct ReplayArgs {
    /// Cardano transaction hash (64 hex characters).
    #[arg(long)]
    tx_hash: String,

    /// The epoch the transaction settled in, overriding the chain read.
    ///
    /// The epoch selects the cost model the replay is charged under. It is
    /// normally read from the chain, because it cannot be read off the
    /// transaction's bytes; this flag exists for a transaction no capture
    /// covers.
    #[arg(long)]
    epoch: Option<u64>,

    /// Use the older Blockfrost client instead of the Koios/capture path.
    ///
    /// That client cannot supply a redeemer's data or a script context, so it
    /// reports the budget comparison as NOT MEASURED rather than running one.
    #[arg(long)]
    via_blockfrost: bool,

    /// Blockfrost API key, for `--via-blockfrost`. If not provided, the
    /// BLOCKFROST_API_KEY environment variable is used.
    #[arg(long)]
    blockfrost_key: Option<String>,

    /// Directory where the trace files will be written.
    ///
    /// Falls back to `CODETRACER_CARDANO_RECORDER_OUT_DIR` when omitted.
    #[arg(short = 'o', long)]
    out_dir: Option<PathBuf>,

    #[command(flatten)]
    chain: ChainArgs,
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
        Commands::Enumerate(args) => enumerate_cmd(args),
        Commands::Provenance(args) => provenance_cmd(args),
        Commands::Eras(args) => eras_cmd(args),
        Commands::PrestateStrategy(args) => prestate_cmd(args),
        Commands::BudgetOracle(args) => budget_oracle_cmd(args),
        Commands::Fixtures => fixtures_cmd(),
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
/// Reads a settled transaction's bytes and the outputs it consumes,
/// reconstructs Plutus's `ScriptContext`, **steps** every script through the
/// chain's own CEK machine under the cost model of the epoch the transaction
/// settled in, and compares each redeemer's replayed budget against the
/// execution units the transaction's own witness set declares.
///
/// The default source is the committed captures, so the comparison is
/// reproducible and needs no credential. `--via-blockfrost` selects the older
/// client instead and is honest about what it cannot do: see
/// [`codetracer_cardano_recorder::blockfrost`].
fn replay(args: ReplayArgs) -> Result<()> {
    use codetracer_cardano_recorder::cek::AccountingFault;
    use codetracer_cardano_recorder::oracle;

    eprintln!("Replaying transaction: {}", args.tx_hash);

    if recording_disabled() {
        eprintln!("{ENV_DISABLED} is set; skipping replay (no output written).");
        return Ok(());
    }

    if args.tx_hash.len() != 64 || !args.tx_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(eyre::eyre!(
            "invalid transaction hash: expected 64 hex characters, got '{}'",
            args.tx_hash
        ));
    }

    // The replay subcommand does not yet write CTFS output, but we
    // resolve the output dir up-front so callers can rely on the
    // env-var fallback once writer support lands.
    let _out_dir = resolve_out_dir(args.out_dir);

    if args.via_blockfrost {
        return replay_via_blockfrost(&args.tx_hash, args.blockfrost_key.clone());
    }

    let src = chain_source(args.chain.live);

    // The epoch is read from the chain, not assumed: it is what selects the
    // cost model, and a replay charged under the wrong era's prices is not a
    // replay of that transaction. `--epoch` exists for a transaction whose
    // epoch no capture covers, and it is an override rather than a default.
    let epoch = match args.epoch {
        Some(e) => {
            eprintln!("epoch {e} supplied on the command line, overriding the chain read");
            e
        }
        None => src.tx_epoch(&args.tx_hash)?,
    };

    let reading =
        oracle::replay_from_source(src.as_ref(), &args.tx_hash, epoch, AccountingFault::none())?;
    print!("{}", oracle::render(&reading));

    // The oracle's verdict. The relation the recorder asserts is the ledger's
    // own — execution fits inside the declaration — because that is what was
    // measured; see `oracle::BudgetRelation`. An OVER reading is always a
    // defect and fails here.
    if reading.over_count() > 0 {
        return Err(eyre::eyre!(
            "{} of {} redeemer(s) replayed MORE than the transaction declares. A settled \
             transaction's execution fit inside its declaration by the ledger's own rule, so \
             either this replay is wrong or the transaction could not have settled",
            reading.over_count(),
            reading.redeemers.len()
        ));
    }
    for r in &reading.redeemers {
        if r.errored {
            return Err(eyre::eyre!(
                "the {}[{}] script evaluated to ERROR. A settled transaction's phase-two scripts \
                 succeeded, so this is a divergence even though the budget fits",
                r.purpose,
                r.index
            ));
        }
    }
    eprintln!(
        "Budget oracle: AGREES -- {} of {} redeemer(s) exact, {} within the declaration, 0 over",
        reading.exact_count(),
        reading.redeemers.len(),
        reading.redeemers.len() - reading.exact_count()
    );

    Ok(())
}

/// The older Blockfrost path, kept and not pretended about.
///
/// `BlockfrostClient` can read a script's bytes and the units the chain
/// recorded, and it cannot read a redeemer's *data* (its endpoint serves the
/// data hash) or assemble a `ScriptContext`. So this prints what it can read
/// and reports the budget comparison as NOT MEASURED rather than running a
/// comparison whose inputs are a hash and an empty context.
fn replay_via_blockfrost(tx_hash: &str, key: Option<String>) -> Result<()> {
    use codetracer_cardano_recorder::blockfrost::BlockfrostClient;

    let client = match key {
        Some(k) => BlockfrostClient::new(k),
        None => BlockfrostClient::from_env()?,
    };
    let summary = client.fetch_script_summary(tx_hash)?;
    print!("{summary}");
    eprintln!(
        "Budget oracle: NOT MEASURED via Blockfrost -- this client cannot supply a redeemer's \
         data or a script context, so there is nothing to step. Drop --via-blockfrost to run the \
         comparison against the committed captures, or pass --live for the keyless Koios path."
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// The acquisition subcommands (CXP-ADA-2)
// ---------------------------------------------------------------------------

/// Resolve the chain source these subcommands read from.
///
/// Offline is the default on purpose: the committed captures make a run
/// reproducible, and a subcommand that silently reached the network would
/// report a different measurement on every machine. When the captures do not
/// cover what was asked for, the source says **NOT MEASURED** and names the
/// flag — never an empty result.
fn chain_source(live: bool) -> Box<dyn codetracer_cardano_recorder::chain::ChainSource> {
    use codetracer_cardano_recorder::{fixture::FixtureSource, koios};
    if live || koios::live_chain_enabled() {
        eprintln!(
            "chain source: koios-live ({}), no API key sent",
            koios::DEFAULT_BASE_URL
        );
        Box::new(koios::KoiosClient::new())
    } else {
        let src = FixtureSource::at_crate_root();
        eprintln!(
            "chain source: koios-fixture ({}). Pass --live or set {}=1 to reach the chain.",
            src.dir().display(),
            koios::LIVE_CHAIN_ENV
        );
        Box::new(src)
    }
}

fn enumerate_cmd(args: EnumerateArgs) -> Result<()> {
    use codetracer_cardano_recorder::{enumerate, fixture::ALONZO_WINDOW};

    let from = args.from_height.unwrap_or(ALONZO_WINDOW.0);
    let to = args.to_height.unwrap_or(ALONZO_WINDOW.1);
    let src = chain_source(args.chain.live);
    let e = enumerate::enumerate(src.as_ref(), from, to)?;
    print!("{}", enumerate::render(&e));
    if !e.agrees() {
        return Err(eyre::eyre!(
            "the enumeration disagrees with the blocks' own transaction counts, so the count \
             above is not a measurement of the range"
        ));
    }
    Ok(())
}

fn provenance_cmd(args: ProvenanceArgs) -> Result<()> {
    use codetracer_cardano_recorder::{fixture::cases, provenance};

    let src = chain_source(args.chain.live);

    // Which transactions to read. With no `--tx-hash`, every captured case,
    // because the point of the reading is the contrast between them.
    let hashes: Vec<String> = match args.tx_hash {
        Some(h) => {
            if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(eyre::eyre!(
                    "invalid transaction hash: expected 64 hex characters, got '{h}'"
                ));
            }
            vec![h]
        }
        None => {
            let fx = codetracer_cardano_recorder::fixture::FixtureSource::at_crate_root();
            let mut out = Vec::new();
            for case in cases::ALL {
                let f = fx.load(&format!("tx_cbor_{case}"))?;
                let h = f
                    .identity
                    .get("expect_tx_hash")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| {
                        eyre::eyre!("capture tx_cbor_{case} declares no expect_tx_hash")
                    })?;
                out.push(h.to_string());
            }
            out
        }
    };

    for h in &hashes {
        let cbor = src.tx_cbor(h)?;
        let era = ledger_era_of(src.as_ref(), h);
        let p = provenance::classify_in_era(&cbor, h, src.as_ref(), era.as_deref())?;
        print!("{}", provenance::render(&p));

        let refs = provenance::reference_inputs(&p, src.as_ref())?;
        if refs.is_empty() {
            println!(
                "  reference inputs: none (CIP-31 reference inputs did not exist before Babbage)"
            );
        } else {
            println!("  reference inputs ({}):", refs.len());
            for r in &refs {
                println!(
                    "    {}  resolved={} is_spent={} script={} type={} inline_datum={}",
                    r.reference,
                    r.resolved,
                    r.is_spent,
                    r.carries_script.as_deref().unwrap_or("-"),
                    r.script_type.as_deref().unwrap_or("-"),
                    r.carries_inline_datum
                );
            }
            println!("    resolution path: {}", refs[0].resolution_path);
        }
        println!();
    }
    Ok(())
}

fn eras_cmd(args: ErasArgs) -> Result<()> {
    use codetracer_cardano_recorder::{eras, fixture::BOUNDARY_EPOCHS};

    let src = chain_source(args.chain.live);
    let versions = src.protocol_versions()?;
    let boundaries = eras::boundaries(&versions);
    eprintln!(
        "read {} epoch(s) of protocol versions; {} boundary/boundaries",
        versions.len(),
        boundaries.len()
    );

    let epochs = args.epochs.unwrap_or_else(|| BOUNDARY_EPOCHS.to_vec());
    let rows = eras::era_table(src.as_ref(), &epochs)?;
    let floors = eras::floors(&rows);
    print!("{}", eras::render(&rows, &boundaries, &floors));

    // And the thing that makes era handling load-bearing rather than
    // decorative: build each era's own cost model from its own parameters,
    // and report the refusals as refusals.
    println!("\ncost models built from each era's own parameters:");
    for row in &rows {
        let p = src.epoch_params(row.epoch_no)?;
        for (lang, _) in eras::EVALUATOR_ARITY {
            match eras::cost_model_for(&p, lang) {
                Ok(_) => println!(
                    "  epoch {:>4} {lang}: BUILT from the chain's own parameters",
                    row.epoch_no
                ),
                Err(e) => println!("  epoch {:>4} {lang}: REFUSED — {e}", row.epoch_no),
            }
        }
    }
    Ok(())
}

fn prestate_cmd(args: ChainArgs) -> Result<()> {
    use codetracer_cardano_recorder::{fixture, prestate, provenance};

    let src = chain_source(args.live);
    let fx = fixture::FixtureSource::at_crate_root();

    let mut cases = Vec::new();
    for case in fixture::cases::ALL {
        let f = fx.load(&format!("tx_cbor_{case}"))?;
        let h = f
            .identity
            .get("expect_tx_hash")
            .and_then(|x| x.as_str())
            .ok_or_else(|| eyre::eyre!("capture tx_cbor_{case} declares no expect_tx_hash"))?
            .to_string();
        let cbor = src.tx_cbor(&h)?;
        let era = ledger_era_of(src.as_ref(), &h);
        cases.push((
            case.to_string(),
            provenance::classify_in_era(&cbor, &h, src.as_ref(), era.as_deref())?,
        ));
    }

    let reading = prestate::read(&cases);
    print!("{}", prestate::render(&reading));
    if !reading.licenses_declaration() {
        return Err(eyre::eyre!(
            "the reading does not license the declared `{}` token, so this producer must not \
             write it into provenance.prestateStrategy",
            prestate::PRESTATE_STRATEGY
        ));
    }
    Ok(())
}

/// The standing budget oracle, over a corpus rather than one transaction.
///
/// Prints the distribution it measured. The relation the recorder asserts is
/// the ledger's own — execution fits inside the declaration — because that is
/// what the measurement found; `oracle::BudgetRelation` records why. An OVER
/// reading is always a defect and fails the command.
fn budget_oracle_cmd(args: BudgetOracleArgs) -> Result<()> {
    use codetracer_cardano_recorder::cek::AccountingFault;
    use codetracer_cardano_recorder::{fixture::cases, oracle};

    let src = chain_source(args.chain.live);

    let census = match (args.from_height, args.to_height) {
        (Some(lo), Some(hi)) => {
            oracle::census_over_range(src.as_ref(), lo, hi, AccountingFault::none())?
        }
        (None, None) => {
            let hashes = if args.tx_hash.is_empty() {
                corpus_tx_hashes()?
            } else {
                for h in &args.tx_hash {
                    if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Err(eyre::eyre!(
                            "invalid transaction hash: expected 64 hex characters, got '{h}'"
                        ));
                    }
                }
                args.tx_hash.clone()
            };
            eprintln!(
                "corpus: {} transaction(s) ({} captured case(s) available)",
                hashes.len(),
                cases::ALL_WITH_BUDGET.len()
            );
            oracle::census_over_hashes(src.as_ref(), &hashes, AccountingFault::none())
        }
        _ => {
            return Err(eyre::eyre!(
                "a height range needs both --from-height and --to-height; one of the two names \
                 no range and would enumerate a different population than the flag reads like"
            ))
        }
    };

    print!("{}", oracle::render_census(&census));

    if census.redeemer_count() == 0 {
        return Err(eyre::eyre!(
            "no redeemer was replayed, so the distribution above is over an empty set. A census \
             of nothing is not a measurement"
        ));
    }
    if census.over_count() > 0 || census.errored_count() > 0 {
        return Err(eyre::eyre!(
            "{} redeemer(s) replayed MORE than the transaction declares and {} evaluated to \
             ERROR. A settled transaction's phase-two scripts succeeded within their \
             declaration, so either the replay is wrong or the transaction could not have settled",
            census.over_count(),
            census.errored_count()
        ));
    }
    Ok(())
}

/// The committed corpus's transaction hashes, taken from the captures rather
/// than hard-coded, so adding a case extends the corpus by committing a
/// capture.
fn corpus_tx_hashes() -> Result<Vec<String>> {
    use codetracer_cardano_recorder::fixture::{cases, FixtureSource};
    let fx = FixtureSource::at_crate_root();
    let mut out = Vec::new();
    for case in cases::ALL_WITH_BUDGET {
        let f = fx.load(&format!("tx_cbor_{case}"))?;
        let h = f
            .identity
            .get("expect_tx_hash")
            .and_then(|x| x.as_str())
            .ok_or_else(|| {
                eyre::eyre!("capture tx_cbor_{case} declares no `identity.expect_tx_hash`")
            })?;
        out.push(h.to_string());
    }
    Ok(out)
}

fn fixtures_cmd() -> Result<()> {
    use codetracer_cardano_recorder::fixture::{
        render_identities, verify_identities, FixtureSource, IdentityVerdict,
    };

    let src = FixtureSource::at_crate_root();
    eprintln!("fixture directory: {}", src.dir().display());
    let readings = verify_identities(&src)?;
    print!("{}", render_identities(&readings));

    let broken: Vec<&str> = readings
        .iter()
        .filter(|r| r.verdict == IdentityVerdict::Broken)
        .map(|r| r.fixture.as_str())
        .collect();
    if !broken.is_empty() {
        return Err(eyre::eyre!(
            "{} committed capture(s) failed their own hash identity: {:?}. These files no longer \
             hold the bytes the chain hashed, so nothing read from them is a reading of the chain",
            broken.len(),
            broken
        ));
    }
    Ok(())
}

/// Resolve the LEDGER era of a transaction, or report that it was not.
///
/// The era comes from the epoch of the block the transaction settled in,
/// looked up in the captured window — never from the CBOR decoder's probe,
/// which returns the newest format that parses and therefore calls a 2021
/// Alonzo transaction "Conway". A transaction outside the window resolves to
/// `None`, and every report prints that as unknown.
fn ledger_era_of(
    src: &dyn codetracer_cardano_recorder::chain::ChainSource,
    tx_hash: &str,
) -> Option<String> {
    use codetracer_cardano_recorder::{chain, fixture::ALONZO_WINDOW};
    let epoch = chain::epoch_of_tx(src, tx_hash, ALONZO_WINDOW.0, ALONZO_WINDOW.1).ok()??;
    src.epoch_params(epoch).ok()?.era
}
