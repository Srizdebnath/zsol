# Contributing to zsol

Baseline follows Zcash development standards (librustzcash style guides as reference).

## Ground rules
- Read `docs/SRS.md` §11 (invariants) and `docs/SPEC.md` before touching protocol code.
- PRs state which SRS requirement IDs they touch and which acceptance gate they satisfy.
- Rust: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` must pass.
- Web: `pnpm -C web build` must pass. No mock data anywhere (SRS invariant 3).
- Commits: imperative, one concern (`feat: zsol_anchor op polling`).

## Upstream-first
No forks, no inline dependency patches. Zebra/Zallet/Zaino issues go upstream; pin versions here.

## Privacy discipline (repo-wide)
Never log addresses, txids, viewing keys, or amounts at `info`. `ZSOL_DEBUG=1` opts into
address-bearing debug logs; CI forbids committing any log evidence showing keys. Registry
code must never persist uivk material (it's verifier-held only).
