## Reprobuild dev env + build recipe for codetracer-cardano-recorder.
##
## Mirrors the dev shell declared in ``flake.nix`` (Linux/macOS) and
## the Windows DIY env declared in ``env.ps1``. ``repro build`` /
## ``repro test`` reproduce the same artefacts and the same test set
## that ``just build`` / ``just test`` produce today.
##
## Per ``codetracer-specs/Repo-Requirements.md`` §2.8 the recipe
## expresses build and test execution NATIVELY through typed-tool
## edges (`cargo.build`, `cargo.test`). It does NOT delegate to
## `shell(command = "bash scripts/...")` wrappers — delegation
## defeats the engine's incremental-build, action-cache, per-test
## invalidation, and the CI sharding the engine grows into per
## ``reprobuild-specs/CI-Sharding.md``.
##
## On Windows the recipe drives real reprobuild tool provisioning via
## the tarball entries the ``uses:`` packages declare (cargo, rustc,
## rustfmt, nim, nimble, capnp). On Linux/macOS the Nix flake
## continues to supply the same toolchain. Either path produces
## byte-equivalent build outputs and the same test pass/fail set —
## CI cross-checks this through the side-by-side `ci.yml` (nix) +
## `ci-reprobuild.yml` (reprobuild) flow per Repo-Requirements §2.9.
##
## Test sharding shape: the recipe's `test` collection currently
## carries one whole-binary execute edge per cargo test binary. The
## ct-test-runner / `repro test --shard k/N` extension specified in
## reprobuild-specs/CI-Sharding.md will fan each whole-binary edge
## into N per-test execute edges by enumerating the binary's
## libtest `--list --format=terse` output at execution time, so this
## recipe does not change when sharding lands.

import repro_project_dsl

package codetracer_cardano_recorder:
  uses:
    # Rust toolchain — declared by version so the tarball-direct
    # provisioning entries in repro_dsl_stdlib/packages/cargo.nim /
    # rustc.nim / rustfmt.nim resolve on Windows (single Rust
    # standalone-distribution tarball pinned at 1.92.0; see
    # cargo.nim's `tarball url = ...` provisioning entry). On
    # Linux/macOS the nix flake supplies the same versions.
    "rustc >=1.85"
    "cargo >=1.85"
    "just >=1"

    # Nim toolchain — codetracer_trace_writer_nim's build.rs compiles
    # a static library at cargo build time. tarball-provisioned on
    # Windows (nim.nim's tarball pin: nim 2.2.10).
    "nim >=2.2 <3.0"
    "nimble"

    # Cap'n Proto schema compiler used by the recorder's build.rs.
    # tarball-provisioned on Windows (capnp.nim's tarball pin: 1.4.0).
    "capnp"

    # libzstd headers + library, needed when linking the Nim FFI
    # static library into the cargo build.
    "zstd"

    # pkg-config + OpenSSL — openssl-sys consults pkg-config to find
    # OpenSSL on Linux/macOS. The Windows build uses the rustls-tls
    # feature instead so neither is on the windows toolchain floor.
    when defined(linux):
      # Nim staticlib builds invoked from cargo expect a GNU archiver on
      # Linux. Use gcc so Nim selects ``ar`` instead of ``llvm-ar``.
      "gcc"
    when defined(macosx):
      # Cargo build scripts look for ``cc`` by default; pass ``CC=clang``
      # below and make clang part of the macOS dev environment.
      "clang"
    when not defined(windows):
      "pkg-config"
      "openssl"

  # The primary shipping binary. ``executable`` declarations register
  # the on-disk artefact name with the implicit-name resolver so
  # ``repro build codetracer-cardano-recorder`` selects this edge
  # directly.
  executable codetracerCardanoRecorder:
    name: "codetracer-cardano-recorder"

  devEnv:
    activity "default"

  build:
    # ---- Primary build edge (the `default` collection) ----------------
    #
    # Native cargo build for the recorder binary. Enrolled into the
    # conventional ``default`` collection per
    # reprobuild-specs/Build-Graph-Collections.md §"`default`"; this
    # makes ``repro build`` (no positional target) materialise this
    # edge's closure.
    const binarySuffix = (when defined(windows): ".exe" else: "")
    const recorderBinary =
      "target/release/codetracer-cardano-recorder" & binarySuffix
    let cargoCompilerEnv: seq[(string, string)] =
      when defined(windows): @[]
      elif defined(macosx): @[("CC", "clang")]
      else: @[("CC", "gcc")]

    let recorderBuild = cargo.build(
      locked = true,
      release = true,
      actionId = "codetracer-cardano-recorder.cargo-build",
      extraInputs = @[
        "Cargo.toml", "Cargo.lock",
        "src", "build.rs"
      ],
      extraOutputs = @[recorderBinary],
      extraEnv = cargoCompilerEnv)
    discard collect("default", @[recorderBuild])

    # ---- Test-binary build + run edges (the `test` collection) -------
    #
    # Two-stage shape per Repo-Requirements.md §2.8:
    #   1. `cargo.test(noRun = true, ...)` builds the integration-test
    #      + lib-unit-test binaries. Cargo emits each one under
    #      `target/debug/deps/<crate>-<hash>` (the hash floats with
    #      compiler-input content so we cannot pin it at recipe-eval
    #      time; the engine tracks the `target/debug/deps/` directory
    #      as the build edge's effect set instead).
    #   2. `cargo.test(noRun = false, ...)` runs every test binary in
    #      one cargo invocation. The execute edge depends on the
    #      build edge so the engine only re-runs tests when an input
    #      changed since the last successful execution.
    #
    # Per-test execute edges (one per libtest test) fall out
    # automatically once the ct-test-runner cargo adapter lands per
    # reprobuild-specs/Test-Edges-And-Parallel-Runner.milestones.org
    # §M4. Until then the whole-binary edge is the natural unit; CI
    # parallelism happens across the multi-recorder workspace, not
    # within a single recorder's test set.

    let testsBuild = cargo.test(
      locked = true,
      noRun = true,
      actionId = "codetracer-cardano-recorder.cargo-test-build",
      extraInputs = @[
        "Cargo.toml", "Cargo.lock",
        "src", "build.rs", "tests"
      ],
      extraOutputs = @["target/debug/deps"],
      extraEnv = cargoCompilerEnv)

    let testsRun = cargo.test(
      locked = true,
      actionId = "codetracer-cardano-recorder.cargo-test-run",
      after = @[testsBuild.action],
      extraInputs = @[
        "Cargo.toml", "Cargo.lock",
        "src", "tests",
        "target/debug/deps"
      ],
      extraEnv = cargoCompilerEnv)

    discard collect("test", @[testsRun.action])
