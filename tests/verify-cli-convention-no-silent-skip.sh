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
#   * `CODETRACER_CARDANO_RECORDER_LIVE_CHAIN` is referenced in source, and the
#     acquisition suite says `NOT MEASURED` instead of skipping and uses no
#     `#[ignore]`
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

for SUB in enumerate provenance eras prestate-strategy fixtures; do
  SUB_HELP="$("${BIN}" "${SUB}" --help)"
  assert_absent "--format" "${SUB} --help" "${SUB_HELP}"
  assert_absent "CODETRACER_FORMAT" "${SUB} --help" "${SUB_HELP}"
done

# The four chain-reading subcommands expose the opt-in gate on the CLI as well
# as in the environment.  `fixtures` deliberately does not: it reads only
# committed captures and has nothing to opt in to.
for SUB in enumerate provenance eras prestate-strategy; do
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

# ---------------------------------------------------------------------------
# Source-level references for the acquisition gate
# ---------------------------------------------------------------------------

if ! grep -rqF "CODETRACER_CARDANO_RECORDER_LIVE_CHAIN" "${REPO_ROOT}/src"; then
  echo "FAIL: CODETRACER_CARDANO_RECORDER_LIVE_CHAIN must be referenced in src/ — without the" \
       "gate a reading would reach the network silently" >&2
  exit 1
fi
echo "ok: CODETRACER_CARDANO_RECORDER_LIVE_CHAIN referenced in src/"

# A test that needs the network must say NOT MEASURED rather than skipping, so
# the token has to exist in the suite.
if ! grep -rqF "NOT MEASURED" "${REPO_ROOT}/tests/test_acquisition.rs"; then
  echo "FAIL: tests/test_acquisition.rs must report NOT MEASURED when the live-chain gate is" \
       "unset; a silent skip is the thing this script exists to prevent" >&2
  exit 1
fi
echo "ok: the acquisition suite reports NOT MEASURED rather than skipping"

# And it must not do it with #[ignore], which hides the reading from the
# summary line entirely.
#
# Matched as an ATTRIBUTE (a line whose first non-space token is `#[ignore`),
# not as a substring: the first spelling of this check fired on the prose in
# that file explaining why `#[ignore]` is not used, which is a false red of
# exactly the kind that gets a gate disabled.
if grep -rqE '^[[:space:]]*#\[ignore' "${REPO_ROOT}/tests/test_acquisition.rs"; then
  echo "FAIL: tests/test_acquisition.rs uses #[ignore]; an ignored test is a silent skip with" \
       "extra steps" >&2
  exit 1
fi
echo "ok: the acquisition suite uses no #[ignore] attribute"

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
