//! Every program in `test-programs/aiken/` must be valid Aiken: the real
//! compiler accepts it, and its tests pass when the compiler's own runner
//! evaluates what the compiler generated.
//!
//! # Why this gate exists
//!
//! `src/tracer.rs` is a source simulator with a hand-written parser, so a
//! fixture only ever had to satisfy that parser. Five of the nineteen modules
//! were written that way and the compiler refused them: an unqualified
//! builtin, `list.map` with no import, `Option` with no type argument, a tuple
//! projected as `p.0`, and a validator whose handlers returned `Int` and were
//! called as free functions. A trace of a module the compiler rejects
//! describes no program that could ever reach the chain, so nothing bounds how
//! far that trace can diverge from what the chain would execute. This file is
//! the standing check that keeps a fixture that is not valid Aiken from being
//! added again.
//!
//! # Why the harness lives in a test
//!
//! `record` runs the simulator, and nothing in `src/` reaches a compiler. This
//! file is a check on the fixtures and does not change that, so `aiken-lang`
//! is a dev-dependency. It calls the same stages `aiken check` does, and adds
//! no front-end of its own:
//!
//! | stage | the compiler's own function |
//! |-------|-----------------------------|
//! | lex + parse | `aiken_lang::parser::module` |
//! | type check | `UntypedModule::infer` |
//! | code generation | `Test::unit_test`, through `CodeGenerator::generate_raw` |
//! | evaluation and verdict | `UnitTest::run` |
//!
//! # What the corpus imports
//!
//! `my_project/<x>` is `test-programs/aiken/lib/<x>.ak`, the name
//! `module_imports_test.ak` uses. `aiken/...` and `cardano/...` are
//! `aiken-lang/stdlib`, the workspace repo checked out at `../stdlib`.
//! `aiken` and `aiken/builtin` are the compiler's own. Any other import is
//! left unresolved, so the type checker refuses it rather than this file
//! guessing at it.
//!
//! # Mocking
//!
//! None. Every stage above is the compiler's, the machine is the `uplc` crate
//! the recorder links, the stdlib is the real package at the revision the
//! workspace pins, and the control modules are real files compiled the same
//! way as the corpus.
//!
//! # No skips
//!
//! A missing `../stdlib` fails with the remedy. It must never read as a
//! pass, because `higher_order_test.ak` can only compile against it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use aiken_lang::ast::{
    Definition, ModuleKind, Tracing, TypedDataType, TypedFunction, UntypedModule,
};
use aiken_lang::gen_uplc::CodeGenerator;
use aiken_lang::line_numbers::LineNumbers;
use aiken_lang::plutus_version::PlutusVersion;
use aiken_lang::test_framework::Test;
use aiken_lang::tipo::TypeInfo;
use aiken_lang::utils::indexmap::{as_ref_values, as_str_ref_values};
use aiken_lang::IdGenerator;
use indexmap::IndexMap;

/// The number of top-level modules. Pinned so that removing a fixture the
/// compiler refuses fails here instead of improving the result.
const CORPUS_MODULES: usize = 19;

/// The number of `test` definitions across those modules, pinned for the same
/// reason: a module emptied of its tests would otherwise pass vacuously.
const CORPUS_TESTS: usize = 22;

/// The prefix `test-programs/aiken/lib/` is imported under.
const CORPUS_LIB_PREFIX: &str = "my_project/";

/// The package the corpus is checked under. Aiken only needs it to be stable.
const CORPUS_PACKAGE: &str = "codetracer/cardano-test-programs";

/// V3 is the version a new Aiken project gets and the one mainnet is on.
const CORPUS_PLUTUS_VERSION: PlutusVersion = PlutusVersion::V3;

/// The compiler the stdlib checkout must have been released for. It is the
/// version of the `aiken-lang` dev-dependency.
const STDLIB_COMPILER_LINE: &str = "compiler = \"v1.1.21\"";

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("test-programs/aiken")
}

/// The `lib/` of the `stdlib` sibling, after checking it is the release
/// made for this compiler. Panics with the remedy rather than letting the
/// corpus fail with an unrelated `UnknownModule`.
fn stdlib_lib_dir() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../stdlib");
    let manifest = std::fs::read_to_string(root.join("aiken.toml")).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}. This test needs the `stdlib` sibling, so run \
             `repro sync --only=stdlib` from the workspace root",
            root.join("aiken.toml").display()
        )
    });
    assert!(
        manifest.lines().any(|line| line == STDLIB_COMPILER_LINE),
        "{} is not the stdlib release for this compiler (expected `{STDLIB_COMPILER_LINE}`). \
         Run `repro sync --only=stdlib` from the workspace root, or move repos/aiken-stdlib.toml \
         and the `aiken-lang` dev-dependency together",
        root.display()
    );
    root.join("lib")
}

/// Where a library module lives on disk, or `None` when nothing here provides
/// it. `None` is not an error at this level: the type checker then reports the
/// import in its own words.
fn library_path(module: &str, corpus: &Path, stdlib_lib: &Path) -> Option<PathBuf> {
    let candidate = if let Some(rest) = module.strip_prefix(CORPUS_LIB_PREFIX) {
        corpus.join("lib").join(format!("{rest}.ak"))
    } else if module.starts_with("aiken/") || module.starts_with("cardano/") {
        stdlib_lib.join(format!("{module}.ak"))
    } else {
        return None;
    };
    candidate.is_file().then_some(candidate)
}

fn parse(path: &Path, kind: ModuleKind) -> Result<(UntypedModule, String), String> {
    let code = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let (ast, _extra) = aiken_lang::parser::module(&code, kind).map_err(|errors| {
        let rendered: Vec<String> = errors.iter().map(|e| one_line(&e.to_string())).collect();
        format!("the Aiken parser refused it: {}", rendered.join("; "))
    })?;
    Ok((ast, code))
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A library module, parsed, waiting for its dependencies to be checked.
struct ParsedLibrary {
    name: String,
    ast: UntypedModule,
    code: String,
}

/// Add `imports` and what they import to `order`, dependencies first.
fn collect_libraries(
    imports: Vec<String>,
    corpus: &Path,
    stdlib_lib: &Path,
    seen: &mut HashSet<String>,
    order: &mut Vec<ParsedLibrary>,
) -> Result<(), String> {
    for name in imports {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(path) = library_path(&name, corpus, stdlib_lib) else {
            continue;
        };
        let (mut ast, code) = parse(&path, ModuleKind::Lib)?;
        ast.name.clone_from(&name);
        collect_libraries(ast.dependencies(&[]), corpus, stdlib_lib, seen, order)?;
        order.push(ParsedLibrary { name, ast, code });
    }
    Ok(())
}

/// What the compiler's own runner said about one `test`.
#[derive(Debug)]
struct TestVerdict {
    module: String,
    name: String,
    success: bool,
    logs: Vec<String>,
}

/// Run one module through the compiler: every import, then the module, then
/// every `test` in it. The error is the compiler's diagnostic.
fn check_module(
    entry: &Path,
    corpus: &Path,
    stdlib_lib: &Path,
) -> Result<Vec<TestVerdict>, String> {
    let id_gen = IdGenerator::new();

    // The prelude, from the compiler's own builtins. Without these two the
    // type checker knows no `Int`, no `Bool` and no builtin.
    let mut module_types: HashMap<String, TypeInfo> = HashMap::new();
    module_types.insert("aiken".to_string(), aiken_lang::builtins::prelude(&id_gen));
    module_types.insert(
        "aiken/builtin".to_string(),
        aiken_lang::builtins::plutus(&id_gen),
    );

    let mut functions: IndexMap<_, TypedFunction> =
        aiken_lang::builtins::prelude_functions(&id_gen, &module_types);
    let mut constants = IndexMap::new();
    let mut data_types: IndexMap<_, TypedDataType> =
        aiken_lang::builtins::prelude_data_types(&id_gen);
    let mut module_sources: HashMap<String, (String, LineNumbers)> = HashMap::new();

    let entry_name = entry
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("{} has no usable module name", entry.display()))?
        .to_string();

    // `Validator` because the compiler accepts a `validator` block only in a
    // module of that kind, and it accepts a `test` in either.
    let (mut entry_ast, entry_code) = parse(entry, ModuleKind::Validator)?;
    entry_ast.name.clone_from(&entry_name);

    let mut libraries = Vec::new();
    let mut seen = HashSet::new();
    collect_libraries(
        entry_ast.dependencies(&[]),
        corpus,
        stdlib_lib,
        &mut seen,
        &mut libraries,
    )?;

    let library_steps = libraries
        .into_iter()
        .map(|lib| (lib.name, lib.ast, lib.code, ModuleKind::Lib));
    let steps = library_steps.chain(std::iter::once((
        entry_name.clone(),
        entry_ast,
        entry_code,
        ModuleKind::Validator,
    )));

    let mut typed_entry = None;
    for (name, ast, code, kind) in steps {
        let mut warnings = Vec::new();
        let typed = ast
            .infer(
                &id_gen,
                kind,
                CORPUS_PACKAGE,
                &module_types,
                Tracing::silent(),
                &mut warnings,
                None,
            )
            .map_err(|e| {
                format!(
                    "the Aiken type checker refused `{name}`: {} [{}]",
                    one_line(&e.to_string()),
                    diagnostic_head(&e)
                )
            })?;
        module_types.insert(name.clone(), typed.type_info.clone());
        module_sources.insert(name, (code.clone(), LineNumbers::new(&code)));
        typed.register_definitions(&mut functions, &mut constants, &mut data_types);
        typed_entry = Some(typed);
    }
    // The entry is the chain's last element, so the loop leaves it here.
    let typed = typed_entry.ok_or("the module list was empty")?;

    let mut generator = CodeGenerator::new(
        CORPUS_PLUTUS_VERSION,
        as_ref_values(&functions),
        as_ref_values(&constants),
        as_ref_values(&data_types),
        as_str_ref_values(&module_types),
        as_str_ref_values(&module_sources),
        Tracing::silent(),
    );

    let mut verdicts = Vec::new();
    for definition in typed.definitions() {
        let Definition::Test(test) = definition else {
            continue;
        };
        if !test.arguments.is_empty() {
            return Err(format!(
                "{entry_name}.{} is a property test, and this gate runs unit tests only",
                test.name
            ));
        }
        let name = test.name.clone();
        let unit_test = match Test::unit_test(
            &mut generator,
            test.clone(),
            entry_name.clone(),
            entry.to_path_buf(),
        ) {
            Test::UnitTest(unit_test) => unit_test,
            Test::PropertyTest(_) | Test::Benchmark(_) => {
                return Err(format!(
                    "the compiler did not lower {entry_name}.{name} to a unit test"
                ));
            }
        };
        let result = unit_test.run(&CORPUS_PLUTUS_VERSION);
        verdicts.push(TestVerdict {
            module: entry_name.clone(),
            name,
            success: result.success,
            logs: result.logs,
        });
    }
    Ok(verdicts)
}

/// The variant, the identifier and the span of a diagnostic. It is cut at the
/// first suggestion list, because those are large and built from `HashMap`
/// iteration, so their order changes between runs.
fn diagnostic_head<E: std::fmt::Debug>(error: &E) -> String {
    let full = one_line(&format!("{error:?}"));
    let cut = ["variables:", "known_modules:", "valid:", "supplied:"]
        .iter()
        .filter_map(|field| full.find(field))
        .min();
    let head: String = full
        .chars()
        .take(cut.unwrap_or(usize::MAX).min(200))
        .collect();
    head.trim_end().trim_end_matches(',').to_string()
}

/// Every `.ak` file directly in the corpus directory, sorted. `lib/` holds
/// imports rather than programs, so it is reached through them.
fn corpus_modules() -> Vec<PathBuf> {
    let dir = corpus_dir();
    let mut modules: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read the corpus at {}: {e}", dir.display()))
        .map(|entry| entry.expect("a corpus directory entry").path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "ak"))
        .collect();
    modules.sort();
    modules
}

/// Every module's result, so a failure names all the refusals at once.
fn read_corpus() -> Vec<(String, Result<Vec<TestVerdict>, String>)> {
    let stdlib_lib = stdlib_lib_dir();
    let corpus = corpus_dir();
    corpus_modules()
        .iter()
        .map(|path| {
            let module = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("a corpus module name")
                .to_string();
            (module, check_module(path, &corpus, &stdlib_lib))
        })
        .collect()
}

/// The standing gate: no fixture may be one the compiler refuses.
#[test]
fn test_every_corpus_module_is_accepted_by_the_aiken_compiler() {
    let reading = read_corpus();
    assert_eq!(
        reading.len(),
        CORPUS_MODULES,
        "the corpus must hold {CORPUS_MODULES} modules. A module the compiler refuses is fixed, \
         not removed"
    );

    let refused: Vec<String> = reading
        .iter()
        .filter_map(|(module, outcome)| {
            outcome
                .as_ref()
                .err()
                .map(|why| format!("  {module}: {why}"))
        })
        .collect();
    assert!(
        refused.is_empty(),
        "{} of {} corpus module(s) are refused by the Aiken compiler, so the recorder traces \
         programs that are not valid Aiken:\n{}",
        refused.len(),
        reading.len(),
        refused.join("\n")
    );
}

/// A fixture must not disagree with the compiler about its own result. A
/// module can type-check and still have a `test` that only the simulator
/// thinks passes, for example a builtin the simulator answers with a
/// placeholder. `UnitTest::run` is what `aiken check` calls, so a `fail` test
/// has to error and any other has to return `True`.
#[test]
fn test_every_corpus_test_passes_under_the_compilers_own_runner() {
    let reading = read_corpus();
    let verdicts: Vec<&TestVerdict> = reading
        .iter()
        .filter_map(|(_, outcome)| outcome.as_ref().ok())
        .flatten()
        .collect();

    let failed: Vec<String> = verdicts
        .iter()
        .filter(|verdict| !verdict.success)
        .map(|v| format!("  {}.{}: {}", v.module, v.name, v.logs.join(" | ")))
        .collect();
    assert!(
        failed.is_empty(),
        "{} of {} corpus test(s) fail when the compiler's own runner evaluates them:\n{}",
        failed.len(),
        verdicts.len(),
        failed.join("\n")
    );
    assert_eq!(
        verdicts.len(),
        CORPUS_TESTS,
        "the corpus must hold {CORPUS_TESTS} `test` definitions; a refused module contributes \
         none, and `test_every_corpus_module_is_accepted_by_the_aiken_compiler` names it"
    );
}

/// The gate has to be able to fail. Each module here is written to be refused,
/// and each is checked by the function the corpus goes through. The three arms
/// are the three ways the corpus went wrong: a type error, syntax only the
/// simulator accepted, and an import nothing provides.
#[test]
fn test_the_gate_refuses_what_the_compiler_refuses() {
    let stdlib_lib = stdlib_lib_dir();
    let dir = tempfile::tempdir().expect("a temporary directory");

    let refusal = |file: &str, source: &str| -> String {
        let path = dir.path().join(file);
        std::fs::write(&path, source).expect("the control module must be writable");
        match check_module(&path, &corpus_dir(), &stdlib_lib) {
            Err(why) => why,
            Ok(verdicts) => panic!("{file} must be refused, and it compiled to {verdicts:?}"),
        }
    };

    let ill_typed = refusal(
        "ill_typed.ak",
        "fn broken() -> Int {\n  let x = True\n  x + 1\n}\n\ntest t() {\n  broken() == 1\n}\n",
    );
    assert!(
        ill_typed.contains("the Aiken type checker refused"),
        "{ill_typed}"
    );

    let numeric_projection = refusal(
        "numeric_projection.ak",
        "test t() {\n  let p = (7, 11)\n  p.0 == 7\n}\n",
    );
    assert!(
        numeric_projection.contains("the Aiken parser refused"),
        "{numeric_projection}"
    );

    let unknown_import = refusal(
        "unknown_import.ak",
        "use no_such_package/no_such_module\n\ntest t() {\n  True\n}\n",
    );
    assert!(unknown_import.contains("UnknownModule"), "{unknown_import}");
}

/// The runner's verdict has to be able to fail too. A verdict the harness
/// misread would leave `test_every_corpus_test_passes_under_the_compilers_own_runner`
/// green for every fixture, so both ways of failing are checked here: a `test`
/// that returns `False`, and a `fail` test that does not fail.
#[test]
fn test_the_runner_verdict_reports_a_failing_test() {
    let stdlib_lib = stdlib_lib_dir();
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("failing_tests.ak");
    std::fs::write(
        &path,
        "test passes() {\n  True\n}\n\ntest returns_false() {\n  False\n}\n\n\
         test fail_that_does_not() fail {\n  True\n}\n",
    )
    .expect("the control module must be writable");

    let verdicts =
        check_module(&path, &corpus_dir(), &stdlib_lib).expect("the control module is valid Aiken");
    let outcomes: Vec<(&str, bool)> = verdicts
        .iter()
        .map(|verdict| (verdict.name.as_str(), verdict.success))
        .collect();
    assert_eq!(
        outcomes,
        vec![
            ("passes", true),
            ("returns_false", false),
            ("fail_that_does_not", false),
        ]
    );
}

/// The stdlib route has to reach the real package. Imports are resolved in
/// this file, so a mapping that found nothing would leave every module that
/// does not import the stdlib passing. This module can only compile if
/// `aiken/collection/list` came from the sibling, and its test can only pass
/// if the sibling's `list.map` ran.
#[test]
fn test_a_stdlib_import_resolves_to_the_sibling_and_runs() {
    let stdlib_lib = stdlib_lib_dir();
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("stdlib_control.ak");
    std::fs::write(
        &path,
        "use aiken/collection/list\n\ntest t() {\n  list.map([1, 2], fn(x) { x + 1 }) == [2, 3]\n}\n",
    )
    .expect("the control module must be writable");

    let verdicts = check_module(&path, &corpus_dir(), &stdlib_lib)
        .expect("a module importing the stdlib must compile");
    assert_eq!(verdicts.len(), 1, "{verdicts:?}");
    assert!(verdicts[0].success, "{verdicts:?}");
}
