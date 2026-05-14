// Exercises Aiken's `aiken/builtin` primitives.
//
// The recorder gained a synthetic-call path for each well-known
// builtin name (`blake2b_256`, `verify_ed25519_signature`,
// `slice_bytearray`, `length_of_bytearray`, `append_bytearray`,
// `encode_utf8`/`decode_utf8`) alongside this fixture.  Because the
// recorder doesn't link a real cryptographic implementation, the
// returned values are placeholders chosen to match the spec-correct
// kind (`blake2b_256` returns a 32-byte ByteArray, signature verify
// returns Bool, length returns Int) so downstream arithmetic still
// drives the existing UPLC-CEK pipeline.
//
// Each builtin invocation surfaces as its own Call/Return event
// pair in the trace, with per-arg variables (`arg0`, `arg1`, ...)
// attached to the call_entry step.
//
// Compute path:
//   * `payload_len()` — `length_of_bytearray(#"deadbeef")` returns 4.
//   * `signature_check()` — `verify_ed25519_signature(...)` returns
//     1 (Bool true).
//   * `compute()` sums them: 4 + 1 = 5.

fn payload_len() -> Int {
  let payload = #"deadbeef"
  let n = length_of_bytearray(payload)
  n
}

fn signature_check() -> Int {
  let pubkey = #"abcd"
  let msg = #"1234"
  let sig = #"5678"
  let ok = verify_ed25519_signature(pubkey, msg, sig)
  ok
}

fn compute() -> Int {
  let len_val = payload_len()
  let sig_val = signature_check()
  let total = len_val + sig_val
  total
}

test builtins() {
  compute() == 5
}
