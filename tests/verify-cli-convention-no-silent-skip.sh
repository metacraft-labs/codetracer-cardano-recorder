#!/usr/bin/env bash
# Verify that the codetracer-cardano-recorder CLI complies with
# `Recorder-CLI-Conventions.md` (no silent skip — every assertion
# either passes or fails loudly):
#
#   * `--format` is absent from `--help` (CTFS-only — convention §4)
#   * `CODETRACER_FORMAT` is absent from `--help` (convention §5)
#   * `--out-dir` and `--version` are present in `--help` (§3)
#   * `--help` mentions `ct print` (the canonical conversion tool, §4)
#   * `CODETRACER_CARDANO_RECORDER_OUT_DIR` is referenced in source so the
#     env-var fallback (§5) cannot regress silently.
#
# And, since CXP-ADA-2, the acquisition subcommands:
#
#   * `enumerate`, `provenance`, `eras`, `prestate-strategy` and `fixtures`
#     each keep the CTFS-only convention
#   * the four chain-reading ones expose `--live`, so reaching the network is
#     always an explicit act; `fixtures` must NOT, because it reads only
#     committed captures
#   * `fixtures` reports `0 broken`, i.e. every committed capture still holds
#     the hash identity the chain published for it
#   * `enumerate` defaults to the committed captures and says `koios-fixture`,
#     and prints a `MEASURED:` count rather than a prose claim
#   * `prestate-strategy` prints the §1.4 token `hydrated-from-node` and the
#     `REFUTED` verdict on the retired `self-contained` claim, and does NOT
#     print `NOT MEASURED` — the reading runs offline
#   * `eras` prints BOTH floors, because the acquisition floor and the
#     script-tracing floor are different numbers and printing one of them is
#     the collapse this milestone measured apart
#   * `CODETRACER_CARDANO_RECORDER_LIVE_CHAIN` is referenced in source, and EVERY
#     test file carrying a live arm says `NOT MEASURED` instead of skipping
#   * no `tests/*.rs` and no `src/*.rs` carries an ignore attribute at all
#
# And, since CXP-ADA-1 deliverable 4, the budget oracle:
#
#   * `budget-oracle` keeps the CTFS-only convention and exposes `--live`
#   * it runs offline over the committed corpus, prints the DISTRIBUTION it
#     measured (EQUAL/UNDER/OVER) rather than a bare verdict, says
#     `NOT MEASURED: none`, and names the epoch and era each transaction was
#     charged under — a replay that did not say which cost model it used is the
#     era-blind defect this landing closed
#
# Wire-up: see `Justfile` (`just lint` and `just test` both run this
# script).
#
# Exit codes:
#   0  all assertions held
#   1  at least one assertion failed (the failing line is printed to
#      stderr and the script exits at the first failure for clarity)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Build the binary if it isn't already built (cargo build is a no-op
# when nothing has changed).  We use --quiet so the output of this
# script stays focused on verification results.
( cd "${REPO_ROOT}" && cargo build --locked --quiet )

BIN="${REPO_ROOT}/target/debug/codetracer-cardano-recorder"
if [[ ! -x "${BIN}" ]]; then
  echo "ERROR: recorder binary not found at ${BIN}" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

assert_absent() {
  # assert_absent <needle> <haystack-description> <haystack>
  local needle="$1"
  local desc="$2"
  local haystack="$3"
  if grep -qF -- "${needle}" <<< "${haystack}"; then
    echo "FAIL: ${desc} must NOT contain '${needle}'" >&2
    echo "----- ${desc} -----" >&2
    echo "${haystack}" >&2
    echo "-------------------" >&2
    exit 1
  fi
  echo "ok: '${needle}' absent from ${desc}"
}

assert_present() {
  # assert_present <needle> <haystack-description> <haystack>
  local needle="$1"
  local desc="$2"
  local haystack="$3"
  if ! grep -qF -- "${needle}" <<< "${haystack}"; then
    echo "FAIL: ${desc} must contain '${needle}'" >&2
    echo "----- ${desc} -----" >&2
    echo "${haystack}" >&2
    echo "-------------------" >&2
    exit 1
  fi
  echo "ok: '${needle}' present in ${desc}"
}

# ---------------------------------------------------------------------------
# Top-level --help
# ---------------------------------------------------------------------------

TOP_HELP="$("${BIN}" --help)"

assert_absent "--format" "top-level --help" "${TOP_HELP}"
assert_absent "CODETRACER_FORMAT" "top-level --help" "${TOP_HELP}"
assert_present "--help" "top-level --help" "${TOP_HELP}"
assert_present "--version" "top-level --help" "${TOP_HELP}"
assert_present "ct print" "top-level --help" "${TOP_HELP}"

# ---------------------------------------------------------------------------
# `record` subcommand --help
# ---------------------------------------------------------------------------

RECORD_HELP="$("${BIN}" record --help)"

assert_absent "--format" "record --help" "${RECORD_HELP}"
assert_absent "CODETRACER_FORMAT" "record --help" "${RECORD_HELP}"
assert_present "--out-dir" "record --help" "${RECORD_HELP}"

# ---------------------------------------------------------------------------
# `replay` subcommand --help
# ---------------------------------------------------------------------------

REPLAY_HELP="$("${BIN}" replay --help)"

assert_absent "--format" "replay --help" "${REPLAY_HELP}"
assert_absent "CODETRACER_FORMAT" "replay --help" "${REPLAY_HELP}"
assert_present "--out-dir" "replay --help" "${REPLAY_HELP}"

# ---------------------------------------------------------------------------
# --version output
# ---------------------------------------------------------------------------

VERSION_OUT="$("${BIN}" --version)"
assert_present "codetracer-cardano-recorder" "--version output" "${VERSION_OUT}"

# ---------------------------------------------------------------------------
# The CXP-ADA-2 acquisition subcommands
# ---------------------------------------------------------------------------
#
# Each must keep the CTFS-only convention (§4) and each must carry the
# live-chain gate, so a reading can never silently go to the network and can
# never silently skip when it does not.

for SUB in enumerate provenance eras prestate-strategy fixtures budget-oracle; do
  SUB_HELP="$("${BIN}" "${SUB}" --help)"
  assert_absent "--format" "${SUB} --help" "${SUB_HELP}"
  assert_absent "CODETRACER_FORMAT" "${SUB} --help" "${SUB_HELP}"
done

# The chain-reading subcommands expose the opt-in gate on the CLI as well as in
# the environment.  `fixtures` deliberately does not: it reads only committed
# captures and has nothing to opt in to.
for SUB in enumerate provenance eras prestate-strategy budget-oracle; do
  SUB_HELP="$("${BIN}" "${SUB}" --help)"
  assert_present "--live" "${SUB} --help" "${SUB_HELP}"
done

FIXTURES_HELP="$("${BIN}" fixtures --help)"
assert_absent "--live" "fixtures --help" "${FIXTURES_HELP}"

# ---------------------------------------------------------------------------
# The acquisition readings run offline, and they report rather than skip
# ---------------------------------------------------------------------------

# Every committed capture must still hold the hash identity it declares.  This
# is the one check that says the fixtures are the chain's bytes, so the lint
# runs it rather than leaving it to the test binary alone.
FIXTURES_OUT="$("${BIN}" fixtures 2>&1)"
assert_present "0 broken" "fixtures output" "${FIXTURES_OUT}"
assert_present "ANCHORED" "fixtures output" "${FIXTURES_OUT}"

# The default must be offline, and must say so.
ENUM_OUT="$("${BIN}" enumerate 2>&1)"
assert_present "koios-fixture" "enumerate output" "${ENUM_OUT}"
assert_present "MEASURED:" "enumerate output" "${ENUM_OUT}"

# The declared prestate strategy must be the §1.4 token, from the binary.
PRESTATE_OUT="$("${BIN}" prestate-strategy 2>&1)"
assert_present "hydrated-from-node" "prestate-strategy output" "${PRESTATE_OUT}"
assert_present "REFUTED" "prestate-strategy output" "${PRESTATE_OUT}"
assert_absent "NOT MEASURED" "prestate-strategy output" "${PRESTATE_OUT}"

# Both floors must be reported, separately.  A run that printed one of them
# would be the silent collapse this milestone measured apart.
ERAS_OUT="$("${BIN}" eras 2>&1)"
assert_present "acquisition floor" "eras output" "${ERAS_OUT}"
assert_present "script-tracing floor" "eras output" "${ERAS_OUT}"

# The CXP-ADA-1 deliverable-4 oracle must run offline over the committed corpus
# and must print the distribution it measured, not a verdict.  A command that
# printed "AGREES" without the counts would hide an empty population, and
# `NOT MEASURED: none` is the line that says no transaction was quietly
# dropped from the census.
BUDGET_OUT="$("${BIN}" budget-oracle 2>&1)"
assert_present "koios-fixture" "budget-oracle output" "${BUDGET_OUT}"
assert_present "EQUAL" "budget-oracle output" "${BUDGET_OUT}"
assert_present "UNDER" "budget-oracle output" "${BUDGET_OUT}"
assert_present "OVER" "budget-oracle output" "${BUDGET_OUT}"
assert_present "NOT MEASURED: none" "budget-oracle output" "${BUDGET_OUT}"
# And it must name the era it charged each transaction under.  A replay that
# did not say which epoch's cost model it used is the era-blind defect this
# landing closed, wearing a passing badge.
assert_present "epoch 290 (era Alonzo)" "budget-oracle output" "${BUDGET_OUT}"
assert_present "epoch 664 (era Conway)" "budget-oracle output" "${BUDGET_OUT}"

# ---------------------------------------------------------------------------
# Source-level references for the acquisition gate
# ---------------------------------------------------------------------------

if ! grep -rqF "CODETRACER_CARDANO_RECORDER_LIVE_CHAIN" "${REPO_ROOT}/src"; then
  echo "FAIL: CODETRACER_CARDANO_RECORDER_LIVE_CHAIN must be referenced in src/ — without the" \
       "gate a reading would reach the network silently" >&2
  exit 1
fi
echo "ok: CODETRACER_CARDANO_RECORDER_LIVE_CHAIN referenced in src/"

# EVERY test file that consults the live-chain gate must report NOT MEASURED
# when it is unset, rather than skipping.
#
# Scoped per file and derived from the files themselves, not from a hard-coded
# path. The previous spelling named `tests/test_acquisition.rs` alone, which
# left three of four test files — and every test file added afterwards —
# outside the ban while the gate stayed green
# That gap was filed as
# `issues/2026-10-31-the-cardano-no-silent-skip-gate-bans-ignore-in-one-test-file-of-four.md`
# and is resolved here, so its file is deleted and the reading is in CXP-ADA-1's
# "Measured state — 2026-10-31". A ban that reads as repository-wide and covers one file is
# the shape Chain-Support-Matrix.md §2.4 warns about: the green is narrower
# than the sentence next to it.
GATED_FILES=0
for F in "${REPO_ROOT}"/tests/*.rs; do
  if grep -qF "CODETRACER_CARDANO_RECORDER_LIVE_CHAIN" "${F}" \
     || grep -qF "live_chain_enabled" "${F}"; then
    GATED_FILES=$((GATED_FILES + 1))
    # Per GUARD, not per file. An existence check over the whole file passes
    # while one of several reports is deleted, and it passes on prose in the
    # module header that merely mentions the token — both of which the filed
    # issue names. So each `live_chain_enabled()` guard must have a NOT
    # MEASURED report inside the twelve lines that follow it, counted over
    # non-comment lines only.
    MISSING="$(awk '
      /live_chain_enabled\(\)/ { guard = NR; found = 0 }
      guard && NR > guard && NR <= guard + 12 {
        line = $0
        sub(/^[[:space:]]+/, "", line)
        if (line !~ /^(\/\/|\*)/ && index($0, "NOT MEASURED")) { found = 1 }
      }
      guard && NR == guard + 12 { if (!found) print guard; guard = 0 }
      END { if (guard && !found) print guard }
    ' "${F}")"
    if [[ -n "${MISSING}" ]]; then
      echo "FAIL: $(basename "${F}") consults the live-chain gate at line(s) ${MISSING}" \
           "without reporting NOT MEASURED in the guarded branch; a test that needs the" \
           "network and says nothing when it is absent is a silent skip" >&2
      exit 1
    fi
    GUARDS="$(grep -c 'live_chain_enabled()' "${F}")"
    echo "ok: $(basename "${F}") reports NOT MEASURED at each of its ${GUARDS} live-chain guard(s)"
  fi
done
if [[ "${GATED_FILES}" -lt 2 ]]; then
  echo "FAIL: only ${GATED_FILES} test file(s) consult the live-chain gate. Both the" \
       "acquisition suite and the budget oracle have a live arm, so a smaller number means a" \
       "live arm was removed or renamed and this check stopped covering it" >&2
  exit 1
fi
echo "ok: ${GATED_FILES} test file(s) carry a live arm, and each reports NOT MEASURED"

# And no test may skip with #[ignore], which hides the reading from the summary
# line entirely.
#
# Every `tests/*.rs` and every `src/*.rs`, not one path, and scanned per
# ATTRIBUTE rather than per line: an attribute is opened by a line whose first
# non-comment token is `#[` and runs until its brackets balance, so `ignore`
# counts wherever it sits inside it. A line-anchored grep was the first
# spelling and it had a measured hole — driven red 2026-10-31 — because
#
#     #[cfg_attr(
#         all(),
#         ignore
#     )]
#
# puts `ignore` on a continuation line that starts with neither `#[` nor a
# comment, and `cargo test` really does report that test `ignored`. `//`
# comment text is stripped before the scan rather than the match being
# narrowed to the first token, which is what keeps the original false red away
# — the prose in `test_acquisition.rs` explaining why `#[ignore]` is not used
# — without excusing any attribute spelling.
for F in "${REPO_ROOT}"/tests/*.rs "${REPO_ROOT}"/src/*.rs; do
  HITS="$(awk '
    {
      line = $0
      sub(/\/\/.*/, "", line)
    }
    !inattr && line ~ /^[[:space:]]*#\[/ { inattr = 1; start = NR; buf = ""; depth = 0 }
    inattr {
      buf = buf " " line
      depth += gsub(/\[/, "[", line) - gsub(/\]/, "]", line)
      if (depth <= 0) {
        if (buf ~ /(^|[^A-Za-z0-9_])ignore([^A-Za-z0-9_]|$)/) {
          sub(/^[[:space:]]+/, "", buf)
          print start ": " buf
        }
        inattr = 0
      }
    }
  ' "${F}")"
  if [[ -n "${HITS}" ]]; then
    echo "FAIL: ${F} uses an ignore attribute; an ignored test is a silent skip with extra" \
         "steps:" >&2
    echo "${HITS}" >&2
    exit 1
  fi
done
echo "ok: no ignore attribute in any tests/*.rs or src/*.rs"

# ---------------------------------------------------------------------------
# Source-level reference for the env-var fallback
# ---------------------------------------------------------------------------

# The recorder must reference CODETRACER_CARDANO_RECORDER_OUT_DIR in
# source (otherwise the env-var fallback either doesn't exist or has
# been silently removed).  We grep recursively under src/.
if ! grep -rqF "CODETRACER_CARDANO_RECORDER_OUT_DIR" "${REPO_ROOT}/src"; then
  echo "FAIL: CODETRACER_CARDANO_RECORDER_OUT_DIR must be referenced in src/" >&2
  exit 1
fi
echo "ok: CODETRACER_CARDANO_RECORDER_OUT_DIR referenced in src/"

if ! grep -rqF "CODETRACER_CARDANO_RECORDER_DISABLED" "${REPO_ROOT}/src"; then
  echo "FAIL: CODETRACER_CARDANO_RECORDER_DISABLED must be referenced in src/" >&2
  exit 1
fi
echo "ok: CODETRACER_CARDANO_RECORDER_DISABLED referenced in src/"

echo "verify-cli-convention-no-silent-skip: all checks passed"
