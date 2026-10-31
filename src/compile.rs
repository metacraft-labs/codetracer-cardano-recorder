//! The real Aiken compiler, invoked — so the artifact this recorder executes
//! is the artifact the compiler produced.
//!
//! This module exists to satisfy CXP-ADA-1 deliverable 3. §2.2 of
//! `Chain-Support-Matrix.md` grades this recorder *"compiler never invoked"*,
//! and that was accurate: before this module nothing in the crate reached an
//! Aiken front-end, so a trace of a local `.ak` file described a program the
//! compiler had never seen.
//!
//! # What this module observes
//!
//! Given an Aiken module on disk, it runs **the compiler's own** pipeline and
//! nothing of its own:
//!
//! | stage | the compiler's own function |
//! |-------|-----------------------------|
//! | lex + parse | [`aiken_lang::parser::module`] |
//! | type check | `UntypedModule::infer` (`aiken_lang::tipo::infer`) |
//! | code generation | [`CodeGenerator::generate_raw`] |
//! | storage encoding + script hash | `uplc::ast::SerializableProgram::compiled_code_and_hash` |
//!
//! There is no parser, no type checker and no lowering in this file. A module
//! the compiler rejects is reported with **the compiler's own diagnostic** and
//! is not compiled by some looser path, which is the whole point: a
//! hand-written front-end that accepts what the compiler rejects is the defect
//! §2.1 warns about, wearing a compiler's name.
//!
//! The last row matters as much as the third. `compiled_code_and_hash` is what
//! Aiken's own blueprint serializer calls to produce a validator's
//! `compiledCode` and `hash` fields (see `uplc::ast::SerializableProgram`'s
//! `Serialize` impl), so the bytes this module calls the *stored artifact* are
//! byte-for-byte the bytes a `plutus.json` publishes and byte-for-byte the
//! shape [`crate::oracle`] reads off a chain-stored script — flat-encoded
//! UPLC with de Bruijn indices, wrapped in CBOR, named by blake2b-224 over the
//! language-tagged script. `Source-Resolution.md` §2 lists the blueprint as
//! part of the UPLC source bundle; this is the artifact half of it.
//!
//! # The comparison this module makes, and the one it does NOT make
//!
//! CXP-ADA-1's verification entry for this deliverable says *"the UPLC
//! executed is byte-compared against the script the chain stores"*. For a
//! **local** `.ak` file there is no chain-stored script to compare against:
//! nothing was ever submitted, so no transaction names it and no script hash
//! of it exists on any ledger. Inventing one would be a fabricated oracle.
//!
//! So the comparison implemented here is stated in its own terms: **the UPLC
//! handed to the CEK machine is byte-identical to what the real compiler
//! produced for that module**, in the encoding and under the hash identity the
//! chain uses to store and name a script. Concretely, three artifacts and two
//! assertions:
//!
//! - `A` — the compiler's output, encoded for storage ([`CompiledArtifact::stored`]).
//! - `B` — the program handed to the machine, decoded from `A` exactly as
//!   [`crate::oracle`] decodes a chain-stored script
//!   (`Program::<FakeNamedDeBruijn>::from_cbor`).
//! - `B'` — `B` re-encoded for storage. **Taken from a clone of `B` made
//!   before the machine is started, not after it finishes.**
//!
//! and the assertions are `A == B'` on bytes and `hash(A) == hash(B')`.
//!
//! **What that establishes, stated exactly, because the obvious stronger
//! reading is wrong.** [`cek::step_program`] takes the program by value and
//! immediately moves `program.term` into its own `MachineState`; the machine
//! never writes back. `B'` is therefore fixed before the first transition and
//! **nothing the stepping does can change it**. So these two assertions say:
//! the value handed to the machine re-encodes to the compiler's bytes under
//! the compiler's own hash — i.e. they rule out a *substitution between
//! `decode` and `execute`* (a re-parse, a normalising rewrite, a different
//! module) and they confirm the storage codec round-trips. With no fault
//! injected there is nothing in that window, so on the production path
//! `A == B'` reduces to `encode(decode(A)) == A`.
//!
//! They do **not** observe execution. Measured, 2026-10-31, by substituting
//! the executed term for a constant *after* `B'` is derived: every assertion
//! in `tests/test_compiler_invocation.rs`'s deliverable entry still passed and
//! it still reported "15 of 15 executed the compiler's own bytes". What rules
//! out executing something else is that [`execute_artifact_with_fault`] moves
//! the very value it cloned straight into the machine — a property of this
//! function's shape, read from the code, and not something these assertions
//! measure. A comparison that observed the run would have to be taken from the
//! machine's own final state, which `uplc`'s `Machine` does not expose.
//!
//! **The other half of the entry's clause is already satisfied elsewhere, and
//! this module does not re-satisfy it.** [`crate::oracle`]'s `replay` /
//! `budget-oracle` path executes the chain's own script bytes, decoded from
//! committed transaction CBOR: the executed artifact *is* the stored script,
//! so "byte-compared against the script the chain stores" holds there by
//! construction rather than by comparison, and the budgets agree with the
//! units the chain recorded. The gap deliverable 3 closes is the **`record`**
//! path's — tracing Aiken *source* — and that is what this module is for.
//!
//! # What this module does NOT do
//!
//! - It does **not** touch [`crate::tracer`]. The source-simulating
//!   interpreter is untouched by this landing — not deleted, not improved, not
//!   routed through. Removing it is deliverable 2 and a separate change.
//! - It does **not** make `record` authoritative. `record` still goes through
//!   [`crate::recorder`] into [`crate::tracer`]. This module is the
//!   replacement's compiler half; wiring it into `record` and deleting the
//!   simulator is deliverable 2's work.
//! - It does **not** reconstruct a `ScriptContext`, resolve a script from a
//!   transaction, or read the chain. [`crate::oracle`] does that.
//! - It does **not** choose a cost model. [`crate::eras::cost_model_for`]
//!   does, from the parameters the chain served for a named epoch, and this
//!   module is handed the result. `CostModel::default()` was removed from
//!   `src/` by the deliverable-4 landing and is **not** reintroduced here: a
//!   corpus charged under default prices would report a budget no era ever
//!   set.
//! - It does **not** compile Aiken *projects*. There is no `aiken.toml` in
//!   this repo and the corpus is 19 bare modules, so the module route is the
//!   one that fits what is on disk — see [`CORPUS_DIR`].
//! - It writes no file and opens no socket. It reads the `.ak` sources it is
//!   pointed at and nothing else.
//!
//! # Mock use
//!
//! None. Every stage above is the compiler's own code, reached through
//! `aiken-lang`'s public API, and the machine is the `uplc` crate's.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aiken_lang::ast::{Definition, ModuleKind, Tracing, TypedModule};
use aiken_lang::gen_uplc::CodeGenerator;
use aiken_lang::line_numbers::LineNumbers;
use aiken_lang::plutus_version::PlutusVersion;
use aiken_lang::tipo::TypeInfo;
use aiken_lang::utils::indexmap::{as_ref_values, as_str_ref_values};
use aiken_lang::IdGenerator;
use eyre::{eyre, Context as _, Result};
use indexmap::IndexMap;
use pallas_primitives::conway::Language;
use uplc::ast::{DeBruijn, FakeNamedDeBruijn, NamedDeBruijn, Program, SerializableProgram};
use uplc::machine::cost_model::{CostModel, ExBudget};

use crate::cek::{self, CekRun};

// ---------------------------------------------------------------------------
// The corpus on disk
// ---------------------------------------------------------------------------

/// Where the Aiken corpus lives, relative to the crate root.
///
/// **Measured, because it decides the compile route.** This directory holds
/// 19 bare `.ak` modules and there is no `aiken.toml` anywhere in the repo, so
/// the corpus is not a set of Aiken *projects*: `aiken build`'s project →
/// validator → blueprint pipeline has no subject here. Each module instead
/// carries `test` definitions, which is exactly what `aiken check` compiles
/// through [`CodeGenerator::generate_raw`] — a typed function body lowered to
/// UPLC with no validator required. That is the route this module takes, and
/// it is the compiler's own route for a `test`, not a bypass of it.
pub const CORPUS_DIR: &str = "test-programs/aiken";

/// The corpus's shared-library subdirectory.
pub const CORPUS_LIB_DIR: &str = "lib";

/// The module-name prefix the corpus's library directory is imported under.
///
/// **Read off the corpus rather than chosen.** `module_imports_test.ak` is the
/// only module in the corpus with a `use` of a sibling, and it spells it
/// `use my_project/helpers.{double, Pair}`; the file it names is
/// `test-programs/aiken/lib/helpers.ak`. So the library directory is exposed
/// to the type checker under this prefix, because that is the name the one
/// importer in the tree uses. A different prefix makes that module fail to
/// type-check, which is a reading this module would report rather than hide.
pub const CORPUS_LIB_MODULE_PREFIX: &str = "my_project";

/// The package name the corpus's modules are type-checked and code-generated
/// under.
///
/// Aiken keys functions and data types by `(module, name)` within a package,
/// so this only has to be stable; it is not a published package.
pub const CORPUS_PACKAGE: &str = "codetracer/cardano-test-programs";

/// The epoch whose cost model the corpus is charged under.
///
/// A local `.ak` module is not a transaction: it settled in no epoch, so no
/// epoch is *its* epoch and there is nothing to read. What must not happen is
/// `CostModel::default()`, which charges prices no era ever set — the
/// era-blindness defect the deliverable-4 landing closed. So the corpus is
/// charged under a **named mainnet epoch's own PlutusV3 parameters**, taken
/// from the committed `/epoch_params` capture, and the epoch is printed with
/// every reading. 664 is the latest epoch the captures cover.
pub const CORPUS_COST_MODEL_EPOCH: u64 = 664;

/// The Plutus version the compiler targets for the corpus.
///
/// `generate_raw` emits a program whose version word is `1.1.0` for V3 and
/// `1.0.0` for V1/V2 ([`CodeGenerator::new_program`]), and the version also
/// decides which `Language` the CEK machine is built with, so this is not
/// cosmetic. V3 is Aiken's own default for a new project and is the version
/// mainnet is on.
pub const CORPUS_PLUTUS_VERSION: PlutusVersion = PlutusVersion::V3;

/// Every `.ak` module in the corpus's top level, sorted, excluding the library
/// subdirectory.
///
/// The library modules are compiled as *dependencies* of the modules that
/// import them, not as corpus entries of their own: `lib/helpers.ak` carries
/// no `test`, so it has nothing to execute.
pub fn corpus_programs(corpus_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(corpus_dir)
        .with_context(|| format!("cannot read the Aiken corpus at {}", corpus_dir.display()))?;
    for entry in entries {
        let path = entry?.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "ak") {
            out.push(path);
        }
    }
    out.sort();
    if out.is_empty() {
        return Err(eyre!(
            "no .ak module found in {} — a corpus reading over an empty population would report \
             a vacuous pass",
            corpus_dir.display()
        ));
    }
    Ok(out)
}

/// Every library module in the corpus, as `(module name, path)`.
pub fn corpus_library_modules(corpus_dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let lib_dir = corpus_dir.join(CORPUS_LIB_DIR);
    if !lib_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&lib_dir)? {
        let path = entry?.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "ak") {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| eyre!("library module {} has no usable name", path.display()))?;
            out.push((format!("{CORPUS_LIB_MODULE_PREFIX}/{stem}"), path));
        }
    }
    out.sort();
    Ok(out)
}

// ---------------------------------------------------------------------------
// The compiled artifact
// ---------------------------------------------------------------------------

/// One `test` definition, compiled by the real Aiken compiler and encoded the
/// way the chain stores a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledArtifact {
    /// The module the `test` was declared in.
    pub module: String,
    /// The `test`'s own name.
    pub test_name: String,
    /// Which Plutus language the artifact is, as the CEK machine names it.
    pub language: Language,
    /// The language tag the chain writes, for reporting.
    pub language_tag: &'static str,
    /// **The stored artifact.** Flat-encoded UPLC with de Bruijn indices,
    /// wrapped in CBOR — byte-for-byte a blueprint's `compiledCode` and
    /// byte-for-byte the shape [`crate::oracle`] decodes off the chain.
    pub stored: Vec<u8>,
    /// The hash the chain would name this script by: blake2b-224 over the
    /// language-tagged script bytes, from `pallas`'s own `compute_hash`.
    pub script_hash: String,
}

impl CompiledArtifact {
    /// Decode the stored artifact into the program the CEK machine runs.
    ///
    /// Deliberately the same two lines [`crate::oracle`] uses on a
    /// chain-stored script, so the `record` path's artifact enters the machine
    /// by the same door a settled transaction's does.
    pub fn decode(&self) -> Result<Program<NamedDeBruijn>> {
        let mut buffer = Vec::new();
        let program: Program<NamedDeBruijn> =
            Program::<FakeNamedDeBruijn>::from_cbor(&self.stored, &mut buffer)
                .map_err(|e| {
                    eyre!(
                        "the compiler's output for {}.{} does not decode as flat UPLC: {e:?}",
                        self.module,
                        self.test_name
                    )
                })?
                .into();
        Ok(program)
    }
}

/// A deliberate alteration of the artifact *handed to the machine*.
///
/// This is the control CXP-ADA-1's verification entry names — *"a deliberately
/// altered script produces a mismatch, so the comparison can fail in the
/// direction the defect would take"* — and it is built the way
/// [`cek::AccountingFault`] is, for the same reason. It perturbs **the
/// artifact the recorder executes**, not the comparison that consumes it:
/// perturbing the comparison would prove only that `!=` works.
///
/// It is [`ArtifactFault::none`] on every production path and reachable only
/// through [`execute_artifact_with_fault`], so it cannot be arrived at by a
/// default argument.
///
/// [`ArtifactFault::altered_version_word`] is chosen over flipping a byte in
/// the flat encoding for a measured reason: a flipped flat byte usually fails
/// to *decode*, so the comparison never runs and the control would be proving
/// that the decoder works. Rewriting the program's version word after decoding
/// leaves a program that decodes, runs to `Done`, and spends the identical
/// budget — so the **byte comparison is the only thing that can catch it**,
/// which is exactly the direction the defect would take.
///
/// **The fault class this covers, and the one it does not.** It is injected
/// into the window between `decode` and `execute`, which is the window
/// [`execute_artifact_with_fault`]'s comparison can see, so it covers
/// *substitution of the machine's input*: a recorder that re-parsed,
/// normalised or swapped the program before running it. It does not — and no
/// fault reachable from here could — cover divergence *inside* the run: `B'`
/// is re-encoded from a clone taken before the first transition, so the
/// comparison is fixed before the machine starts. Note also why the budget and
/// step count are identical rather than merely close: the machine is handed
/// `program.term` and is built with [`crate::cek`]'s separate `Language`
/// argument, so the version word this fault rewrites is never read during
/// execution at all. That is what makes it a clean test of the byte
/// comparison, and it is also why it is not evidence that the comparison
/// observes execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArtifactFault {
    /// Replace the program's version word before it is executed.
    pub version: Option<(usize, usize, usize)>,
}

impl ArtifactFault {
    /// No injection: what every production path uses.
    pub fn none() -> Self {
        ArtifactFault { version: None }
    }

    /// Execute an artifact whose version word is not the one the compiler
    /// emitted.
    pub fn altered_version_word() -> Self {
        ArtifactFault {
            version: Some((1, 0, 0)),
        }
    }

    pub fn is_clean(&self) -> bool {
        self.version.is_none()
    }

    fn apply(&self, mut program: Program<NamedDeBruijn>) -> Program<NamedDeBruijn> {
        if let Some(version) = self.version {
            program.version = version;
        }
        program
    }
}

/// What happened when one compiled artifact was executed.
#[derive(Debug, Clone)]
pub struct ExecutedArtifact {
    /// The artifact the compiler produced.
    pub artifact: CompiledArtifact,
    /// `true` when the artifact the machine executed re-encodes, byte for
    /// byte, to the artifact the compiler produced.
    pub bytes_identical: bool,
    /// `true` when the executed artifact carries the compiler's own script
    /// hash.
    pub hash_identical: bool,
    /// The storage encoding of what was actually executed. Equal to
    /// [`CompiledArtifact::stored`] iff [`Self::bytes_identical`].
    pub executed_stored: Vec<u8>,
    /// The script hash of what was actually executed.
    pub executed_script_hash: String,
    /// The stepped run, or the machine's own failure.
    pub run: std::result::Result<CekRun, String>,
}

impl ExecutedArtifact {
    /// The deliverable's clause, in one place: the artifact executed is the
    /// artifact the compiler produced, and it ran.
    pub fn is_measured(&self) -> bool {
        self.bytes_identical && self.hash_identical && self.run.is_ok()
    }

    /// Transitions observed, or `None` when the machine failed.
    pub fn step_count(&self) -> Option<usize> {
        self.run.as_ref().ok().map(|r| r.step_count())
    }

    /// Units the run consumed, or `None` when the machine failed.
    pub fn consumed(&self) -> Option<ExBudget> {
        self.run.as_ref().ok().map(|r| r.consumed())
    }
}

// ---------------------------------------------------------------------------
// Invoking the compiler
// ---------------------------------------------------------------------------

/// Type-check a module with the real compiler and lower every unit `test` in
/// it to UPLC.
///
/// `deps` are modules the entry may `use`, given as `(module name, path)` and
/// type-checked first in the order supplied. The entry's own module name is
/// its file stem.
///
/// Errors carry **the compiler's own diagnostic**. A module the compiler
/// refuses is a finding, and this function is the only place that can produce
/// it: there is no second, looser front-end to fall back to.
pub fn compile_module(
    entry: &Path,
    deps: &[(String, PathBuf)],
    plutus_version: PlutusVersion,
) -> Result<Vec<CompiledArtifact>> {
    let id_gen = IdGenerator::new();

    // The prelude, from the compiler's own builtins. Without these two the
    // type checker knows no `Int`, no `Bool` and no builtin.
    let mut module_types: HashMap<String, TypeInfo> = HashMap::new();
    module_types.insert("aiken".to_string(), aiken_lang::builtins::prelude(&id_gen));
    module_types.insert(
        "aiken/builtin".to_string(),
        aiken_lang::builtins::plutus(&id_gen),
    );

    let mut functions = aiken_lang::builtins::prelude_functions(&id_gen, &module_types);
    let mut constants = IndexMap::new();
    let mut data_types = aiken_lang::builtins::prelude_data_types(&id_gen);
    let mut module_sources: HashMap<String, (String, LineNumbers)> = HashMap::new();

    for (name, path) in deps {
        let typed = infer_one(
            path,
            name,
            ModuleKind::Lib,
            &id_gen,
            &mut module_types,
            &mut module_sources,
        )?;
        typed.register_definitions(&mut functions, &mut constants, &mut data_types);
    }

    let entry_name = entry
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| eyre!("{} has no usable module name", entry.display()))?
        .to_string();

    // `ModuleKind::Validator` rather than `Lib`, because the corpus contains a
    // module with a `validator` block and the compiler only accepts one in a
    // validator-kind module. A `test` is accepted in either kind, so this is
    // the kind that accepts the whole corpus.
    let typed = infer_one(
        entry,
        &entry_name,
        ModuleKind::Validator,
        &id_gen,
        &mut module_types,
        &mut module_sources,
    )?;
    typed.register_definitions(&mut functions, &mut constants, &mut data_types);

    let mut generator = CodeGenerator::new(
        plutus_version,
        as_ref_values(&functions),
        as_ref_values(&constants),
        as_ref_values(&data_types),
        as_str_ref_values(&module_types),
        as_str_ref_values(&module_sources),
        Tracing::silent(),
    );

    let mut out = Vec::new();
    for def in typed.definitions() {
        let Definition::Test(test) = def else {
            continue;
        };
        if !test.arguments.is_empty() {
            // A property test is a template, not a program: the compiler
            // lowers it with a fuzzer argument applied. The corpus has none
            // (measured: all 22 `test` definitions are nullary), and
            // pretending one compiled to a runnable artifact would be a
            // fabricated row.
            return Err(eyre!(
                "{}.{} is a property test; this module compiles unit tests only",
                entry_name,
                test.name
            ));
        }

        // The compiler's own code generation for a unit test — the identical
        // call `aiken_lang::test_framework::Test::unit_test` makes.
        let program = generator.generate_raw(&test.body, &[], &entry_name);

        out.push(encode_for_storage(
            &entry_name,
            &test.name,
            program,
            plutus_version,
        )?);
    }

    if out.is_empty() {
        return Err(eyre!(
            "{} type-checks but declares no `test`, so there is nothing to execute",
            entry.display()
        ));
    }

    Ok(out)
}

/// Parse and type-check one module with the compiler's own front-end.
fn infer_one(
    path: &Path,
    module_name: &str,
    kind: ModuleKind,
    id_gen: &IdGenerator,
    module_types: &mut HashMap<String, TypeInfo>,
    module_sources: &mut HashMap<String, (String, LineNumbers)>,
) -> Result<TypedModule> {
    let code =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;

    let (mut ast, _extra) = aiken_lang::parser::module(&code, kind).map_err(|errors| {
        eyre!(
            "the Aiken parser refused this module: {}",
            render_errors(&errors)
        )
    })?;
    ast.name = module_name.to_string();

    let mut warnings = Vec::new();
    let typed = ast
        .infer(
            id_gen,
            kind,
            CORPUS_PACKAGE,
            module_types,
            Tracing::silent(),
            &mut warnings,
            None,
        )
        .map_err(|e| {
            eyre!(
                "the Aiken type checker refused this module: {} [{}]",
                one_line(&e.to_string()),
                // The `Debug` form names the error's own variant and the
                // offending identifier and span. It is included because the
                // `Display` text alone says what kind of mistake it is and not
                // where — and "the specific reason" is what a corpus refusal
                // has to carry to be a finding rather than a count.
                diagnostic_head(&e)
            )
        })?;

    module_types.insert(module_name.to_string(), typed.type_info.clone());
    module_sources.insert(
        module_name.to_string(),
        (code.clone(), LineNumbers::new(&code)),
    );

    Ok(typed)
}

fn render_errors<E: std::fmt::Display>(errors: &[E]) -> String {
    errors
        .iter()
        .map(|e| one_line(&e.to_string()))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Collapse a multi-line compiler diagnostic onto one line, so a corpus
/// reading stays one row per module.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Fields of a compiler diagnostic that carry a *suggestion list* rather than
/// the diagnostic itself.
///
/// They are cut from the reported head for a measured reason: they are built
/// from `HashMap` iteration, so their order changes between runs. A reading
/// quoted into the record has to be the same reading on the next run, and a
/// list that reshuffles would make an unchanged corpus look like it moved.
const UNSTABLE_DIAGNOSTIC_FIELDS: [&str; 5] = [
    "variables:",
    "known_modules:",
    "valid:",
    "supplied:",
    "candidates:",
];

/// The head of a diagnostic's structural form: its own variant name, the
/// identifier it is about and the span it points at.
///
/// Truncated, because several variants carry the whole set of names in scope
/// as a suggestion list and that would be pages — and unstably ordered pages.
fn diagnostic_head<E: std::fmt::Debug>(e: &E) -> String {
    let mut full = one_line(&format!("{e:?}"));
    if let Some(cut) = UNSTABLE_DIAGNOSTIC_FIELDS
        .iter()
        .filter_map(|f| full.find(f))
        .min()
    {
        full.truncate(cut);
        full = format!("{}…}}", full.trim_end().trim_end_matches(','));
    }
    if full.chars().count() <= 200 {
        full
    } else {
        format!("{}…", full.chars().take(200).collect::<String>())
    }
}

/// Encode a generated program the way the chain stores a script, and name it
/// the way the chain names one.
///
/// Both come from `uplc::ast::SerializableProgram::compiled_code_and_hash`,
/// which is the same function Aiken's blueprint serializer calls — so these
/// bytes are a `plutus.json`'s `compiledCode` and this hash is its `hash`.
fn encode_for_storage(
    module: &str,
    test_name: &str,
    program: Program<uplc::ast::Name>,
    plutus_version: PlutusVersion,
) -> Result<CompiledArtifact> {
    let debruijn = program.to_debruijn().map_err(|e| {
        eyre!(
            "the compiler's output for {module}.{test_name} would not convert to de Bruijn: {e:?}"
        )
    })?;
    let (language, language_tag, stored, script_hash) = store(debruijn, plutus_version);
    Ok(CompiledArtifact {
        module: module.to_string(),
        test_name: test_name.to_string(),
        language,
        language_tag,
        stored,
        script_hash,
    })
}

/// The storage encoding and the script identity, per Plutus version.
fn store(
    program: Program<DeBruijn>,
    plutus_version: PlutusVersion,
) -> (Language, &'static str, Vec<u8>, String) {
    let serializable = match plutus_version {
        PlutusVersion::V1 => SerializableProgram::PlutusV1Program(program),
        PlutusVersion::V2 => SerializableProgram::PlutusV2Program(program),
        PlutusVersion::V3 => SerializableProgram::PlutusV3Program(program),
    };
    let (hash, script) = serializable.compiled_code_and_hash();
    let (language, tag) = match plutus_version {
        PlutusVersion::V1 => (Language::PlutusV1, "PlutusV1"),
        PlutusVersion::V2 => (Language::PlutusV2, "PlutusV2"),
        PlutusVersion::V3 => (Language::PlutusV3, "PlutusV3"),
    };
    (language, tag, script.as_ref().to_vec(), hash.to_string())
}

// ---------------------------------------------------------------------------
// Executing the compiled artifact, and proving it is the one that ran
// ---------------------------------------------------------------------------

/// Step a compiled artifact through the chain's CEK machine and byte-compare
/// what ran against what the compiler produced.
pub fn execute_artifact(
    artifact: &CompiledArtifact,
    costs: CostModel,
    initial_budget: ExBudget,
) -> Result<ExecutedArtifact> {
    execute_artifact_with_fault(artifact, costs, initial_budget, ArtifactFault::none())
}

/// [`execute_artifact`], with a deliberate alteration injected into the
/// artifact that is executed.
///
/// Only the control test passes a non-clean [`ArtifactFault`]; every other
/// caller goes through [`execute_artifact`].
pub fn execute_artifact_with_fault(
    artifact: &CompiledArtifact,
    costs: CostModel,
    initial_budget: ExBudget,
    fault: ArtifactFault,
) -> Result<ExecutedArtifact> {
    // `B`: decoded from the compiler's stored bytes, by the same two lines the
    // oracle uses on a chain-stored script.
    let executed = fault.apply(artifact.decode()?);

    // `B'`: what the machine *is about to be handed*, re-encoded for storage.
    //
    // Note the order, because it bounds the claim: this clone is taken BEFORE
    // `step_program` below, and `step_program` moves the program into the
    // machine and never writes back. So `A == B'` is a statement about the
    // machine's INPUT — it rules out a substitution in the window between
    // `decode` and `execute`, which is the window `fault` injects into — and
    // not a statement about what the machine then did. See this module's
    // header for the measurement that pins this down.
    let round_trip = Program::<DeBruijn>::from(executed.clone());
    let plutus_version = match artifact.language {
        Language::PlutusV1 => PlutusVersion::V1,
        Language::PlutusV2 => PlutusVersion::V2,
        Language::PlutusV3 => PlutusVersion::V3,
    };
    let (_, _, executed_stored, executed_script_hash) = store(round_trip, plutus_version);

    let run = cek::step_program(executed, &artifact.language, costs, initial_budget)
        .map_err(|e| e.to_string());

    Ok(ExecutedArtifact {
        artifact: artifact.clone(),
        bytes_identical: executed_stored == artifact.stored,
        hash_identical: executed_script_hash == artifact.script_hash,
        executed_stored,
        executed_script_hash,
        run,
    })
}

// ---------------------------------------------------------------------------
// The corpus reading
// ---------------------------------------------------------------------------

/// What the compiler and the machine made of one corpus module.
#[derive(Debug, Clone)]
pub enum ProgramOutcome {
    /// Every `test` in the module compiled; each row says whether it ran and
    /// whether what ran was the compiler's artifact.
    Compiled(Vec<ExecutedArtifact>),
    /// The real compiler refused the module. The string is its own diagnostic,
    /// verbatim.
    CompilerRefused(String),
}

/// One corpus module's reading.
#[derive(Debug, Clone)]
pub struct ProgramReading {
    pub path: PathBuf,
    pub module: String,
    pub outcome: ProgramOutcome,
}

/// The corpus, compiled and stepped.
#[derive(Debug, Clone)]
pub struct CorpusReading {
    pub corpus_dir: PathBuf,
    /// The epoch whose cost model every artifact was charged under.
    pub epoch: u64,
    /// That epoch's era, as the chain names it.
    pub era: String,
    pub language_tag: &'static str,
    pub programs: Vec<ProgramReading>,
}

impl CorpusReading {
    pub fn programs_total(&self) -> usize {
        self.programs.len()
    }

    pub fn programs_compiled(&self) -> usize {
        self.programs
            .iter()
            .filter(|p| matches!(p.outcome, ProgramOutcome::Compiled(_)))
            .count()
    }

    pub fn programs_refused(&self) -> usize {
        self.programs_total() - self.programs_compiled()
    }

    fn artifacts(&self) -> impl Iterator<Item = &ExecutedArtifact> {
        self.programs.iter().flat_map(|p| match &p.outcome {
            ProgramOutcome::Compiled(rows) => rows.as_slice(),
            ProgramOutcome::CompilerRefused(_) => &[],
        })
    }

    pub fn artifacts_total(&self) -> usize {
        self.artifacts().count()
    }

    /// Artifacts that ran to `Done`.
    pub fn artifacts_stepped(&self) -> usize {
        self.artifacts().filter(|a| a.run.is_ok()).count()
    }

    /// Artifacts whose executed bytes were the compiler's bytes.
    pub fn artifacts_identical(&self) -> usize {
        self.artifacts()
            .filter(|a| a.bytes_identical && a.hash_identical)
            .count()
    }

    /// Artifacts satisfying the deliverable's clause outright.
    pub fn artifacts_measured(&self) -> usize {
        self.artifacts().filter(|a| a.is_measured()).count()
    }

    /// Every artifact whose executed bytes differed from the compiler's.
    pub fn divergent(&self) -> Vec<&ExecutedArtifact> {
        self.artifacts()
            .filter(|a| !(a.bytes_identical && a.hash_identical))
            .collect()
    }
}

/// Compile and step the whole corpus, under a named epoch's own cost model.
///
/// `src` supplies the epoch parameters. Offline against
/// [`crate::fixture::FixtureSource`]; the cost model is a real mainnet era's
/// either way and the epoch is reported with the reading.
pub fn read_corpus(
    corpus_dir: &Path,
    src: &dyn crate::chain::ChainSource,
    epoch: u64,
) -> Result<CorpusReading> {
    let params = src.epoch_params(epoch).with_context(|| {
        format!(
            "the corpus is charged under epoch {epoch}'s own prices, so its parameters are a \
                 required read and not an optional one"
        )
    })?;
    let language_tag = match CORPUS_PLUTUS_VERSION {
        PlutusVersion::V1 => "PlutusV1",
        PlutusVersion::V2 => "PlutusV2",
        PlutusVersion::V3 => "PlutusV3",
    };
    // Built once here so an epoch that cannot price this language refuses
    // before any artifact is executed, and rebuilt per artifact below because
    // `uplc::machine::cost_model::CostModel` is deliberately not `Clone` and
    // `Machine::new` takes it by value.
    let _ = crate::eras::cost_model_for(&params, language_tag)?;

    let deps = corpus_library_modules(corpus_dir)?;
    let mut programs = Vec::new();
    for path in corpus_programs(corpus_dir)? {
        let module = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let outcome = match compile_module(&path, &deps, CORPUS_PLUTUS_VERSION) {
            Err(e) => ProgramOutcome::CompilerRefused(format!("{e:#}")),
            Ok(artifacts) => {
                let mut rows = Vec::new();
                for artifact in &artifacts {
                    let costs = crate::eras::cost_model_for(&params, language_tag)?;
                    rows.push(execute_artifact(artifact, costs, ExBudget::max())?);
                }
                ProgramOutcome::Compiled(rows)
            }
        };
        programs.push(ProgramReading {
            path,
            module,
            outcome,
        });
    }

    Ok(CorpusReading {
        corpus_dir: corpus_dir.to_path_buf(),
        epoch,
        era: params
            .era
            .clone()
            .unwrap_or_else(|| "no parameter row".to_string()),
        language_tag,
        programs,
    })
}

/// Render a corpus reading.
///
/// Prints the population before the verdict and names every refusal with the
/// compiler's own reason, because a corpus figure without its refusals is the
/// shape `Chain-Support-Matrix.md` §2.4 warns about.
pub fn render(reading: &CorpusReading) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();

    let _ = writeln!(
        s,
        "Aiken corpus at {} — compiled by the real Aiken compiler (aiken-lang {}), stepped \
         through the chain's CEK machine under epoch {} (era {}) {} prices",
        reading.corpus_dir.display(),
        aiken_lang::version::compiler_version(false),
        reading.epoch,
        reading.era,
        reading.language_tag,
    );

    for program in &reading.programs {
        match &program.outcome {
            ProgramOutcome::CompilerRefused(why) => {
                let _ = writeln!(s, "  {} — COMPILER REFUSED: {why}", program.module);
            }
            ProgramOutcome::Compiled(rows) => {
                for row in rows {
                    let identity = if row.bytes_identical && row.hash_identical {
                        "artifact IDENTICAL to the compiler's output"
                    } else {
                        "artifact DIFFERS from the compiler's output"
                    };
                    match &row.run {
                        Ok(run) => {
                            let used = run.consumed();
                            let _ = writeln!(
                                s,
                                "  {}.{} — {} bytes, hash {}, {}; {} CEK transitions, \
                                 mem={} steps={}, result {}",
                                program.module,
                                row.artifact.test_name,
                                row.artifact.stored.len(),
                                row.artifact.script_hash,
                                identity,
                                run.step_count(),
                                used.mem,
                                used.cpu,
                                result_label(run),
                            );
                        }
                        Err(why) => {
                            let _ = writeln!(
                                s,
                                "  {}.{} — {} bytes, hash {}, {}; MACHINE FAILED: {why}",
                                program.module,
                                row.artifact.test_name,
                                row.artifact.stored.len(),
                                row.artifact.script_hash,
                                identity,
                            );
                        }
                    }
                }
            }
        }
    }

    let _ = writeln!(
        s,
        "MEASURED: {} of {} module(s) compiled, {} refused by the compiler; {} artifact(s) \
         generated, {} executed the compiler's own bytes (byte-identical re-encoding and \
         identical script hash), {} stepped to Done",
        reading.programs_compiled(),
        reading.programs_total(),
        reading.programs_refused(),
        reading.artifacts_total(),
        reading.artifacts_identical(),
        reading.artifacts_stepped(),
    );

    s
}

/// How a run ended, in one word, without claiming more than was observed.
fn result_label(run: &CekRun) -> String {
    use uplc::ast::{Constant, Term};
    match &run.result {
        Term::Error => "Error".to_string(),
        Term::Constant(c) => match c.as_ref() {
            Constant::Bool(b) => format!("Bool({b})"),
            Constant::Integer(i) => format!("Integer({i})"),
            Constant::Unit => "Unit".to_string(),
            other => format!("Constant({other:?})")
                .chars()
                .take(48)
                .collect::<String>(),
        },
        other => format!("{other:?}").chars().take(48).collect::<String>(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The corpus constants must name things that exist, or every reading
    /// below them is over an empty population.
    #[test]
    fn test_the_corpus_directory_holds_modules_and_a_library() {
        let dir = Path::new(CORPUS_DIR);
        let programs = corpus_programs(dir).expect("the corpus must be readable");
        assert!(
            programs.len() >= 19,
            "the corpus had {} top-level module(s); the charter's reading is over 19 and a \
             shrunken corpus must fail rather than report a better ratio over fewer rows",
            programs.len()
        );
        let libs = corpus_library_modules(dir).expect("the library directory must be readable");
        assert_eq!(
            libs.len(),
            1,
            "the corpus's library directory held {} module(s); `module_imports_test.ak` imports \
             exactly one",
            libs.len()
        );
        assert_eq!(libs[0].0, "my_project/helpers");
    }

    /// `ArtifactFault::none` is what every production path uses, and the fault
    /// cannot be reached by a default argument.
    #[test]
    fn test_the_artifact_fault_is_clean_by_default() {
        assert!(ArtifactFault::none().is_clean());
        assert!(ArtifactFault::default().is_clean());
        assert!(!ArtifactFault::altered_version_word().is_clean());
    }
}
