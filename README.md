## codetracer-cardano-recorder

A recorder for Cardano/Aiken smart contracts that produces [CodeTracer](https://github.com/metacraft-labs/CodeTracer) traces.

> **Note:** This project is in early development. APIs and trace formats may change.
> We welcome contributions and discussion!

### Overview

`codetracer-cardano-recorder` compiles Aiken source files to UPLC
(Untyped Plutus Lambda Calculus), evaluates them through the CEK machine,
and captures step-level execution traces in the canonical CodeTracer
CTFS multi-stream format. It also supports replaying on-chain Cardano
transactions via the Blockfrost API.

### Building

```bash
cargo build
```

Or enter the Nix dev shell first:

```bash
nix develop
cargo build
```

### Usage

#### Record an Aiken program

```bash
codetracer-cardano-recorder record <file.ak> --out-dir <dir>
```

Parses the `.ak` source file, evaluates variable assignments through the
UPLC CEK machine, captures the execution trace, and writes a CTFS trace
bundle to `--out-dir`.

The recorder always writes traces in the canonical CodeTracer CTFS
multi-stream format (a single `.ct` container plus
`trace_metadata.json` / `trace_paths.json` sidecars). There is no
`--format` flag — see "Converting traces" below for human-readable
output.

#### Replay an on-chain Cardano transaction

```bash
codetracer-cardano-recorder replay --tx-hash <tx-hash> [--blockfrost-key <key>]
```

Fetches the transaction via the Blockfrost API, extracts the Plutus
script and its arguments (datum, redeemer, script context), and traces
the validator execution through the UPLC CEK machine.

#### Converting traces to JSON / text

The recorder is CTFS-only. To convert a recorded `.ct` bundle to a
human-readable form, use `ct print` from
[`codetracer-trace-format-nim`](https://github.com/metacraft-labs/codetracer-trace-format-nim):

```bash
ct-print --json <recording-dir>/<program>.ct
```

`ct-print` accepts `--json`, `--json-events`, `--summary`, and
`--follow` modes; see its `--help` for details. This conversion path
is the canonical way to produce textual oracles for golden-snapshot
tests, debugging, and interop with non-CodeTracer tools.

### Architecture

The recorder is structured around the following modules in `src/`:

| Module | Purpose |
|---|---|
| `main.rs` | CLI entry point (clap) |
| `recorder.rs` | Top-level recording orchestration |
| `tracer.rs` | Step-level trace capture during CEK machine evaluation |
| `source_map.rs` | Mapping between UPLC nodes and Aiken source locations |
| `blockfrost.rs` | Blockfrost API client for fetching on-chain transactions |
| `transaction.rs` | Transaction reconstruction and script argument extraction |
| `lib.rs` | Public library API |

### Testing

```bash
cargo test
```

Test programs live in:

- `test-programs/aiken/` -- Aiken smart contract examples
- `test-programs/uplc/` -- Raw UPLC programs

### Environment variables

The recorder respects the standard CodeTracer recorder env-var contract
defined in `Recorder-CLI-Conventions.md` §5:

| Variable | CLI equivalent | Description |
|---|---|---|
| `CODETRACER_CARDANO_RECORDER_OUT_DIR` | `--out-dir` | Fallback output directory when `--out-dir` is omitted. The CLI flag always wins. |
| `CODETRACER_CARDANO_RECORDER_DISABLED` | — | Set to `1` or `true` to run the recorder in pass-through mode (no trace artefacts written). |
| `CODETRACER_CARDANO_RECORDER_LOG_LEVEL` | — | Recorder log verbosity (advisory; the Cardano recorder currently logs to stderr unconditionally). |
| `BLOCKFROST_API_KEY` | `--blockfrost-key` | Blockfrost API key, required for the `replay` subcommand. |

### Contributing

We'd be very happy if the community finds this useful, and if anyone wants to:

* Use and test the Cardano/Aiken support or CodeTracer.
* Provide feedback and discuss alternative implementation ideas: in the issue tracker, or in our [discord](https://discord.gg/qSDCAFMP).
* Contribute code to enhance the Cardano support of CodeTracer.
* Provide [sponsorship](https://opencollective.com/codetracer), so we can hire dedicated full-time maintainers for this project.

### Legal info

LICENSE: MIT

Copyright (c) 2025 Metacraft Labs Ltd
