# zsol 0.1 audit findings (2026-10-26)

This review was done before the 0.2 redesign. Each finding gives its evidence and where 0.2
resolves it. We publish it on purpose: the zsol pitch is "trust without disclosure", which only
holds if we are honest about what failed.

## Protocol

### F-1 · Critical · The live-balance check could never fail
0.1 shared a **UIVK** (unified incoming viewing key). An incoming key detects notes *arriving* but
cannot derive nullifiers, so it never sees them *spent*. An attestor could anchor 1 ZEC, move
everything out, and the verifier's "balance ≥ claim" check would keep passing forever.
**0.2:** share the UFVK of a dedicated account. C6 checks the nullifiers of the anchored notes on
every block (SPEC §6, §8).

### F-2 · Critical · The verifier import path cannot work on any Zallet
`zsol_import_scope` called `z_importkey` with a UIVK. In Zallet (all releases through beta.3 and
`main`), `z_importkey` accepts only **Sapling extended spending keys**. `z_importviewingkey`,
added in beta.2, accepts only **Sapling extended full viewing keys**. No Zallet RPC imports a
UIVK or an Orchard/Ironwood UFVK.
Evidence: `zallet-core/src/components/json_rpc/methods/import_key.rs` and
`import_viewing_key.rs` (`PARAM_VKEY_DESC`: "only Sapling extended full viewing keys are
supported"). The regtest evidence never reached the `verified` tier.
**0.2:** the verifier is a view-only light wallet built on `zcash_client_backend` and
`zcash_client_sqlite`. It imports a UFVK directly and does not depend on Zallet.

### F-3 · High · False security claim: "writing a memo requires spend authority"
SPEC 0.1 §4 and §5 (P3) said this. It is false: anyone who knows an address can send it a note
with any memo. A party holding the UFVK could therefore manufacture a binding.
**0.2:** check C3. The anchor transaction must consume the account's own notes, which consensus
spend-authorization signatures make unforgeable.

### F-4 · High · Relay and impersonation
0.1 had no challenge. A verifier V₁ that received `{object, key}` could present both to V₂ and be
accepted as the attestor.
**0.2:** an Ed25519 `sessionKey` is committed inside the frame digest, and presentation is a
signature over V's nonce (SPEC §7).

### F-5 · Medium · Memo encoding was not ZIP-302 compliant
0.1 memos began with `'z'` (0x7A), which ZIP-302 interprets as UTF-8 text, followed by binary
bytes. **0.2:** the memo starts with the `0xFF` "arbitrary data" prefix.

## Infrastructure

### F-6 · Critical · Testnet was never live
The testnet Zebra was stuck at height 144,400. The `z3-testnet` Docker network's embedded DNS had
no upstream ("NO EXTERNAL NAMESERVERS DEFINED"), so the seeder lookups failed. All 0.1 evidence
is from **regtest**.
**Fix:** `z3/docker-compose.testnet.override.yml` sets `dns:` for zebra and zallet.

### F-7 · Critical · NU7 is active on testnet; the pinned stack predates it
NU7 activated at testnet height **4,465,026** (branch `0x77190ad9`, 25-second blocks). z3 pins
Zebra **6.2.3**, which cannot follow it. **Fix:** Zebra **7.0.0** (released 2026-10-22); a
resync from genesis started on 2026-10-26.

### F-8 · Critical · No released Zallet can transact on testnet
Zallet beta.3 pins librustzcash rev `1f6bb20`, where the NU7 testnet activation height is
`None`. It would therefore sign with the wrong consensus branch ID. The fix is
zcash/wallet PR #898 (open, Linux CI green, changes requested).
**0.2:** a light-client backend comes first, built on librustzcash `main` pre-releases (the same
pins as `zcash/zcash-devtool`) against `testnet.zec.rocks:443`. That endpoint was verified live
at height 4,544,735 on branch `0x77190ad9`. Zallet built from PR #898 becomes a second backend
once the local Zebra 7 node is synced.

## Claims
- Removed: the "BaFin proof-of-ownership above €1,000 (Aug 2026)" and "FinCEN withdrawal
  (Oct 5 2026)" references. We could not verify either from a primary source.
- Kept, with a source: the Orchard counterfeiting bug found on 2026-05-29 by Taylor Hornby
  (Shielded Labs review) using an AI auditing framework, which led to the Ironwood pool.
