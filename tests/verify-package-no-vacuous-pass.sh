#!/usr/bin/env bash
# Regression test: `just verify-package` must never report a check it did not
# run as passed, and CI must reach every `verify-*` recipe.
#
# Before this test existed, `verify-package` skipped a missing tool with exit 0
# unless CT_VERIFY_STRICT=1 was set, and then still printed `homebrew ... OK` /
# `nix ... OK`; with no .deb/.rpm built it exited 0. No workflow ran it at all.
#
# The test runs the real Justfile against a scratch copy of `packaging/`, so it
# can break a recipe file without touching the checkout. It hides a tool by
# running `just` with a PATH that holds only the binaries the recipe itself
# needs (just, python3, bash, sh, env), linked into a scratch directory. No mocks:
# every check below drives the real recipe and the real tools.
#
# Wire-up: `just lint` runs this script (the shared recorder CI runs `lint`).
#
# Exit codes:
#   0  every assertion held
#   1  an assertion failed (printed to stderr)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "${SCRATCH}"' EXIT

fail() {
  echo "verify-package-no-vacuous-pass: FAIL: $*" >&2
  exit 1
}

for tool in just python3 bash; do
  command -v "${tool}" >/dev/null || fail "${tool} is required to run this test"
done

# A scratch tree: the real Justfile and a copy of packaging/.
TREE="${SCRATCH}/tree"
mkdir -p "${TREE}"
cp "${REPO_ROOT}/Justfile" "${TREE}/Justfile"
cp -R "${REPO_ROOT}/packaging" "${TREE}/packaging"
rm -rf "${TREE}/packaging/dist"

# A PATH with only what the recipe needs: no ruby, nix-instantiate, dpkg-deb
# or rpm. `env` is linked too, for the recipe's `#!/usr/bin/env python3` on
# hosts whose /usr/bin/env is absent, and `sh`, which `just` runs the
# line-wise `verify-deb` / `verify-rpm` recipes with.
BARE="${SCRATCH}/bare-bin"
mkdir -p "${BARE}"
for tool in just python3 bash sh env; do
  ln -s "$(command -v "${tool}")" "${BARE}/${tool}"
done

OUT="${SCRATCH}/out"

# run_just <PATH> <args...>: runs `just` in the scratch tree, output to $OUT,
# and sets RC to its exit status.
run_just() {
  local path="$1"
  shift
  set +e
  (cd "${TREE}" && PATH="${path}" just --justfile "${TREE}/Justfile" "$@") >"${OUT}" 2>&1
  RC=$?
  set -e
}

expect_fail_without_ok() {
  local what="$1"
  [ "${RC}" -ne 0 ] || { cat "${OUT}" >&2; fail "${what}: exited 0"; }
  if grep -q ' OK$' "${OUT}"; then
    cat "${OUT}" >&2
    fail "${what}: printed OK"
  fi
}

# 1. Strict by default: a missing tool fails the check and prints no OK.
unset CT_VERIFY_STRICT
run_just "${BARE}" verify-package homebrew
expect_fail_without_ok "homebrew without ruby"
grep -q 'ruby not on PATH' "${OUT}" || { cat "${OUT}" >&2; fail "homebrew without ruby: reason not named"; }

run_just "${BARE}" verify-package nix
expect_fail_without_ok "nix without nix-instantiate"

# 2. deb/rpm with nothing built fail, both from the default dist directory and
#    from an explicit empty one (publish-*.yml passes `out`).
mkdir -p "${SCRATCH}/empty-dist"
for ch in deb rpm; do
  run_just "${PATH}" verify-package "${ch}"
  expect_fail_without_ok "${ch} with no package built"
  run_just "${PATH}" "verify-${ch}" "${SCRATCH}/empty-dist"
  expect_fail_without_ok "verify-${ch} on an empty dist directory"
done

# 2b. verify-deb / verify-rpm look in the directory they are given: with a
#     package there, the check gets past "no package" and stops at the missing
#     inspection tool instead (the package itself is never read here).
mkdir -p "${SCRATCH}/one-pkg"
touch "${SCRATCH}/one-pkg/x.deb" "${SCRATCH}/one-pkg/x.rpm"
for ch in deb rpm; do
  tool=dpkg-deb
  [ "${ch}" = rpm ] && tool=rpm
  run_just "${BARE}" "verify-${ch}" "${SCRATCH}/one-pkg"
  expect_fail_without_ok "verify-${ch} on a directory holding a .${ch}, without ${tool}"
  grep -q "${tool} not on PATH" "${OUT}" ||
    { cat "${OUT}" >&2; fail "verify-${ch} did not look in the directory it was given"; }
done

# 3. CT_VERIFY_STRICT=0 skips with exit 0, says SKIPPED, and never says OK.
for ch in homebrew nix deb rpm; do
  CT_VERIFY_STRICT=0 run_just "${BARE}" verify-package "${ch}"
  [ "${RC}" -eq 0 ] || { cat "${OUT}" >&2; fail "${ch} lenient: exited ${RC}"; }
  grep -q "SKIPPED" "${OUT}" || { cat "${OUT}" >&2; fail "${ch} lenient: no SKIPPED line"; }
  if grep -q ' OK$' "${OUT}"; then
    cat "${OUT}" >&2
    fail "${ch} lenient: printed OK for a skipped check"
  fi
done

# 4. With the tools present, every source channel passes on the real files...
run_just "${PATH}" verify-packaging-sources
[ "${RC}" -eq 0 ] || { cat "${OUT}" >&2; fail "verify-packaging-sources on the real packaging exited ${RC}"; }
for ch in homebrew nix aur scoop chocolatey; do
  grep -q "^\[verify\] ${ch} .* OK$" "${OUT}" || { cat "${OUT}" >&2; fail "verify-packaging-sources did not verify ${ch}"; }
done

# 5. ...and each check really inspects its file: a syntax error fails it.
break_and_expect_failure() {
  local ch="$1" file="$2" junk="$3"
  cp "${file}" "${SCRATCH}/saved"
  printf '%s\n' "${junk}" >>"${file}"
  run_just "${PATH}" verify-package "${ch}"
  expect_fail_without_ok "${ch} with a broken $(basename "${file}")"
  cp "${SCRATCH}/saved" "${file}"
}
break_and_expect_failure homebrew "$(ls "${TREE}"/packaging/homebrew/*.rb)" 'def broken('
break_and_expect_failure nix "${TREE}/packaging/nix/default.nix" '{ broken'
break_and_expect_failure aur "${TREE}/packaging/aur/PKGBUILD" 'if then'
break_and_expect_failure scoop "$(ls "${TREE}"/packaging/scoop/*.json)" '{'

# 6. CI reaches the recipes: ci.yml runs the source channels, and each publish
#    workflow verifies its package before signing and uploading it.
WF="${REPO_ROOT}/.github/workflows"
grep -Eq '^[[:space:]]+run: dev-exec just verify-packaging-sources$' "${WF}/ci.yml" ||
  fail "ci.yml does not run \`just verify-packaging-sources\`"
for ch in deb rpm; do
  verify_line="$(grep -n "just verify-${ch} out" "${WF}/publish-${ch}.yml" | head -n1 | cut -d: -f1)"
  sign_line="$(grep -n "name: Sign .${ch}" "${WF}/publish-${ch}.yml" | head -n1 | cut -d: -f1)"
  [ -n "${verify_line}" ] || fail "publish-${ch}.yml does not run \`just verify-${ch} out\`"
  [ -n "${sign_line}" ] && [ "${verify_line}" -lt "${sign_line}" ] ||
    fail "publish-${ch}.yml verifies the package after signing it"
done

echo "verify-package-no-vacuous-pass: all checks passed"
