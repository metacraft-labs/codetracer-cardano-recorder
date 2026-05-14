// Exercises Aiken's `use module/path.{names}` import directive and
// the multi-module project layout (a top-level project file plus a
// `lib/` directory of sibling helper modules).
//
// Pre-this-fixture the recorder only parsed the source file passed
// to `record(source_path, ...)` and would silently drop any
// reference to an imported function or type — `double(7)` would
// resolve as an unknown identifier and the test body would surface
// no value.  The recorder gained a `use`-directive follower
// alongside this fixture: the parser now scans for `use ...` lines,
// resolves the module path to a sibling file under `lib/`, parses
// the imported file's `pub fn` declarations, and merges them into
// the function table — each tagged with the imported file's path so
// step events from cross-module calls carry the right source path
// on the wire.
//
// The strict test pins:
//   * Both source files are registered in the trace's path table
//     (the main file plus `lib/helpers.ak`).
//   * Function `double` is registered against `lib/helpers.ak` (not
//     the main file's path).
//   * Step events fired from inside `double` carry the
//     `lib/helpers.ak` path.
//   * The cross-module record literal `Pair { a: 7, b: 14 }` lands
//     as a `Value::Record { type_name: "Pair" }` (proving the
//     imported type name is recognised by the record-literal
//     parser).
//
// Compute path:
//   * `let raw = 7`
//   * `let doubled = double(raw)`           (cross-module call → 14)
//   * `let p = Pair { a: raw, b: doubled }` (cross-module record literal)
//   * `let total = p.a + p.b`               (7 + 14 = 21)
//   * `total`
//
// Test body: `compute() == 21`.

use my_project/helpers.{double, Pair}

fn compute() -> Int {
  let raw = 7
  let doubled = double(raw)
  let p = Pair { a: raw, b: doubled }
  let total = p.a + p.b
  total
}

test module_imports() {
  compute() == 21
}
