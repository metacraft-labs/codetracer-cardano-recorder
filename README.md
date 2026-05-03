## codetracer-cardano-recorder

A recorder for Cardano/Aiken smart contracts that produces [CodeTracer](https://github.com/metacraft-labs/CodeTracer) traces.

> **Note:** This project is in early development. APIs and trace formats may change.
> We welcome contributions and discussion!

### Overview

`codetracer-cardano-recorder` compiles Aiken source files to UPLC
(Untyped Plutus Lambda Calculus), evaluates them through the CEK machine,
and captures step-level execution traces in the CodeTracer trace format.
It also supports replaying on-chain Cardano transactions via the
Blockfrost API.

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
codetracer-cardano-recorder record <file.ak> --out-dir <dir> [--format ctfs|binary|json]
```

Parses the `.ak` source file, evaluates variable assignments through the
UPLC CEK machine, captures the execution trace, and writes CodeTracer
trace files to `--out-dir`. The default `ctfs` format is the canonical
CodeTracer multi-stream container; `binary` is the legacy CBOR+Zstd
format and `json` is intended for debugging.

#### Replay an on-chain Cardano transaction

```bash
codetracer-cardano-recorder replay <tx-hash> --out-dir <dir> [--api-key <key>]
```

Fetches the transaction via the Blockfrost API, extracts the Plutus
script and its arguments (datum, redeemer, script context), and traces
the validator execution through the UPLC CEK machine.

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

| Variable | Description |
|---|---|
| `BLOCKFROST_API_KEY` | Blockfrost API key, required for the `replay` subcommand. Can also be passed via `--api-key`. |

### Contributing

We'd be very happy if the community finds this useful, and if anyone wants to:

* Use and test the Cardano/Aiken support or CodeTracer.
* Provide feedback and discuss alternative implementation ideas: in the issue tracker, or in our [discord](https://discord.gg/qSDCAFMP).
* Contribute code to enhance the Cardano support of CodeTracer.
* Provide [sponsorship](https://opencollective.com/codetracer), so we can hire dedicated full-time maintainers for this project.

### Legal info

LICENSE: MIT

Copyright (c) 2025 Metacraft Labs Ltd
