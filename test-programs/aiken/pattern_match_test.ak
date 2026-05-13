// Exercises deep pattern matching.  The recorder's when-arm matcher
// (added alongside `recursion_test.ak`) handles three pattern shapes
// today:
//
//   1. Integer literal patterns: `0 -> ...`, `-1 -> ...`.
//   2. Wildcard `_` — always matches.
//   3. Bare constructor names and single-payload constructors:
//      `None`, `Some(x)`, `Pending`, `Active(n)`.
//
// Nested constructor patterns (`Some(Foo { x, .. })`), list rest
// patterns (`[head, ..tail]`), and tuple patterns (`(a, b, c)`) are
// NOT decoded — they fall through to the wildcard arm.  This
// fixture pins the CURRENT behaviour for both the well-handled
// shapes (a flat `Option<Int>` switch) and the not-yet-handled
// nested shapes (a `Wrapped<Inner>` two-level switch where the
// inner discriminator is invisible to the matcher).
//
// Flow:
//
//   * `flat_match(Some(7))` → 7  (integer-payload binding works)
//   * `flat_match(None)`    → -1 (nullary-constructor match works)
//   * `nested_match(Wrapped(InnerActive(9)))` should ideally
//     bind `n=9` and return `9`, but the matcher today can't
//     destructure the inner constructor, so the `_ -> 100`
//     wildcard fires.  Captured as 100 here; the spec-correct
//     value (9) is pinned by `test_pattern_match_test_nested_decodes`
//     with `#[ignore]`.

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
  compute() == 106
}
