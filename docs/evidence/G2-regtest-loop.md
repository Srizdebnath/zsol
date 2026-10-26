# G2 — regtest end-to-end loop (T-39, FR-4.3): REAL, no mocks

Ecosystem truth: **z3 regtest** (zebra 130+ blocks, zallet v0.1.0-beta.1, rpc-router),
NOT testnet. Pools labeled `ironwood` = the NU6.3/NU7-pre regtest shielding pool
(orchard semantics; z3 config contract). All values below come from live RPC
results captured while executing the loop.

## Stack (all services up and healthy)

- zebra @ 127.0.0.1:29232 (`zebra:zebra`), `getblockchaininfo`: `chain:"test"`,
  NU6.3 active at height 2, chainSupply present.
- zallet @ 127.0.0.1:50232, wallet volume initialized via `scripts/regtest-init.sh`
  (`generate-mnemonic`, `init-wallet-encryption`, identity.txt).
- rpc-router merges OpenRPC `rpc.discover` at 127.0.0.1:8181 (used to pin beta.1
  param shapes honestly).
- zsol side: `zsol-registry` @ 127.0.0.1:18088 + `zsol-mcp` HTTP @ 127.0.0.1:18099
  (`DRY_RUN=0`, `ZSOL_ATTEST_ACCOUNT=0`, `ZSOL_OP_POLL_SECS=180`).

## Funding ladder (operator-land, honest)

1. Redirected mining to the wallet's own transparent receiver: UA at
   `z_getaddressforaccount` → `z_listunifiedreceivers` →
   `tmX1kJCE8oZyhRKUVhRFPAw6pNb1QZkf6LE`; `ZEBRA_MINING__MINER_ADDRESS` updated;
   120+ blocks mined via zebra `generate`.
2. Account creation: `z_getnewaccount "zsol-attn-9"` → account_uuid
   `d372d15c-c525-4bd5-adf9-3d90e9e5d22a`, zip32 index 0 → **`ZSOL_ATTEST_ACCOUNT=0`**
   for the regtest mapping (spec's account-9 slot ≙ beta.1 wallet's first account;
   documented mapping, not a redefinition).
3. Shielding coinbase: `z_shieldcoinbase` (beta.1 shape
   `[fromaddress,toaddress,fee,limit,memo,privacy_policy]`, `AllowRevealedSenders`) →
   opid-d938f272… **success**, 6.25 ZEC → ironwood pool; the earlier 21-UTXO sweep
   was marked `failed` by zallet on a transport glitch but demonstrably landed too
   (balances show both effects) — recorded as-is, not tidied.

## The anchor (zsol's own tool, DRY_RUN=0)

`zsol_anchor {agentId:"regtest-agent-2", claimZat:1000000}` →

```
opid-12fe4ada-19fc-468f-b0db-b07ca470f640  status=success
txid      64f675f120fb073472bb82fcabeca9fdcceb1c250ed3f5afefd2b0a862c8d230
frameDigestHex 264945ff84843161772dc654c83af3d968d6bdb346ee16477dd9f89f77eac939
windowHeight    132   (memo bytes: 7a733031 01 8400000000000000 ‖ digest)
```

An earlier zsol anchor (agent regtest-agent-2 window) also landed:
opid-f1acb995-be6b-434f-9b5d-6f9e772e6763 → txid
`96d544477e76bc5f8379f6b3b329ba063d140d1e665e73d856a9f4c01bd5bcd4`, memo
`7a733031018200000000000000befb012e…1199ed`, status success, `execution_secs=41`.
z_viewtransaction shows the zs01 memo inside output[3] (value 0.01):

```
outputs[3] value=0.01
memo=7a733031018200000000000000befb012e0f25bc36f7619f18f9bea2e3a21e5102
     016727e9b17d6441141199ed0000…
```

Cost of finding the op-poll parity: one extra anchor (opid-12fe4ada is the
poll-budget-discovery attempt) — both stayed tiny (0.01 ZEC) and each is a REAL
memo-carrying self-spend; disclosed rather than hidden.

## Attestation → publish → fetch (all live)

`zsol_make_attestation` (same session as the anchor):

```
{"accountUa":"uregtest1pfaffcqgt…6jvpw6jutvcc2afyg0",
 "agentId":"regtest-agent-2","anchorHeight":132,
 "anchorTxid":"64f675f120fb073472bb82fcabeca9fdcceb1c250ed3f5afefd2b0a862c8d230",
 "claimZat":1000000,"issuedAt":1792997742,"maxAgeHours":24,
 "memoBinding":"enMwMQGEAAAAAAAAACZJRf-EhDFhdy3GVMg689lo1r2zRu4WR33Z-J936sk5",
 "protocol":"zsol/0.1"}
```

`zsol_publish` → registry POST accepted:

```
chainDigest = entryId = ce9f521d891db5e4670fc58416b4fe5aaa533a83a6c96f5bc7dde72a335dff44
GET /v0/attestations/regtest-agent-2 → seq 1, object present, chainHead = same digest
```

## Verifier bridge (honest tier)

`zsol_verify {object}` → **tier `claim-only`**, reason `uivk-unavailable`,
checks:
- structural PASS, zs01 45-byte prefix PASS, frame digest/memo-height PASS, freshness PASS
- scope row explicitly FAILED (`no uivk imported: anchor-scan + balance rows skipped`)
  — zallet beta.1 exports NO unified viewing key (`z_exportkey` is Sapling-spend
  keys only per its own OpenRPC description). The verified tier is blocked on wallet
  parity (testnet-gated T-44), and zsol refuses to substitute wallet-side
  self-attestation for the scope's scan view.

## Zallet beta.1 parity traps found live (each fixed in zsol)

1. `z_sendmany` `from` = address only (account alias "0" and `ANY_TADDR` both
   rejected; `ANY_TADDR` additionally gated on
   `features.legacy_pool_seed_fingerprint` which must name a *migrated* legacy
   account — unusable here, so zsol sends address-sourced).
2. memo MUST be hex (base64 rejected `-8 Invalid parameter, expected memo data in
   hexadecimal format`).
3. fee field MUST be null (`Zallet always calculates fees internally; the fee
   field must be null`).
4. send replies are the bare opid STRING (not `{"operationId":…}`).
5. `z_getoperationstatus`/`z_getoperationresult` take `[[opid]]` (sequence of
   sequences); empty or flat forms rejected.
6. balance shape is `pools.<pool>.valueZat` (zatoshi) — zsol tags this
   `zallet-pools=zat` and never silently maps it.
7. `z_getaddressforaccount` wants STRING receiver types (`["orchard","sapling"]`);
   ZIP-32 coordinate lists rejected; account numbers pre-account-creation error
   "multiple seeds" — creation via `z_getnewaccount <name>` required.
8. `z_shieldcoinbase` accepts a single t-addr or account UUID (NOT `*`) and needs
   `AllowRevealedSenders` when the source of funds is transparent.
9. Orchard spends take ~40 s on this 4-CPU host: FR-2.5 poll budget now
   env-tunable (`ZSOL_OP_POLL_SECS`), and a timed-out-but-submitted op is RESUMED
   by `zsol_make_attestation` (same opid; never a second send).

Bottom line: the regtest loop is REAL end-to-end (real funds, real anchor memo on
chain, real registry digest-chain entry), with every deviation discovered here
fixed in code and pinned by tests (`ac_self_spend_params_shape`).
