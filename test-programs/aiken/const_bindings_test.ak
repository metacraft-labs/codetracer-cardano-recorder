// Exercises Aiken's top-level `const` declarations.
//
// Pre-this-fixture the recorder's parser ignored `const ... = ...`
// lines entirely, so any reference to a const from a function body
// resolved as an unknown identifier and the let-binding RHS that
// referenced it surfaced no value in the trace.  This fixture pins
// the new "parse top-level consts and seed the eval environment"
// recorder behaviour:
//
//   * `const max_supply: Int = 1_000_000`        — integer literal
//     with Aiken's underscore-as-thousands-separator notation.
//   * `const half_supply: Int = max_supply / 2`  — const-to-const
//     reference (forward-evaluated when the second const is parsed).
//   * `const allowed_token: ByteArray = #"deadbeef"` — non-Int const
//     surfaces as the same `ValueRecord::String { text: "#\"...\"" }`
//     shape that bytearray_string_test pins.
//
// Compute path: `compute()` reads the consts via let-bindings so the
// trace surfaces each lookup as a regular variable event:
//   * `let supply = max_supply`          → Int 1000000
//   * `let half = half_supply`           → Int 500000
//   * `let token = allowed_token`        → String "#\"deadbeef\""
//   * `let total = supply + half`        → Int 1500000
//   * `total - 500000`                   → Int 1000000
//
// Test body: `compute() == 1000000`.

const max_supply: Int = 1_000_000

const allowed_token: ByteArray = #"deadbeef"

const half_supply: Int = max_supply / 2

fn compute() -> Int {
  let supply = max_supply
  let half = half_supply
  let token = allowed_token
  let total = supply + half
  let final = total - 500000
  final
}

test const_bindings() {
  compute() == 1000000
}
