# Committed Koios captures — CXP-ADA-2 acquisition

Cardano mainnet's own responses, captured once and committed as text, so the
acquisition suite measures something with **no network and no credential**.
CI has neither: Blockfrost needs an API key that no lane in this workspace
holds, and Koios needs none but is still a network call. A suite that reached
the network would measure a different thing on every machine, and one that
skipped would report success about a population it never read.

Each file is an envelope:

```json
{ "network": …, "endpoint": …, "query": …, "fetched": …,
  "source": …, "identity": { "kind": …, "note": … },
  "response": <the endpoint's response, VERBATIM> }
```

## These are the chain's bytes, and that is a reading rather than a claim

A committed capture's failure mode is that somebody edited it. Every file
therefore declares an identity that `codetracer-cardano-recorder fixtures`
**recomputes** — `src/fixture.rs`, `verify_identities`. `just lint` and
`just test` both run it, and it fails on a single flipped byte
(`tests/test_acquisition.rs`, `test_a_tampered_capture_fails_its_own_identity`).

| `identity.kind` | what is recomputed |
|---|---|
| `tx-body-blake2b-256` | `blake2b-256`(transaction **body**'s original CBOR) `== tx_hash`, **and** `blake2b-256`(whole transaction CBOR) `!= tx_hash`. The second arm is what says *which* bytes the chain hashed |
| `datum-blake2b-256` | `blake2b-256(inline_datum.bytes) == datum_hash` |
| `script-blake2b-224-tagged` | `blake2b-224(language tag ‖ script bytes) == script_hash` |
| `reference-resolution` | both hashes above over a `/utxo_info` row set, **plus** that some row is spent *and* still carries its inline datum — the fact the whole `hydrated-from-node` route rests on |
| `pre-babbage-datum-hash-only` | the row names its datum by hash and carries no inline datum (CIP-32 did not exist), and the paired `/datum_info` capture's bytes hash to that datum hash |
| `cross-fixture` | the `/blocks` and `/block_txs` captures agree: every block hash in both, and each block's own `tx_count` equal to the rows enumerated for it |
| `evaluator-arity` | each `cost_models` entry reduces to a **dense** positional `0..n-1` vector, and its length is compared against the arity the UPLC evaluator reads |
| `none` | **stated as unanchored.** A projection of epoch/protocol-version pairs is not hashed by the chain in any recomputable form; what is checked is internal (epochs ascending, `protocol_major` non-decreasing) |

The last two verdicts are reported `UNANCHORED (stated, not proven)` rather
than being quietly counted as passes. A protocol-parameter set carries no
hash; saying so is the honest option and inventing an anchor is not.

## The captures

| file | endpoint | what it is for |
|---|---|---|
| `blocks_alonzo_window` | `GET /blocks` | the bounded historic range: the first ten blocks of the Alonzo era, heights 6,236,060–6,236,069 |
| `block_txs_alonzo_window` | `POST /block_txs` | every transaction in those ten blocks — the enumeration's own rows, not a `tx_count` |
| `protocol_versions_by_epoch` | `GET /epoch_params?select=…` | every epoch's announced protocol version, from which the era boundaries are derived |
| `epoch_params_era_boundaries` | `GET /epoch_params?_epoch_no=…` | full parameters for epochs 0, 207, 208, 236, 251, 290, 298, 365, 394, 507, 537, 644, 664, keyed by epoch |
| `tx_cbor_first_plutus_execution` | `POST /tx_cbor` | the earliest Plutus script execution reachable on mainnet (block 6,236,063, epoch 290, PlutusV1) |
| `utxo_info_first_plutus_execution` | `POST /utxo_info` | its spent input: pre-Babbage, datum by hash, no inline datum |
| `script_info_first_plutus_execution` | `POST /script_info` | its 2,739-byte PlutusV1 script, for the positive arm |
| `datum_info_first_plutus_execution` | `POST /datum_info` | that datum's bytes, which hash to the output's `datum_hash` |
| `tx_cbor_v3_reference_script_spend` | `POST /tx_cbor` | a live PlutusV3 spend whose scripts are CIP-33 reference scripts |
| `utxo_info_v3_reference_script_spend` | `POST /utxo_info` | its three reference inputs **and** its spent inputs — deliverable 3's evidence |
| `script_info_v3_reference_script_spend` | `POST /script_info` | the two reference scripts it runs |
| `tx_cbor_v3_embedded_script_spend` | `POST /tx_cbor` | a live PlutusV3 spend whose script **is** carried in the transaction |
| `utxo_info_v3_embedded_script_spend` | `POST /utxo_info` | its spent input, with the inline datum that is *not* carried |
| `script_info_v3_embedded_script_spend` | `POST /script_info` | its script |

Three transaction cases, chosen for what they measure rather than for being
typical: one where everything travels with the transaction (Alonzo), one where
neither the script nor the datum does (CIP-32 + CIP-33), and one in between.
Reading only the middle one would support "post-Babbage the script is always
absent", which the third refutes.

## Regenerating

Deliberately manual. A capture refreshed as a side effect of a test run would
have a date nobody chose and no reason recorded. To take a new reading, fetch
the endpoint in the `endpoint` field with the `query` in the `query` field,
put the response under `response` verbatim, set `fetched`, and run
`codetracer-cardano-recorder fixtures` — which will refuse the file if its
declared identity no longer holds.

`CODETRACER_CARDANO_RECORDER_LIVE_CHAIN=1 cargo test --test test_acquisition`
compares every capture against the live chain
(`test_the_live_chain_agrees_with_the_committed_captures`). Without that
variable the test prints **NOT MEASURED** and names what is unmeasured:
drift, not authenticity.

## No `.ct` files, ever

These are blockchain **inputs** — immutable JSON and CBOR the chain served.
Derived recordings are a different thing and are never committed in this repo;
a test that needs one records it on the fly.
