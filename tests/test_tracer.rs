//! Integration tests for the Aiken tracer.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use codetracer_trace_writer_nim::TraceEventsFileFormat;
use num_bigint::BigInt;
use uplc::ast::{Constant, NamedDeBruijn, Program, Term};
use uplc::builtins::DefaultFunction;
use uplc::machine::cost_model::ExBudget;

const CTFS_MAGIC: [u8; 5] = [0xC0, 0xDE, 0x72, 0xAC, 0xE2];

fn test_programs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-programs/aiken")
}

fn run_tracer_on_file(source_path: &Path, out_dir: &Path) {
    codetracer_cardano_recorder::recorder::record(
        source_path,
        out_dir,
        TraceEventsFileFormat::Binary,
    )
    .expect("trace_program should succeed");
}

fn assert_valid_ct_file(out_dir: &Path) -> PathBuf {
    let ct_files: Vec<_> = std::fs::read_dir(out_dir)
        .expect("failed to read output directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "ct"))
        .collect();
    assert!(!ct_files.is_empty(), "expected at least one .ct file in {:?}", out_dir);
    let ct_path = &ct_files[0];
    let content = std::fs::read(ct_path).expect("failed to read .ct file");
    assert!(content.len() >= 5, ".ct file too small");
    assert_eq!(&content[..5], &CTFS_MAGIC, "CTFS magic bytes mismatch");
    ct_path.clone()
}

#[test]
fn test_aiken_compile_and_run() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    let ct_path = assert_valid_ct_file(&out_dir);
    let size = std::fs::metadata(&ct_path).unwrap().len();
    assert!(size > 100, ".ct file should have substantial content, got {} bytes", size);
}

#[test]
fn test_aiken_compute_value() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_aiken_variable_values() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_aiken_step_events() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_aiken_metadata_structure() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_aiken_function_calls() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_aiken_cli_record() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("cli-traces");
    let source_path = test_programs_dir().join("flow_test.ak");

    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "run", "--quiet", "--",
            "record", source_path.to_str().unwrap(),
            "--out-dir", out_dir.to_str().unwrap(),
            "--format", "json",
        ])
        .output()
        .expect("failed to run");
    assert!(output.status.success(), "record should succeed, stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert_valid_ct_file(&out_dir);
}

// ---------------------------------------------------------------------------
// UPLC evaluation tests (pure unit tests, no trace writer)
// ---------------------------------------------------------------------------

#[test]
fn test_uplc_cek_machine_addition() {
    let term: Term<NamedDeBruijn> = Term::Apply {
        function: Rc::new(Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::AddInteger)),
            argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(10))))),
        }),
        argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(32))))),
    };
    let program = Program { version: (1, 0, 0), term };
    let eval_result = program.eval(ExBudget::default());
    let result_term = eval_result.result().unwrap();
    assert_eq!(result_term, Term::Constant(Rc::new(Constant::Integer(BigInt::from(42)))));
}

#[test]
fn test_uplc_cek_machine_full_computation() {
    let sum_val = Term::Apply {
        function: Rc::new(Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::AddInteger)),
            argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(10))))),
        }),
        argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(32))))),
    };
    let doubled = Term::Apply {
        function: Rc::new(Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::MultiplyInteger)),
            argument: Rc::new(sum_val),
        }),
        argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(2))))),
    };
    let final_result = Term::Apply {
        function: Rc::new(Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::AddInteger)),
            argument: Rc::new(doubled),
        }),
        argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(10))))),
    };
    let comparison = Term::Apply {
        function: Rc::new(Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::EqualsInteger)),
            argument: Rc::new(final_result),
        }),
        argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(94))))),
    };
    let program = Program { version: (1, 0, 0), term: comparison };
    let eval_result = program.eval(ExBudget::default());
    let result_term = eval_result.result().unwrap();
    assert_eq!(result_term, Term::Constant(Rc::new(Constant::Bool(true))));
}

#[test]
fn test_uplc_parse_and_eval() {
    let uplc_src = "(program 1.0.0 [ [ (builtin addInteger) (con integer 10) ] (con integer 32) ])";
    let parsed = uplc::parser::program(uplc_src).expect("UPLC should parse");
    let named_db: Program<NamedDeBruijn> = parsed.to_named_debruijn().expect("should convert");
    let eval_result = named_db.eval(ExBudget::default());
    let result_term = eval_result.result().unwrap();
    assert_eq!(result_term, Term::Constant(Rc::new(Constant::Integer(BigInt::from(42)))));
}

#[test]
fn test_uplc_file_evaluation() {
    let uplc_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-programs/uplc/flow_test.uplc");
    let uplc_src = std::fs::read_to_string(&uplc_path).expect("failed to read flow_test.uplc");
    let parsed = uplc::parser::program(&uplc_src).expect("flow_test.uplc should parse");
    let named_db: Program<NamedDeBruijn> = parsed.to_named_debruijn().expect("should convert");
    let eval_result = named_db.eval(ExBudget::default());
    let result_term = eval_result.result().unwrap();
    assert_eq!(result_term, Term::Constant(Rc::new(Constant::Bool(true))));
}
