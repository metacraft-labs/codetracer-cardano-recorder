# Cardano Recorder CTFS Audit — 2026-05-02

This audit checks `codetracer-cardano-recorder` against the canonical
CodeTracer multi-stream CTFS schema and the section 5.6 audit checklist
maintained in `/tmp/isonim-migration.txt`.  Prior audits set the
canonical patterns: Ruby (1.21, 1.22), Python (1.27), JavaScript (1.38),
EVM (1.39), PHP (1.41), Solana (1.44), and Move (1.46).  This is the
**eighth** recorder audited.

The Cardano recorder is a *live* tracer for Aiken smart contracts.
Its execution model:

* `src/aiken_parser.rs` parses an Aiken `.ak` source file into a
  function/statement IR (`FunctionDef` / `LetBinding` / `Expr`).
* `src/tracer.rs` walks that IR and, for each statement, compiles
  the right-hand expression to UPLC and evaluates it through the
  real `uplc` crate's CEK machine.  Step / Variable / Call / Return
  events are emitted as the walk progresses.
* `src/main.rs` exposes two CLI subcommands: `record` (trace an Aiken
  source file) and `replay` (fetch a Plutus tx via Blockfrost and
  evaluate the on-chain script under the CEK machine).

The recorder uses the **Rust-native NimTraceWriter**
(`codetracer_trace_writer_nim` crate, sibling-path dep), not the C
FFI, so every canonical entry point (`register_call`, `arg`,
`register_special_event`, `register_thread_*`) is reachable.  The
recorder does **not** suffer from the C-FFI gaps documented in
section 5.6 of the migration handoff.

## Summary

| # | Check | Status (pre-fix) | Status (post-fix) | Notes |
|---|---|---|---|---|
| a | `register_call` for each call | OK | OK | `tracer.rs::evaluate_function` emits `register_call(fn_id, vec![])` for every non-entry function.  No `add_event(Call(..))` call anywhere. |
| b | Call args via `register_call_arg` / `arg()` | **GAP (parser-limited)** | **GAP (parser-limited)** | Every `register_call` site passes `vec![]`.  The Aiken parser currently only supports nullary calls (e.g. `compute()`) — `aiken_parser.rs` lacks a general argument expression parser, so even if the recorder wanted to stage args via `TraceWriter::arg`, it has nothing to stage.  Closing this requires extending the parser to capture `CallExpr.args` first.  See "Open: parameter recovery" below. |
| c | Write/WriteOther/Error/TraceLogEvent for IO and structured events via `register_special_event` | **GAP** | **OK** | Pre-fix: UPLC CEK evaluation errors (budget exhaustion, divide-by-zero, type errors) propagated up via `?` and aborted `trace_program` mid-write — partially-written trace, error message lost from the trace stream.  Post-fix: both `eval_expr_via_uplc` and `eval_comparison_with_call` route CEK errors through `register_special_event(Error, "AikenUplcEvalError", message)` and return `Ok(None)` so the trace finalises cleanly.  Aiken / Plutus has no native stdout/stderr (smart contracts are pure), so no Write/WriteOther path is needed. |
| d | Thread events (ThreadStart / Exit / Switch) | OK (N/A) | OK (N/A) | Plutus / UPLC is single-threaded by construction — every smart-contract execution runs in a single CEK-machine evaluation.  The recorder correctly emits no thread events. |
| e | Step records for line navigation | OK | OK | `tracer.rs::evaluate_function` emits `register_step(path, line)` for every `LetBinding` and every `Expr` statement.  Granularity matches the parser's IR (one step per statement, which corresponds to one source line). |
| f | Canonical CTFS schema match | **GAP** | **OK** | Pre-fix: CLI `--format` exposed only `Binary` / `Json` (defaulting to `Binary`, the legacy CBOR+Zstd format) with no way to request the canonical CTFS multi-stream container.  Post-fix: CLI exposes a typed `OutputFormat` enum (`Ctfs` / `Binary` / `Json`), defaults to `ctfs`, and dispatch sites convert via `impl From<OutputFormat> for TraceEventsFileFormat`.  This is the same legacy-format issue caught in EVM (1.39), Solana (1.44), and Move (1.46) audits. |
| g | Obsolete `#[no_mangle]` stubs | OK | OK | `grep -r '#\[no_mangle\]' src/` returns no results.  This recorder predates the JS-recorder pattern that introduced FFI stubs that conflicted with upstream Nim exports. |
| C-FFI vs native | OK | OK | `Cargo.toml` depends on `codetracer_trace_writer_nim` (sibling-path dep), not on the C FFI.  Every canonical API is reachable.  No FFI-extension blockers. |

## Concrete fixes applied

### 1. CLI now exposes and defaults to `Ctfs`

`src/main.rs`'s `OutputFormat` enum used to expose only `Binary` and
`Json`, with `Binary` as the default for both `RecordArgs` and
`ReplayArgs`.  There was no way to request the canonical CTFS
multi-stream container — and `Binary` writes the legacy CBOR+Zstd
format that the canonical Nim `ct_reader_*` FFI and the db-backend's
`CTFSTraceReader` cannot consume directly.

Post-fix: `OutputFormat` gains a `Ctfs` variant (listed first), with
doc-comments explaining each option, and a freshly added
`impl From<OutputFormat> for TraceEventsFileFormat` makes the call
sites uniform:

```rust
#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    /// Canonical CodeTracer multi-stream container (recommended).
    Ctfs,
    /// Legacy CBOR + Zstd binary format.
    Binary,
    /// Human-readable JSON (slower; useful for debugging).
    Json,
}

impl From<OutputFormat> for TraceEventsFileFormat {
    fn from(f: OutputFormat) -> Self {
        match f {
            OutputFormat::Ctfs => TraceEventsFileFormat::Ctfs,
            OutputFormat::Binary => TraceEventsFileFormat::Binary,
            OutputFormat::Json => TraceEventsFileFormat::Json,
        }
    }
}
```

Both `RecordArgs.format` and `ReplayArgs.format` now use
`#[arg(short = 'f', long, value_enum, default_value_t = OutputFormat::Ctfs)]`,
so `record --help` advertises `[default: ctfs]` and existing
invocations that omit `--format` automatically opt into the canonical
container.  This mirrors the canonical-format fix applied in the
EVM (1.39), Solana (1.44), and Move (1.46) recorders.

### 2. UPLC CEK evaluation errors routed through `register_special_event`

`src/tracer.rs::eval_expr_via_uplc` and
`eval_comparison_with_call` previously bubbled CEK failures up via
`?`:

```rust
let result_term = eval_uplc_term(uplc_term)?;
return Ok(term_to_i64(&result_term));
```

A CEK error (budget exhaustion, divide-by-zero, type mismatch, etc.)
would propagate all the way back to `trace_program`, abort the trace
walk mid-stream, and surface as a recorder-process error.  The result:
no `Error` event in the trace (the CodeTracer event-log pane stayed
silent), and the trace container itself was partially written —
`finish_writing_trace_events` may not even have run.

Post-fix (audit (c)), both call sites match on the result and route
the failure through `register_special_event` with the canonical
`EventLogKind::Error` and a stable metadata tag the frontend can
route on:

```rust
match eval_uplc_term(uplc_term) {
    Ok(result_term) => return Ok(term_to_i64(&result_term)),
    Err(err) => {
        let message = format!("{err}");
        TraceWriter::register_special_event(
            &mut *self.writer,
            EventLogKind::Error,
            "AikenUplcEvalError",
            &message,
        );
        return Ok(None);
    }
}
```

This mirrors the canonical pattern established by the Move (1.46)
audit for `Effect::ExecutionError` and the Solana (1.44) audit for
syscall errors.  The `metadata` tag (`"AikenUplcEvalError"`) is the
stable handle the frontend uses to distinguish Aiken UPLC failures
from other special events; the human-readable error message goes in
`content`.

The same fix is applied to both `eval_expr_via_uplc` (general
expression evaluation) and `eval_comparison_with_call` (comparison
operators that go through a separate UPLC compile path) so neither
escapes the trace stream.

## Tests added

`tests/test_ctfs_audit.rs` (new) locks in the post-fix behaviour
with four regression tests, mirroring the pattern used by EVM
(1.39), Solana (1.44), and Move (1.46):

* `test_ctfs_writer_produces_ct_container` — runs the recorder
  end-to-end against `flow_test.ak` with
  `TraceEventsFileFormat::Ctfs`, then asserts the produced `.ct`
  container starts with the canonical CTFS magic bytes
  (`0xC0 0xDE 0x72 0xAC 0xE2`).  This guards against accidental
  regressions in the `TraceEventsFileFormat` plumbing.
* `test_ctfs_format_advertised_in_help` — runs the CLI binary with
  `record --help` and asserts the output advertises `ctfs` as a
  `--format` value with `[default: ctfs]`.  Catches accidental
  reverts of the `OutputFormat` enum to its pre-fix shape.
* `test_steps_emitted_for_let_bindings` — originally a structural
  smoke test; the read-side follow-up now opens the produced `.ct`
  through `NimTraceReaderHandle` and asserts readable step count,
  function table entries, source path, call count, and step JSON.
* `test_uplc_eval_error_does_not_abort_trace` — synthesises an
  Aiken source whose final expression triggers a divide-by-zero in
  UPLC, runs the recorder, and asserts (a) `record(...)` returns
  `Ok` (the eval error did not abort the writer) and (b) the error
  is readable back from the `.ct` through `NimTraceReaderHandle` as
  an `error` event whose content mentions the divide failure.  Pre-
  fix this would fail because the CEK error propagated through `?`
  and aborted `trace_program` mid-write; post-fix it succeeds because
  the error is routed through `register_special_event`.

## Tests run

`cargo test --release` after fixes:

* `lib` unit tests: 31/31 passing.
* `test_ctfs_audit` (new): 4/4 passing.
* `test_tracer` (existing integration suite): 11/11 passing.

Total: **46/46 passing**, 0 regressions.

### Build / test environment note

The dev shell's flake config does not put `zstd` on `LIBRARY_PATH`
by default, and the system's first-on-`LIBRARY_PATH` zstd-1.5.2
package has its `RUNPATH` pinned to glibc-2.33, which then conflicts
with the binary's main libc (glibc-2.40) at load time
(`undefined symbol: __libc_siglongjmp, version GLIBC_PRIVATE`).
The fix is to point `LIBRARY_PATH` at a glibc-2.40-built zstd before
running cargo:

```bash
LIBRARY_PATH="/nix/store/5hg6h4zjxc3ax7j4ywn6ksd509yl4pmd-zstd-1.5.6/lib" \
  cargo test --release
```

This is an existing dev-shell ergonomics issue that predates the
audit; documented here so the next agent can run the suite without
re-investigating.

## Closed follow-ups

### Read-side CTFS content assertions

Closed in the follow-up audit test update.  `tests/test_ctfs_audit.rs`
now opens produced `.ct` containers through
`codetracer_trace_writer_nim::NimTraceReaderHandle` and asserts
reader-visible step count, call count, function names (`flow_test` /
`compute`), source path resolution for `flow_test.ak`, step JSON, and
the divide-by-zero `AikenUplcEvalError` as a readable `error` event.
This closes only the assertion-depth follow-up; no recorder semantics
changed.

## Open gaps / follow-ups

### Aiken parser limited to nullary calls (audit (b))

`src/aiken_parser.rs` currently parses calls as `name()` only.  The
parser does not yet have a general argument-expression sub-parser,
so multi-arg call syntax (`add(a, b)`, `compute(x + 1)`) is rejected
or miscategorised before the tracer ever sees it.  Until the parser
recognises argument expressions, there is no `args: Vec<Value>` to
stage — every `register_call` correctly passes `vec![]`.

Concrete shape for closing this:

1. Extend `aiken_parser.rs` to parse argument lists in `CallExpr`
   (a delimited expression list inside the parentheses).
2. In `tracer.rs::evaluate_function`, before recursing into the
   callee, evaluate each argument expression (using the same
   `eval_expr_via_uplc` path as for let-binding RHS) and stage
   each resulting value via `TraceWriter::arg(&format!("arg{idx}"), value)`
   — same idiom as the Move (1.46) Sui path.
3. Replace the `vec![]` argument to `register_call` with an empty
   vec still (the staged args attach via the writer's pending-args
   buffer, not via the explicit `args` argument — see the Move
   audit doc and `codetracer_trace_writer_nim/src/lib.rs:927` for
   the canonical pattern).

Real Aiken parameter names (rather than synthetic `arg{idx}`) would
require also lifting parameter identifiers from the function
definition through `func.params: Vec<String>` (currently parsed as a
flat string list) and using them as the `arg(name, value)` keys.

This mirrors the open Aptos parameter-recovery follow-up in 1.46
and the EVM `Call.args` follow-up in 1.39.

### Plutus replay path lacks step events

`main.rs::replay` (the Blockfrost-driven on-chain replay path) calls
`program.eval_version(...)` directly on the UPLC `Program`, prints
the result, and does **not** create a `TraceWriter` at all — so on-
chain Plutus replays emit no CodeTracer trace.  This is documented
behaviour for the current implementation (the `replay` subcommand is
currently a CLI evaluator, not a tracer), but to surface the on-
chain CEK steps in CodeTracer it would need:

1. A CEK-machine instrumentation hook similar to the one
   `tracer.rs` uses for source-driven recording.  The `uplc` crate
   does not currently expose a per-step callback; this would
   require either patching `uplc` or running the CEK steps through
   a wrapper machine.
2. A path-resolution layer that maps UPLC term positions back to
   the originating Aiken / Plutus source — typically requires the
   tx's accompanying source-map blob (which Blockfrost does not
   currently surface).

This is a larger feature than a CTFS audit can scope; tracked here
for completeness.

### Multi-stream IO event collapse (cross-cutting infrastructure issue)

Same cross-cutting issue documented in 1.39 (EVM), 1.41 (PHP),
1.44 (Solana), and 1.46 (Move): the Nim multi-stream IO event stream's
`toIOEventKind` collapse drops most of the 13 `EventLogKind` variants
into 4 buckets (`stdout`, `stderr`, `fileOp`, `error`).  Aiken's
`AikenUplcEvalError` (kind `Error`) lands cleanly in the `error`
bucket — the audit-relevant routing is correct.  But `metadata` is
dropped entirely in the multi-stream path, so the frontend cannot
distinguish Aiken UPLC errors from generic recorder errors without
reaching back to the embedded raw event stream.

Out of scope for any single recorder audit; flag as an
infrastructure follow-up in
`codetracer-trace-format-nim/src/codetracer_trace_writer_ffi.nim`.

### Aiken recorder repository split reclassification

The May 2026 handoff originally listed a separate
`codetracer-aiken-recorder` sibling.  The local checkout set contains
no such repository, and this crate is already the Aiken source recorder:
`src/recorder.rs` records `.ak` files, `src/tracer.rs` implements the
`AikenTracer`, and the audit tests use `test-programs/aiken/`.

Treat the Aiken audit scope as part of this Cardano recorder unless a
new standalone sibling is later added intentionally.  The still-open
Aiken-source gaps are the parser-limited call-argument support and the
broader Plutus replay-path tracing feature documented above; there is
no separate missing-repo audit item to run against the current sibling
layout.

### `start()` toplevel function still has `vec![]` args

`TraceWriter::start(writer, source_path, Line(1))` opens the
implicit toplevel call (the synthetic `<toplevel>` frame).  No args
attach to it, which is correct: the toplevel has no callsite-visible
args.  Listed here for completeness, matching the 1.46 audit's
analogous note.

### Variable types are always `Int`

`tracer.rs::evaluate_function` registers every variable as
`ValueRecord::Int { i, type_id }` because the parser currently
infers no other types.  Aiken supports `Bool`, `ByteArray`, `String`,
`List`, `Pair`, custom data types, etc. — surfacing them through
`ValueRecord` would improve the locals pane fidelity but is
orthogonal to the CTFS audit and tracked here as a non-blocking
follow-up.

## Convention compliance follow-up — 2026-05-08

The 2026-05-02 audit landed a `--format ctfs|binary|json` `clap::ValueEnum`
defaulting to `Ctfs`, mirroring the EVM (1.39) / Solana (1.44) /
Move (1.46) audits.  Subsequent to that audit,
`Recorder-CLI-Conventions.md` §4 in `codetracer-specs` was tightened
to require **CTFS-only** output: recorders no longer accept a
`--format` flag and `ct print` (shipped with `codetracer-trace-format-nim`)
is the canonical conversion tool for human-readable output.
`Repo-Requirements.md` §2.2 / §2.3 reflect this contract.

This entry records the convention compliance follow-up applied to the
Cardano recorder on 2026-05-08, mirroring the Cairo recorder
2026-05-08 follow-up landed in commit `2710b5e` of
`codetracer-cairo-recorder`:

* The `--format` / `-f` CLI flag was removed from `record` and
  `replay` subcommands.  The `OutputFormat` enum and the
  `impl From<OutputFormat> for TraceEventsFileFormat` block were
  deleted.  Clap rejects `--format <anything>` with an
  "unexpected argument" error.
* The JSON / legacy-binary output paths were removed.  The recorder's
  writer is hard-pinned to `TraceEventsFileFormat::Ctfs` at every call
  site: `tracer.rs::AikenTracer::trace_program` and
  `recorder.rs::record` no longer take a `format` parameter.
* The `events_filename` match in `tracer.rs` (which used to dispatch
  on `Json` / `Binary` / `BinaryV0` / `Ctfs`) was collapsed to a
  single CTFS arm.
* `CODETRACER_CARDANO_RECORDER_OUT_DIR` was added as a fallback for
  `--out-dir`.  Lookup order is CLI flag → env var → `./ct-traces/`.
* `CODETRACER_CARDANO_RECORDER_DISABLED=1` (or `true`) skips the
  trace emission entirely; the Cardano recorder doesn't run a
  separate target subprocess so "disabled" simply means "don't write
  any artefacts".
* The CTFS-only contract is now in force across the codebase: the
  binary's `--help` output mentions `ct print` as the conversion tool;
  the README documents only CTFS, the env-var contract, and the
  `ct print` workflow.
* Tests in `tests/test_tracer.rs` that previously drove the CLI with
  `--format json` were rewritten to drive the recorder library
  directly and assert on the produced `.ct` container.  A new
  `test_recorded_trace_via_ct_print_json` records `flow_test.ak` and
  pipes the resulting `.ct` container through `ct-print --json`,
  asserting that the canonical flow values (10 / 32 / 42 / 84 / 94)
  surface in the textual rendering.  The test skips gracefully (with
  a printed `SKIP:` line) when `ct-print` is not present (e.g. when
  the crate is built outside the metacraft workspace).
* New env-var integration tests
  (`test_env_out_dir_used_when_flag_omitted`,
  `test_env_disabled_skips_recording`, `test_format_flag_rejected_by_clap`)
  cover the convention §5 surface, plus
  `test_no_format_flag_in_help` and `test_help_mentions_ct_print` in
  `tests/test_ctfs_audit.rs` lock in the §4 invariants.
* `tests/test_ctfs_audit.rs` was updated to drop the
  `TraceEventsFileFormat` parameter from its `record_flow_test`
  helper and `recorder::record(...)` call sites, and the old
  `test_ctfs_format_advertised_in_help` test (which would have locked
  in the regression) was replaced with the post-fix
  `test_no_format_flag_in_help` and `test_help_mentions_ct_print`
  tests.
* `tests/verify-cli-convention-no-silent-skip.sh` was added as a
  shell-level guard that runs the binary's `--help`, asserts
  `--format` and `CODETRACER_FORMAT` are absent, asserts the standard
  flags (`--out-dir`, `--version`) are present, and asserts the
  `CODETRACER_CARDANO_RECORDER_OUT_DIR` env var is referenced in
  source.  A `Justfile` was added at repo root to wire it into
  `just lint` / `just test`.

References:

* [`codetracer-specs/Recorder-CLI-Conventions.md`](../codetracer-specs/Recorder-CLI-Conventions.md) §4 (CTFS-only) and §5 (env vars).
* [`codetracer-specs/Repo-Requirements.md`](../codetracer-specs/Repo-Requirements.md) §2.2 (CLI compliance) and §2.3 (trace format compatibility).
* [`codetracer-cairo-recorder` commit `2710b5e`](../codetracer-cairo-recorder/) — the Cairo recorder follow-up that this work mirrors.
