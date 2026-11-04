// Exercises Aiken's `validator name { spend(...) { ... } }` block —
// the primary use case for on-chain code.  A validator declares one
// or more entry-point functions (`spend`, `mint`, `withdraw`,
// `publish`, `vote`, `propose`, or the fallback `else`) inside a
// `validator <name> { ... }` outer block.  The recorder's parser
// gained outer-block-aware handling alongside this fixture so each
// entry-point function inside the validator surfaces as its own
// function-table entry.
//
// This fixture's `gift_card` validator has two handlers:
//
//   * `spend(datum, amount, utxo, self)` multiplies the redeemer by
//     10 through the shared helper, adds 1 and accepts the result 21.
//   * `mint(action, policy_id, self)` multiplies the redeemer by 10
//     through the same helper, adds 5 and accepts the result 35.
//
// A handler must return Bool and takes the arguments the compiler
// fixes for its purpose: four for `spend`, whose first is the
// optional datum, and three for the others.  The context arguments
// are typed `Data` because neither handler reads them, which keeps
// this module free of the `cardano/` stdlib types.
//
// A handler is not a free function: code outside the validator calls
// it through the validator's name, `gift_card.spend(...)`.  `compute()`
// does that for both handlers (the test body uses `compute() == ...`
// rather than running the validator block end-to-end because the
// recorder doesn't model blockchain context yet) and scores each
// accepted handler through `score`, so the result is 21 + 35 = 56.

fn redeemer_bonus(amount: Int) -> Int {
  amount * 10
}

fn score(accepted: Bool, points: Int) -> Int {
  when accepted is {
    True -> points
    False -> 0
  }
}

validator gift_card {
  spend(_datum: Option<Data>, amount: Int, _utxo: Data, _self: Data) -> Bool {
    let bonus = redeemer_bonus(amount)
    let adjusted = bonus + 1
    adjusted == 21
  }

  mint(action: Int, _policy_id: Data, _self: Data) -> Bool {
    let base = redeemer_bonus(action)
    let total = base + 5
    total == 35
  }
}

fn compute() -> Int {
  let spend_ok = gift_card.spend(None, 2, 0, 0)
  let mint_ok = gift_card.mint(3, 0, 0)
  let spent = score(spend_ok, 21)
  let minted = score(mint_ok, 35)
  let combined = spent + minted
  combined
}

test validator_smoke() {
  compute() == 56
}
