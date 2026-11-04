//! Bitrefill API v2 client (SRS FR-6.3). Bearer-key auth; the key never appears in logs.
//! In `test` mode only Bitrefill's official free test products may be bought, paid from
//! account balance (verified live 2026-10-26: invoice delivered, balance unchanged).

use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};

const BASE: &str = "https://api.bitrefill.com/v2";

pub const TEST_PRODUCTS: [&str; 3] = [
    "test-gift-card-code",
    "test-gift-card-link",
    "test-phone-refill",
];

#[derive(Clone)]
pub struct Bitrefill {
    http: reqwest::Client,
    key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Purchase {
    pub invoice_id: String,
    pub order_id: Option<String>,
    pub status: String,
    /// Redemption payload as returned by Bitrefill (code / link / instructions).
    pub redemption: Value,
}

impl std::fmt::Debug for Bitrefill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Bitrefill { key: <redacted> }")
    }
}

impl Bitrefill {
    pub fn new(key: String) -> anyhow::Result<Self> {
        if key.trim().is_empty() {
            bail!("BITREFILL_API_KEY is empty");
        }
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()?,
            key: key.trim().to_string(),
        })
    }

    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        let r = self
            .http
            .get(format!("{BASE}{path}"))
            .bearer_auth(&self.key)
            .send()
            .await
            .with_context(|| format!("GET {path}"))?;
        let status = r.status();
        let body: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            bail!(
                "Bitrefill GET {path} → HTTP {status}: {}",
                error_text(&body)
            );
        }
        Ok(body)
    }

    pub async fn ping(&self) -> anyhow::Result<()> {
        self.get("/ping").await.map(|_| ())
    }

    pub async fn balance(&self) -> anyhow::Result<Value> {
        Ok(self.get("/accounts/balance").await?["data"].clone())
    }

    pub async fn product(&self, id: &str) -> anyhow::Result<Value> {
        Ok(self.get(&format!("/products/{id}")).await?["data"].clone())
    }

    /// Creates an invoice paid from balance and waits for delivery (≤ 60 s).
    pub async fn buy(
        &self,
        product_id: &str,
        value: u64,
        test_mode: bool,
    ) -> anyhow::Result<Purchase> {
        if test_mode && !TEST_PRODUCTS.contains(&product_id) {
            bail!(
                "test mode only buys Bitrefill test products ({TEST_PRODUCTS:?}), not {product_id:?}"
            );
        }
        let r = self
            .http
            .post(format!("{BASE}/invoices"))
            .bearer_auth(&self.key)
            .json(&json!({
                "products": [{"product_id": product_id, "value": value, "quantity": 1}],
                "payment_method": "balance",
                "auto_pay": true,
            }))
            .send()
            .await
            .context("POST /invoices")?;
        let status = r.status();
        let body: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("Bitrefill invoice → HTTP {status}: {}", error_text(&body));
        }
        let invoice_id = body["data"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("Bitrefill invoice response has no id"))?
            .to_string();
        for _ in 0..30 {
            let inv = self.get(&format!("/invoices/{invoice_id}")).await?["data"].clone();
            let st = inv["status"].as_str().unwrap_or("").to_string();
            match st.as_str() {
                "all_delivered" | "complete" => {
                    let order = inv["orders"].get(0).cloned().unwrap_or(Value::Null);
                    return Ok(Purchase {
                        invoice_id,
                        order_id: order["id"].as_str().map(str::to_string),
                        status: st,
                        redemption: order["redemption_info"].clone(),
                    });
                }
                s if s.contains("fail") || s.contains("cancel") || s.contains("expired") => {
                    bail!("Bitrefill invoice {invoice_id} ended in status {s:?}");
                }
                _ => tokio::time::sleep(Duration::from_secs(2)).await,
            }
        }
        bail!("Bitrefill invoice {invoice_id} not delivered within 60s")
    }
}

/// Whether `value` is a denomination the product sells: inside its `range` on a `step`, or equal
/// to one of its numeric `packages`.
pub fn offers_value(product: &Value, value: u64) -> bool {
    let r = &product["range"];
    if let (Some(min), Some(max)) = (r["min"].as_u64(), r["max"].as_u64()) {
        let step = r["step"].as_u64().filter(|s| *s > 0).unwrap_or(1);
        if (min..=max).contains(&value) && (value - min).is_multiple_of(step) {
            return true;
        }
    }
    product["packages"].as_array().is_some_and(|ps| {
        ps.iter()
            .any(|p| p["value"].as_str().and_then(|v| v.parse::<u64>().ok()) == Some(value))
    })
}

fn error_text(body: &Value) -> String {
    body.get("message")
        .or_else(|| body.get("error"))
        .map(|v| v.to_string())
        .unwrap_or_else(|| "no error message".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shapes as returned by GET /v2/products/{id} on 2026-10-27.
    #[test]
    fn denominations() {
        let code = json!({"range": {"min": 10, "max": 100, "step": 10}, "packages": [{"value": "10"}, {"value": "20"}]});
        assert!(offers_value(&code, 10) && offers_value(&code, 100));
        assert!(!offers_value(&code, 5) && !offers_value(&code, 15) && !offers_value(&code, 110));
        let link = json!({"range": null, "packages": [{"value": "Dinner for One"}]});
        assert!(!offers_value(&link, 20));
    }
}
