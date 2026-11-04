// Exercises Aiken's `aiken/builtin` primitives, imported by name.
//
// The recorder gained a synthetic-call path for each well-known
// builtin name (`blake2b_256`, `verify_ed25519_signature`,
// `slice_bytearray`, `length_of_bytearray`, `append_bytearray`,
// `encode_utf8`/`decode_utf8`) alongside this fixture.  Because the
// recorder doesn't link a real cryptographic implementation, the
// returned values are placeholders chosen to match the spec-correct
// kind (`blake2b_256` returns a 32-byte ByteArray, signature verify
// returns Bool, length returns Int) so downstream arithmetic still
// drives the existing UPLC-CEK pipeline.  The recorder models Bool
// as Int, so its answer for the verify call is 1.
//
// Each builtin invocation surfaces as its own Call/Return event
// pair in the trace, with per-arg variables (`arg0`, `arg1`, ...)
// attached to the call_entry step.
//
// Compute path:
//   * `payload_len()` — `length_of_bytearray(#"deadbeef")` returns 4.
//   * `signature_check()` — `verify_ed25519_signature(...)` returns
//     True, which the `when` turns into the Int 1.  The inputs are
//     test 1 of RFC 8032 section 7.1 (an empty message), because the
//     real builtin needs a 32-byte key and a 64-byte signature and
//     fails on anything else.
//   * `compute()` sums them: 4 + 1 = 5.

use aiken/builtin.{length_of_bytearray, verify_ed25519_signature}

fn payload_len() -> Int {
  let payload = #"deadbeef"
  let n = length_of_bytearray(payload)
  n
}

fn signature_check() -> Int {
  let pubkey = #"d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
  let msg = #""
  let sig = #"e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
  let ok = verify_ed25519_signature(pubkey, msg, sig)
  when ok is {
    True -> 1
    False -> 0
  }
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
