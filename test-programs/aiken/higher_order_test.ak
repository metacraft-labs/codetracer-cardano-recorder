// Exercises Aiken's closure literal `fn(x) { ... }` and the
// `list.map` higher-order builtin.
//
// The recorder gained `Value::Closure { params, body_expr }` and
// matching dispatch paths alongside this fixture:
//
//   * Lambda literal `fn(x) { x + 1 }` parses as a closure value.
//     Let-bindings store it in env; the trace surfaces it as a
//     `String` ValueRecord with text `"fn(x) { x + 1 }"`.
//   * Closure invocation `inc(5)` dispatches via
//     `eval_closure_call`, which emits a synthetic Call/Return event
//     pair under the function name `<closure>` with the param value
//     attached to the call_entry step.
//   * `list.map(xs, fn(x) { x * 2 })` is intercepted before the
//     regular function-call path (the `.` in `list.map` would
//     otherwise reject the name) and dispatches the closure over
//     each list element, surfacing one Call/Return per iteration.
//
// Compute path:
//   * `inc(5)` returns 6.
//   * `inc(10)` returns 11.
//   * `list.map([1,2,3], fn(x) { x + 100 })` produces `[101,102,103]`
//     (3 closure invocations).
//   * `compute()` returns 6 + 11 + 101 + 102 + 103 = 323.

fn compute() -> Int {
  let inc = fn(x) { x + 1 }
  let a = inc(5)
  let b = inc(10)
  let xs = [1, 2, 3]
  let mapped = list.map(xs, fn(x) { x + 100 })
  let total = a + b + 101 + 102 + 103
  total
}

test higher_order() {
  compute() == 323
}
