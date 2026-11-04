// Exercises Aiken's two literal byte-sequence forms: hex byte
// literals (`#"deadbeef"`) and UTF-8 text literals (`"FOO"`).
//
// These are syntactically distinct in Aiken: `#"abcd"` decodes to
// the 2-byte sequence `[0xab, 0xcd]` while `"abcd"` decodes to the
// 4-byte UTF-8 sequence `[0x61, 0x62, 0x63, 0x64]`.  Both surface
// at the wire-format level as `ByteArray` values, but the recorder
// distinguishes them at the in-memory `Value` layer (Value::ByteArray
// for `#"..."`, Value::String for `"..."`) so the trace can render
// each in its source-shape (hex vs text).
//
// Pre-this-fixture the recorder dropped both literal forms from the
// substitution map (no Value variant for them, no parser path), so
// any let-binding to a literal would simply not surface as a
// variable in the trace.  This fixture pins the new ValueRecord::
// String / ValueRecord::Raw encoding paths.
//
// Compute path (all integer arithmetic so the test body still drives
// the existing UPLC-CEK pipeline):
//   * `payload_len()` returns the literal 4, which is the length of
//     the hex literal in `payload` but is not computed from it: nothing
//     reads `payload`, so the compiler drops it.
//   * `name_len()` returns the literal 3, on the same terms for "FOO".
//   * `compute()` sums them: 4 + 3 = 7.

fn payload_len() -> Int {
  let payload = #"deadbeef"
  let n = 4
  n
}

fn name_len() -> Int {
  let name = "FOO"
  let n = 3
  n
}

fn compute() -> Int {
  let p = payload_len()
  let q = name_len()
  let total = p + q
  total
}

test bytearray_string() {
  compute() == 7
}
