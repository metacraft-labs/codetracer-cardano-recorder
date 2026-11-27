set shell := ["bash", "-euo", "pipefail", "-c"]

# Default recipe — list available recipes.
default:
  @just --list

build:
  cargo build --locked

build-release:
  cargo build --release --locked

# The Cargo.lock check runs first: when the lock is stale it names the cause
# and the remedy, where `cargo test --locked` would only refuse the lock.
test: build-ct-print
  bash tests/verify-cargo-lock-matches-trace-format.sh
  cargo test --locked
  bash tests/verify-cli-convention-no-silent-skip.sh

t: test

# `test` runs the same `cargo test`, after the same ct-print build.
# ci-test-audit: covered-by test
test-rust: build-ct-print
  cargo test --locked

# Build `ct-print` in the sibling codetracer-trace-format-nim checkout. The
# tests read their recordings back with it at the fixed path
# `../codetracer-trace-format-nim/ct-print`. Cargo rebuilds the recorder's
# writer from that checkout's sources by itself, and nothing rebuilds this
# binary, so without this recipe an old one refuses what the writer now writes,
# and an absent one turns those tests into skips. The sibling's `buildCtPrint`
# nimble task owns the compile flags and resolves that package's Nim
# dependencies. The compile runs every time, and is the staleness check.
build-ct-print:
  cd ../codetracer-trace-format-nim && nimble buildCtPrint

# `test` and `lint` run the same script.
# ci-test-audit: covered-by test
verify-cli-convention:
  bash tests/verify-cli-convention-no-silent-skip.sh

# The committed Cargo.lock must be exactly the lock of the trace-format tree
# repro.lock pins and of the sibling checkout (see the script header).
# `test` runs the same script first.
# ci-test-audit: covered-by test
verify-cargo-lock:
  bash tests/verify-cargo-lock-matches-trace-format.sh

lint:
  cargo fmt --check
  cargo clippy --locked --all-targets -- -D warnings
  bash tests/verify-cli-convention-no-silent-skip.sh
  bash tests/verify-package-no-vacuous-pass.sh

format:
  cargo fmt

fmt: format

# Recorder-specific CI prep, run by the shared reusable-recorder-ci workflow
# (nixos-modules) after setup-dev-env, before lint/test. Installs the Nim
# dependencies of the codetracer-trace-format-nim sibling. Shebang recipe so
# shell state persists across lines.
#
# The trace writer's C ABI archive is NOT built here. The
# codetracer_trace_writer_nim crate's build script compiles and links it, with
# the flags the library requires (one process heap, --threads:off). An archive
# built here would not be the one linked, and its flags could only drift.
prepare-ci:
  #!/usr/bin/env bash
  set -euo pipefail
  (
    cd "${GITHUB_WORKSPACE}/../codetracer-trace-format-nim"
    nimble install -y stew results
  )

# --- M13: Packaging UX Standardization ---
# These recipes implement Repo-Requirements.md §2.5. The OS-packaged
# recorders share a uniform packaging surface: `bump-version` rewrites
# Cargo.toml + packaging/recorder-metadata.yml; `build-package` shells
# out to packaging/build-all.sh with the requested channel selector;
# `verify-package` introspects the produced artifact.
#
# The shebang recipes below let Just hand the recipe body to the
# interpreter verbatim — Just's `{{...}}` interpolation still runs
# (so the `component` / `channel` arguments are substituted) but no
# other token in the body is parsed by Just.

# Bump the version in Cargo.toml + packaging/recorder-metadata.yml.
# Argument: major | minor | patch | MAJOR.MINOR.PATCH
bump-version component:
    #!/usr/bin/env python3
    import re, pathlib
    component = "{{component}}"
    root = pathlib.Path(".").resolve()
    cargo = root / "Cargo.toml"
    metadata = root / "packaging" / "recorder-metadata.yml"

    def current():
        if cargo.exists():
            m = re.search(r'^version\s*=\s*"(\d+\.\d+\.\d+)"', cargo.read_text(), re.MULTILINE)
            if m:
                return m.group(1)
        if metadata.exists():
            m = re.search(r'^\s*version:\s*(\d+\.\d+\.\d+)', metadata.read_text(), re.MULTILINE)
            if m:
                return m.group(1)
        return "0.1.0"

    def bump(c, v):
        if re.match(r'^\d+\.\d+\.\d+$', c):
            return c
        a, b, p = map(int, v.split("."))
        if c == "major":
            return f"{a+1}.0.0"
        if c == "minor":
            return f"{a}.{b+1}.0"
        if c == "patch":
            return f"{a}.{b}.{p+1}"
        raise SystemExit(f"unknown component {c!r}")

    cur = current()
    new = bump(component, cur)
    print(f"[bump-version] {cur} -> {new}")
    if cargo.exists():
        text = cargo.read_text()
        text = re.sub(r'(^version\s*=\s*")\d+\.\d+\.\d+(")', rf'\g<1>{new}\g<2>', text, count=1, flags=re.MULTILINE)
        cargo.write_text(text)
    if metadata.exists():
        text = metadata.read_text()
        text = re.sub(r'(^\s*version:\s*)\d+\.\d+\.\d+', rf'\g<1>{new}', text, count=1, flags=re.MULTILINE)
        metadata.write_text(text)

# Build a release artifact for the given channel.
# Channels: homebrew nix aur deb rpm scoop chocolatey
build-package channel:
    CT_PACKAGING_CHANNELS={{channel}} bash packaging/build-all.sh

# Verify the packaging for one channel: the source recipe for homebrew, nix,
# aur, scoop and chocolatey, the built artifact for deb and rpm.
#
# Strict by default: a missing verification tool, or no .deb/.rpm to inspect,
# fails the check, because a check that did not run must not report OK. On a
# machine without the tools, CT_VERIFY_STRICT=0 turns each such failure into a
# `SKIPPED` line and exit 0; it never prints OK for a check it skipped.
#
# `dist` is the directory holding the built .deb/.rpm (deb and rpm only);
# empty means packaging/dist/<channel>, where `build-package` puts them.
verify-package channel dist="":
    #!/usr/bin/env python3
    import json, os, re, shutil, subprocess, sys
    from pathlib import Path
    ch = "{{channel}}"
    dist_arg = "{{dist}}"
    root = Path(".").resolve()
    pkg = root / "packaging"
    strict = os.environ.get("CT_VERIFY_STRICT", "1") != "0"

    def run(cmd):
        # Show the tool's own output when it rejects the input.
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            sys.stdout.write(r.stdout)
            sys.stderr.write(r.stderr)
            print(f"::error::{ch}: `{' '.join(cmd)}` exited {r.returncode}")
            sys.exit(1)

    def cannot_check(why):
        # The check cannot run. Strict: fail. Lenient: say so, never OK.
        if strict:
            print(f"::error::{ch}: {why} (set CT_VERIFY_STRICT=0 to skip)")
            sys.exit(1)
        print(f"[verify] {ch} SKIPPED: {why}")
        sys.exit(0)

    def need(tool):
        if not shutil.which(tool):
            cannot_check(f"{tool} not on PATH")

    def built(ext):
        d = Path(dist_arg) if dist_arg else pkg / "dist" / ch
        found = sorted(d.glob(f"*.{ext}")) if d.is_dir() else []
        if not found:
            cannot_check(f"no .{ext} in {d} — run `just build-package {ch}` first")
        return found

    if ch == "homebrew":
        formula = next(pkg.glob("homebrew/*.rb"), None)
        if formula is None:
            print("::error::no homebrew formula"); sys.exit(1)
        need("ruby")
        run(["ruby", "-c", str(formula)])
        print(f"[verify] homebrew {formula.name} OK")
    elif ch == "nix":
        drv = pkg / "nix" / "default.nix"
        if not drv.exists():
            print("::error::no nix derivation"); sys.exit(1)
        need("nix-instantiate")
        # Parse the expression: catches syntax errors without fetching nixpkgs.
        run(["nix-instantiate", "--parse", str(drv)])
        print(f"[verify] nix {drv.name} OK")
    elif ch == "aur":
        pkgbuild = pkg / "aur" / "PKGBUILD"
        if not pkgbuild.exists():
            print("::error::no PKGBUILD"); sys.exit(1)
        run(["bash", "-n", str(pkgbuild)])
        print(f"[verify] aur {pkgbuild.name} OK")
    elif ch == "deb":
        debs = built("deb")
        need("dpkg-deb")
        for d in debs:
            run(["dpkg-deb", "-I", str(d)])
            print(f"[verify] deb {d.name} OK")
    elif ch == "rpm":
        rpms = built("rpm")
        need("rpm")
        for r in rpms:
            run(["rpm", "-qpi", str(r)])
            print(f"[verify] rpm {r.name} OK")
    elif ch == "scoop":
        sf = next(pkg.glob("scoop/*.json"), None)
        if sf is None:
            print("::error::no scoop manifest"); sys.exit(1)
        data = json.loads(sf.read_text())
        for k in ("version", "bin"):
            if k not in data:
                print(f"::error::scoop manifest missing {k}"); sys.exit(1)
        print(f"[verify] scoop {sf.name} OK")
    elif ch == "chocolatey":
        import xml.etree.ElementTree as ET
        nuspec = next(pkg.glob("chocolatey/*.nuspec"), None)
        if nuspec is None:
            print("::error::no nuspec"); sys.exit(1)
        tree = ET.parse(nuspec)
        root_el = tree.getroot()
        ns_m = re.match(r"\{.*\}", root_el.tag)
        ns = ns_m.group(0) if ns_m else ""
        meta = root_el.find(f"{ns}metadata")
        for field in ("id", "version", "authors", "description"):
            if meta is None or meta.find(f"{ns}{field}") is None:
                print(f"::error::nuspec missing {field}"); sys.exit(1)
        print(f"[verify] chocolatey {nuspec.name} OK")
    else:
        print(f"::error::unknown channel {ch!r}"); sys.exit(1)

# Per-channel build shortcuts.
build-homebrew:
    just build-package homebrew

build-nix:
    just build-package nix

build-aur:
    just build-package aur

build-deb:
    just build-package deb

build-rpm:
    just build-package rpm

build-scoop:
    just build-package scoop

build-chocolatey:
    just build-package chocolatey

# Per-channel verify shortcuts. CI runs the five source-only channels through
# `verify-packaging-sources` (ci.yml), and `verify-deb` / `verify-rpm` on the
# package that publish-deb.yml / publish-rpm.yml just built, before upload.
verify-homebrew:
    just verify-package homebrew

verify-nix:
    just verify-package nix

verify-aur:
    just verify-package aur

verify-deb dist="packaging/dist/deb":
    just verify-package deb '{{dist}}'

verify-rpm dist="packaging/dist/rpm":
    just verify-package rpm '{{dist}}'

verify-scoop:
    just verify-package scoop

verify-chocolatey:
    just verify-package chocolatey

# The channels whose packaging is source only, so they verify without a build.
verify-packaging-sources: verify-homebrew verify-nix verify-aur verify-scoop verify-chocolatey

# Install genuine canonical checks and preserve matching Repro publication hooks.
# Pass the actual matching engine explicitly; unqualified protocols fail.
[positional-arguments]
install-hooks REPRO:
    python3 tools/install-canonical-hooks.py --repro "$1"
