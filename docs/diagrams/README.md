# zsol diagrams

Mermaid sources for the README and the pitch. Each file uses only `flowchart` or
`sequenceDiagram`, so Excalidraw converts it into editable shapes. Every number and txid comes
from [`../evidence/G3-negative-cases.md`](../evidence/G3-negative-cases.md) and
[`../evidence/G4-G5-credit-loop.md`](../evidence/G4-G5-credit-loop.md).

| # | Source | Caption | Save screenshot as |
|---|---|---|---|
| 01 | [`01-problem.mmd`](01-problem.mmd) | Agents cannot prove they can pay without leaking or forging, and Bitrefill does not take ZEC. | `img/01-problem.png` |
| 02 | [`02-solution-overview.mmd`](02-solution-overview.mmd) | Shielded anchor, UFVK to one verifier, C1–C8, live watch, Credit Desk buys on Bitrefill, repaid in shielded TAZ. | `img/02-solution-overview.png` |
| 03 | [`03-architecture.mmd`](03-architecture.mmd) | Components and trust boundaries: what crosses each line, and what never leaves the borrower. | `img/03-architecture.png` |
| 04 | [`04-attestation-anchor.mmd`](04-attestation-anchor.mmd) | Building an anchor: frame, digest D, self-spend with memo `0xFF‖zs02‖D`, attestation object. | `img/04-attestation-anchor.png` |
| 05 | [`05-verification-c1-c8.mmd`](05-verification-c1-c8.mmd) | Challenge, presentation, view-only import, the eight checks with their failure codes, and the tier. | `img/05-verification-c1-c8.png` |
| 06 | [`06-attacks-defeated.mmd`](06-attacks-defeated.mmd) | The four live adversarial cases from G3 and the one check that stops each. | `img/06-attacks-defeated.png` |
| 07 | [`07-credit-loop.mmd`](07-credit-loop.mmd) | The live G4/G5 run on testnet, with real heights and shortened txids. | `img/07-credit-loop.png` |
| 08 | [`08-loan-states.mmd`](08-loan-states.mmd) | Credit Desk loan states and when an agent's line is frozen. | `img/08-loan-states.png` |

## Importing into Excalidraw

1. Open <https://excalidraw.com> and choose **More tools → Mermaid to Excalidraw**.
2. Paste the full contents of one `.mmd` file and click **Insert**.
3. Restyle, then export a PNG to `docs/diagrams/img/` under the filename in the table above.
