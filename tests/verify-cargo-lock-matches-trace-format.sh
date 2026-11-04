#!/usr/bin/env bash
# Verify that the committed `Cargo.lock` is exactly the lock cargo writes for
# the sibling `codetracer-trace-format` tree — both the revision `repro.lock`
# pins and the checkout that sits beside this repo (no silent skip: every
# check either passes, fails loudly, or SKIPs loudly).
#
# Why this exists
# ---------------
# `Cargo.toml` reaches three trace-format crates by sibling path
# (`../codetracer-trace-format/codetracer_trace_{types,writer_nim,reader}`).
# A path dependency records no source or revision in `Cargo.lock`, but cargo
# DOES write the path crate's own dependency edges into it, so the lock is
# valid for exactly the trace-format tree it was written against. When the
# sibling gains an edge, `cargo build --locked` (`just build-release`, and
# codetracer's `scripts/build-siblings.sh`) stops with "the lock file ...
# needs to be updated but --locked was passed", while `repro.lock` still names
# the old revision — nothing in this repo said the lock had fallen behind
# (codetracer-pm issues/2026-10-22-cardano-circom-and-leo-recorder-cargo-locks-
# are-stale-against-the-trace-format-agents-tip.md).
#
# This is a compact, single-lock adaptation of codetracer's
# `scripts/test-sibling-cargo-locks.sh`; its header has the full rationale
# and the measurements behind each choice below.
#
# What is checked
# ---------------
#   1. LOCK MATCHES THE PIN — against a `git archive` of the trace-format
#      revision `repro.lock` pins (so neither the sibling's HEAD nor its
#      working tree influences the verdict). Catches the pin moved without
#      the lock refreshed, and the lock refreshed without the pin moving.
#   2. LOCK MATCHES THE CHECKOUT — against the sibling as it sits on disk,
#      which is the tree `cargo build --locked` actually reads.
#
# Each check is an EXACT re-resolution, not `cargo metadata --locked`:
# `--locked` refuses a lock MISSING an edge but accepts one carrying an edge
# the tree no longer has (which any unlocked cargo run then rewrites). So the
# committed lock is copied into a scratch shadow of this repo, `cargo update
# --workspace --offline` re-resolves it there (the same minimal re-resolution
# an unlocked `cargo build` performs;
# https://doc.rust-lang.org/cargo/commands/cargo-update.html#update-options),
# and the result must be byte-identical to the committed lock.
#
# The scratch layout puts the tree under test where the manifest's relative
# path looks for it — cargo joins `..` lexically, not through symlinks:
#
#     <tmp>/<check>/<this repo>                a symlink per top-level entry
#                                              of this checkout, except
#                                              `Cargo.lock` (a COPY) and
#                                              `target` (left out)
#     <tmp>/<check>/codetracer-trace-format    `git archive` of the pin
#                                              (check 1), or a symlink to
#                                              the sibling checkout (check 2)
#
# The committed `Cargo.lock` is never modified: cargo only writes the scratch
# copy, and the lock is compared before and after the run (and restored, with
# a failure, if it changed).
#
# Reading the pin
# ---------------
# `repro.lock` is TOML whose `deps` is a one-line array of inline tables. The
# dev shell has no python3, so the record is selected with grep: each inline
# table is isolated, and the one whose `name` KEY is exactly
# `codetracer-trace-format` is taken. Matching the bare string would also hit
# the self record's `depends = "...,codetracer-trace-format,..."`. Anything
# other than exactly one such record with one 40-hex `revision` is a hard
# failure naming the parse, never a statement about the lock.
#
# Skips
# -----
# A missing sibling checkout, a sibling lacking the pinned revision, or a
# registry cache that cannot resolve offline each print a SKIP naming what is
# missing and the command that fixes it; the run then ends in SKIP, never OK.
# CT_SIBLING_CARGO_LOCKS_STRICT=1 (the variable codetracer's guard reads)
# turns every skip into a failure. CT_TRACE_FORMAT_CHECKOUT overrides where
# the sibling checkout is looked for.
#
# Wire-up: see `Justfile` (`just test` runs this script) and `repro.nim`
# (the `verify-cargo-lock` execute edge in the `test` collection).
#
# Exit codes:
#   0  every check passed, or (non-strict) a check was skipped loudly
#   1  the lock is stale, a check could not run for a reason that is not a
#      skip, or (strict) a check was skipped

set -euo pipefail

RECORDER_NAME="codetracer-cardano-recorder"
SIBLING_NAME="codetracer-trace-format"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
WORKSPACE_ROOT="$(cd "${REPO_ROOT}/.." && pwd)"
SIBLING_DIR="${CT_TRACE_FORMAT_CHECKOUT:-${WORKSPACE_ROOT}/${SIBLING_NAME}}"
STRICT="${CT_SIBLING_CARGO_LOCKS_STRICT:-0}"
CARGO="${CARGO:-cargo}"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

for tool in "${CARGO}" git tar diff cmp grep mktemp; do
  command -v "${tool}" >/dev/null 2>&1 || fail \
    "'${tool}' is required and is not on PATH. Run inside the dev shell: 'nix develop --command just test'. NOTHING about Cargo.lock has been established by this run."
done

[[ -f "${REPO_ROOT}/repro.lock" ]] || fail "${REPO_ROOT}/repro.lock does not exist."
[[ -f "${REPO_ROOT}/Cargo.lock" ]] || fail "${REPO_ROOT}/Cargo.lock does not exist."

# ---------------------------------------------------------------------------
# The pinned revision (see "Reading the pin" above)
# ---------------------------------------------------------------------------

DEPS_LINES="$(grep -cE '^deps = \[' "${REPO_ROOT}/repro.lock" || true)"
[[ "${DEPS_LINES}" == "1" ]] || fail \
  "expected exactly one 'deps = [' line in repro.lock, found ${DEPS_LINES}. The lock layout changed; update this check's parser. This is a parse failure, NOT a statement about Cargo.lock."

RECORDS="$(grep -E '^deps = \[' "${REPO_ROOT}/repro.lock" |
  grep -oE '\{[^{}]*\}' |
  grep -E "(\{ *|, *)name = \"${SIBLING_NAME}\" *(,|\})" || true)"
RECORD_COUNT="$(grep -c . <<< "${RECORDS}" || true)"
[[ "${RECORD_COUNT}" == "1" ]] || fail \
  "expected exactly one '${SIBLING_NAME}' dep record in repro.lock, found ${RECORD_COUNT}. If the dep was renamed or dropped, update this check. This is a parse failure, NOT a statement about Cargo.lock."

REVS="$(grep -oE '(\{ *|, *)revision = "[0-9a-f]{40}"' <<< "${RECORDS}" |
  grep -oE '[0-9a-f]{40}' || true)"
[[ "$(grep -c . <<< "${REVS}" || true)" == "1" ]] || fail \
  "the '${SIBLING_NAME}' record in repro.lock does not carry exactly one 40-hex revision:
${RECORDS}"
PINNED_REV="${REVS}"
echo "repro.lock pins ${SIBLING_NAME} = ${PINNED_REV}"

# ---------------------------------------------------------------------------
# Scratch shadow + exact re-resolution
# ---------------------------------------------------------------------------

FAILED=0
SKIPPED=0
STALE=()

skip_note() {
  SKIPPED=$((SKIPPED + 1))
  echo "SKIP: $*" >&2
}

failure_note() {
  FAILED=$((FAILED + 1))
  echo "FAIL: $*" >&2
}

SCRATCH="$(mktemp -d)"
trap 'rm -rf "${SCRATCH}"' EXIT HUP INT TERM
cp "${REPO_ROOT}/Cargo.lock" "${SCRATCH}/committed.lock"

# make_shadow <dest> — see the layout in the header.
make_shadow() {
  local dest="$1" entry name
  mkdir -p "${dest}"
  shopt -s nullglob dotglob
  for entry in "${REPO_ROOT}"/*; do
    name="$(basename "${entry}")"
    case "${name}" in
      Cargo.lock) cp "${entry}" "${dest}/${name}" ;;
      target) ;;
      *) ln -s "${entry}" "${dest}/${name}" ;;
    esac
  done
  shopt -u nullglob dotglob
}

# resolve_against <label> <tree> <description>
resolve_against() {
  local label="$1" tree="$2" what="$3"
  local ws="${SCRATCH}/${label}"
  local shadow="${ws}/${RECORDER_NAME}"
  local out rc=0
  make_shadow "${shadow}"
  [[ -e "${ws}/${SIBLING_NAME}" ]] || ln -s "${tree}" "${ws}/${SIBLING_NAME}"
  # No --locked: see "exact re-resolution" in the header.
  out="$(cd "${shadow}" && "${CARGO}" update --workspace --offline 2>&1 >/dev/null)" || rc=$?
  if [[ "${rc}" -ne 0 ]]; then
    case "${out}" in
      *"--offline was specified"* | *"attempting to make an HTTP request"* | *"failed to download"* | *"no matching package"*"offline"*)
        skip_note "Cargo.lock against ${what} could not be re-resolved offline: the local cargo registry cache lacks a crate. This is NOT a verdict on the lock. Fill the cache with: (cd '${REPO_ROOT}' && cargo fetch --locked)
${out}"
        ;;
      *)
        failure_note "cargo update --workspace failed against ${what} for a reason that is NOT a stale lock (exit ${rc}). Read cargo's own output:
${out}"
        ;;
    esac
    return 0
  fi
  if cmp -s "${SCRATCH}/committed.lock" "${shadow}/Cargo.lock"; then
    echo "ok: Cargo.lock is exactly the lock of ${what}"
    return 0
  fi
  STALE+=("${label}")
  echo "STALE: Cargo.lock is NOT the lock of ${what}; cargo re-resolves it as:" >&2
  diff -u --label "committed Cargo.lock" --label "re-resolved against ${what}" \
    "${SCRATCH}/committed.lock" "${shadow}/Cargo.lock" | head -n 60 >&2 || true
}

# ---------------------------------------------------------------------------
# Check 1 — the pinned tree, from the sibling's objects only
# ---------------------------------------------------------------------------

PINNED_OK=0
if [[ ! -e "${SIBLING_DIR}/.git" ]]; then
  skip_note "no ${SIBLING_NAME} checkout at ${SIBLING_DIR} (clone it beside this repo, or set CT_TRACE_FORMAT_CHECKOUT). Cargo.lock was NOT resolved against the pinned revision ${PINNED_REV}, nor against a checkout."
elif ! git -C "${SIBLING_DIR}" cat-file -e "${PINNED_REV}^{commit}" 2>/dev/null; then
  skip_note "the ${SIBLING_NAME} checkout at ${SIBLING_DIR} does not have the pinned revision ${PINNED_REV}, so Cargo.lock was NOT resolved against it. Run: git -C '${SIBLING_DIR}' fetch origin ${PINNED_REV}"
else
  mkdir -p "${SCRATCH}/pinned-tree"
  if git -C "${SIBLING_DIR}" archive --format=tar "${PINNED_REV}" | tar -x -C "${SCRATCH}/pinned-tree"; then
    echo "check 1: against the pinned tree ${SIBLING_NAME}@${PINNED_REV} (git archive)"
    resolve_against pinned "${SCRATCH}/pinned-tree" "${SIBLING_NAME}@${PINNED_REV} (pinned)"
    PINNED_OK=1
  else
    failure_note "could not materialise ${SIBLING_NAME}@${PINNED_REV} with git archive from ${SIBLING_DIR}."
  fi
fi

# ---------------------------------------------------------------------------
# Check 2 — the checkout on disk (what `cargo build --locked` reads)
# ---------------------------------------------------------------------------

DISK_REV=""
DISK_DESC=""
if [[ -e "${SIBLING_DIR}/.git" ]]; then
  if DISK_REV="$(git -C "${SIBLING_DIR}" rev-parse --verify -q HEAD)"; then
    DISK_DESC="${SIBLING_NAME} checkout at ${SIBLING_DIR} (HEAD ${DISK_REV}"
    if [[ -n "$(git -C "${SIBLING_DIR}" status --porcelain --untracked-files=no 2>/dev/null)" ]]; then
      DISK_DESC="${DISK_DESC} + uncommitted changes"
    fi
    DISK_DESC="${DISK_DESC})"
    echo "check 2: against the ${DISK_DESC}"
    resolve_against disk "$(cd "${SIBLING_DIR}" && pwd)" "the ${DISK_DESC}"
  else
    failure_note "${SIBLING_DIR} looks like a git checkout but has no HEAD."
  fi
fi

# The committed lock must be byte-identical to before the run.
if ! cmp -s "${SCRATCH}/committed.lock" "${REPO_ROOT}/Cargo.lock"; then
  cp "${SCRATCH}/committed.lock" "${REPO_ROOT}/Cargo.lock"
  failure_note "Cargo.lock was MODIFIED during this run (and has been restored). Nothing here may write the committed lock; re-examine this check."
fi

# ---------------------------------------------------------------------------
# Verdict
# ---------------------------------------------------------------------------

print_remedy() {
  cat >&2 <<EOF
    repro lock update ${SIBLING_NAME} --to $1
        (also move codetracer-trace-format-nim with it when the range
         changes what ct-print must read; the tests dump traces with it)
    nix develop --command cargo update --workspace --offline
        and review that the Cargo.lock diff touches only the dependency
        edges of the ${SIBLING_NAME} crates
    bash tests/verify-cargo-lock-matches-trace-format.sh
EOF
}

for label in "${STALE[@]+"${STALE[@]}"}"; do
  FAILED=$((FAILED + 1))
  if [[ "${label}" == "pinned" ]]; then
    cat >&2 <<EOF

FAIL: the committed Cargo.lock does not match the ${SIBLING_NAME} revision
      repro.lock pins (${PINNED_REV}).

Either the lock was refreshed against another trace-format tree without
moving the pin, or the pin moved without refreshing the lock. Move the pin and
refresh the lock TOGETHER, in one commit:
EOF
    print_remedy "<sha>"
  else
    cat >&2 <<EOF

FAIL: the committed Cargo.lock does not match the sibling checkout that
      \`cargo build --locked\` actually reads.

  pinned (repro.lock)  ${SIBLING_NAME} = ${PINNED_REV}
  on disk              ${DISK_DESC}

A lock MISSING an edge the checkout has fails \`cargo build --locked\` ("the
lock file ... needs to be updated but --locked was passed"); a lock carrying
an edge the checkout lacks passes --locked but is rewritten by any unlocked
cargo run. Remedy, one of:

  * adopt the sibling's revision — move the pin and refresh the lock together:
EOF
    print_remedy "${DISK_REV}"
    cat >&2 <<EOF
  * or put the sibling back on the pin:
        git -C '${SIBLING_DIR}' checkout ${PINNED_REV}
EOF
  fi
done

if [[ "${FAILED}" -gt 0 ]]; then
  echo "verify-cargo-lock-matches-trace-format: FAIL (${FAILED} problem(s); see above)" >&2
  exit 1
fi

if [[ "${SKIPPED}" -gt 0 || "${PINNED_OK}" -ne 1 ]]; then
  if [[ "${STRICT}" == "1" ]]; then
    echo "verify-cargo-lock-matches-trace-format: FAIL (CT_SIBLING_CARGO_LOCKS_STRICT=1: ${SKIPPED} check(s) skipped; see above)" >&2
    exit 1
  fi
  echo "verify-cargo-lock-matches-trace-format: SKIP (Cargo.lock was NOT fully verified: ${SKIPPED} skip(s); see above)" >&2
  exit 0
fi

echo "verify-cargo-lock-matches-trace-format: all checks passed (Cargo.lock is exactly the lock of ${SIBLING_NAME}@${PINNED_REV} and of the checkout on disk)"
