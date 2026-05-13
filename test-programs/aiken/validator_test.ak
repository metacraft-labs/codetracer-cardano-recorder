// Exercises Aiken's `validator name { spend(...) { ... } }` block —
// the primary use case for on-chain code.  A validator declares one
// or more entry-point functions (`spend`, `mint`, `withdraw`,
// `publish`, `vote`, `propose`, or the fallback `else`) inside a
// `validator <name> { ... }` outer block.  The recorder's parser
// gained outer-block-aware handling alongside this fixture so each
// entry-point function inside the validator surfaces as its own
// function-table entry.
//
// This fixture's `gift_card` validator implements two checks (one
// per entry point):
//
//   * `spend(...)` returns the integer `bonus` decoded from the
//     redeemer-shaped helper.  The trace surfaces `spend` as a
//     function call with the resolved arg value.
//   * `mint(action)` runs the bonus check unconditionally and
//     returns its sum with the redeemer-derived value.
//
// The `compute()` entry point exercises both validator functions
// (the test body uses `compute() == ...` rather than running the
// validator block end-to-end because the recorder doesn't model
// blockchain context yet).

fn redeemer_bonus(amount: Int) -> Int {
  amount * 10
}

validator gift_card {
  spend(amount: Int) -> Int {
    let bonus = redeemer_bonus(amount)
    let adjusted = bonus + 1
    adjusted
  }

  mint(action: Int) -> Int {
    let base = redeemer_bonus(action)
    let total = base + 5
    total
  }
}

fn compute() -> Int {
  let spent = spend(2)
  let minted = mint(3)
  let combined = spent + minted
  combined
}

test validator_smoke() {
  compute() == 56
}
