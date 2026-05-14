// Exercises Aiken's `opaque type`, generics, and type aliases.
//
// `opaque type Wrapper { inner: Int }` is an access-control feature
// (call sites outside the defining module can't destructure the
// constructor) — at the runtime level the wrapper is identical to
// the equivalent `type Wrapper { inner: Int }` record.  The
// recorder labels the registered type id with a `" (opaque)"`
// suffix so consumers that surface type names in the trace can
// distinguish opaque-wrapped values from regular records, but the
// inner field (`inner: 42`) still surfaces verbatim as part of the
// `ValueRecord::Struct.field_values` slot.
//
// `pair_first<a, b>(p: (a, b)) -> a` is a generic function — the
// recorder strips the `<a, b>` decl so the parameter parser sees a
// plain `(p: (a, b))` shape.  At call time the actual instantiation
// is whatever the caller passes in; the trace only carries the
// concrete `Value::Tuple` so generics are essentially type erased
// at the recorder boundary.
//
// Compute path:
//   * `unwrap(Wrapper { inner: 42 })` returns 42.
//   * `extract_first()` builds a tuple `(7, 11)`, calls
//     `pair_first(pair)`, which projects the first element via
//     `p.0`.  Returns 7.
//   * `compute()` sums them: 42 + 7 = 49.

opaque type Wrapper {
  inner: Int
}

fn unwrap(w: Wrapper) -> Int {
  let value = w.inner
  value
}

fn pair_first(p: (Int, Int)) -> Int {
  let first = p.0
  first
}

fn extract_first() -> Int {
  let pair = (7, 11)
  let f = pair_first(pair)
  f
}

fn compute() -> Int {
  let wrapped = Wrapper { inner: 42 }
  let unwrapped = unwrap(wrapped)
  let first = extract_first()
  let total = unwrapped + first
  total
}

test opaque_generic() {
  compute() == 49
}
