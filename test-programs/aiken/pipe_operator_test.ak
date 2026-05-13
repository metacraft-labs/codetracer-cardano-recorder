// Exercises the `|>` pipe operator.
//
// In Aiken, `x |> f(y)` desugars to `f(x, y)`; chained pipes produce
// left-associative composition: `a |> f(b) |> g(c)` desugars to
// `g(f(a, b), c)`.  Real-world Aiken code pervasively pipes data
// through transformation functions — the gift_card example pipes
// `mint |> assets.tokens(policy_id) |> dict.to_pairs()` to flatten
// a nested map.
//
// This fixture chains three pipe stages through plain integer
// helpers so the recorder's expression evaluator (which currently
// can only handle top-level function calls and integer arithmetic
// — no closures, no list combinators) still surfaces every stage.
//
// Stage 0: raw = 5
// Stage 1: raw |> add(7)    → add(5, 7)   = 12
// Stage 2: ...   |> mul(3)  → mul(12, 3)  = 36
// Stage 3: ...   |> sub(2)  → sub(36, 2)  = 34
// Final:   compute() == 34

fn add(a: Int, b: Int) -> Int {
  a + b
}

fn mul(a: Int, b: Int) -> Int {
  a * b
}

fn sub(a: Int, b: Int) -> Int {
  a - b
}

fn compute() -> Int {
  let raw = 5
  let after_add = raw |> add(7)
  let after_mul = after_add |> mul(3)
  let result = after_mul |> sub(2)
  result
}

test pipe_operator() {
  compute() == 34
}
