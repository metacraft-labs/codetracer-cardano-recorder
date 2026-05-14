// Exercises deep pattern matching.  The recorder's when-arm matcher
// (added alongside `recursion_test.ak`) handles these pattern shapes:
//
//   1. Integer literal patterns: `0 -> ...`, `-1 -> ...`.
//   2. Wildcard `_` — always matches.
//   3. Bare constructor names and single-payload constructors:
//      `None`, `Some(x)`, `Pending`, `Active(n)`.
//   4. Nested constructor patterns: `Wrap(InnerActive(n))` —
//      destructured recursively so the inner `n` binds to the
//      payload of the inner Variant.
//
// List rest patterns (`[head, ..tail]`) and tuple patterns
// (`(a, b, c)`) are not yet decoded and still fall through to the
// wildcard arm.  This fixture pins the well-handled shapes plus the
// nested-constructor case.
//
// Flow:
//
//   * `flat_match(Some(7))` → 7  (integer-payload binding works)
//   * `flat_match(None)`    → -1 (nullary-constructor match works)
//   * `nested_match(Wrap(InnerActive(9)))` → 9 (the inner pattern
//     binds `n=9`).  The wildcard `_ -> 100` arm is dead.
//   * `compute()` totals 7 + (-1) + 9 = 15.

type WInner {
  InnerActive(Int)
  InnerIdle
}

type Wrapped {
  Wrap(WInner)
}

fn flat_match(opt: Option<Int>) -> Int {
  when opt is {
    Some(x) -> x
    None -> -1
  }
}

fn nested_match(w: Wrapped) -> Int {
  when w is {
    Wrap(InnerActive(n)) -> n
    _ -> 100
  }
}

fn compute() -> Int {
  let some_val = flat_match(Some(7))
  let none_val = flat_match(None)
  let wrapped = Wrap(InnerActive(9))
  let nested_val = nested_match(wrapped)
  let total = some_val + none_val + nested_val
  total
}

test pattern_match() {
  compute() == 15
}
