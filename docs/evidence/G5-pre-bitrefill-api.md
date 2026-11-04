# G5 precheck: Bitrefill API path is live (2026-10-26)

Real calls to `https://api.bitrefill.com/v2` with the owner's API key. The key is kept in a
git-ignored `.env` with mode 600 and is never logged.

| Step | Call | Result |
|---|---|---|
| Auth | `GET /ping` | HTTP 200 `pong` |
| Balance | `GET /accounts/balance` | `0 BTC` |
| Catalog | `GET /products/search?q=test&include_test_products=true` | official test products listed (`test-gift-card-code`, `test-gift-card-link`, `test-phone-refill`, `*-fail`) |
| Buy | `POST /invoices` `{test-gift-card-code, value 10 USD, payment_method: balance, auto_pay: true}` | invoice `23e5ebfa-d900-464f-934b-4c32232065fe` |
| Deliver | `GET /invoices/{id}` | `all_delivered`; order `6adf9550685fe944d1059c17` `delivered` with redemption info (not recorded here) |
| Cost | `GET /accounts/balance` | still `0 BTC`, so test products are free, as documented |

This confirms SRS FR-6.3's purchase path, using the real API with no mock. It is not gate G5
itself: G5 also requires the zsol verification and the shielded repayment.
