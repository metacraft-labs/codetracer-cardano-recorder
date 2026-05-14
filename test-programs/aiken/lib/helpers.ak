// Helper module imported by `module_imports_test.ak` via the
// `use my_project/helpers.{double, Pair}` directive.  The recorder
// gained a sibling-file loader alongside the fixture: `use` lines
// are followed, the named module's source is read, and any
// `pub fn` declarations land in the function table tagged with the
// imported file's path so cross-module step events carry the
// correct source path on the wire.
//
// The file is structured to be stable across recorder iterations:
// `double` is a pure-arithmetic Int → Int function (drives the
// existing UPLC-CEK pipeline) and `Pair` is a flat 2-field record
// (drives the existing `Value::Record` shape).

pub fn double(x: Int) -> Int {
  let d = x * 2
  d
}

pub type Pair {
  a: Int,
  b: Int,
}
