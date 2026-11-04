/// Hand-rolled JSON-RPC 2.0 envelope (no SDK): single + batch, calls vs notifications,
/// exact RFC error codes. Used by both transports (stdio line-delimited; HTTP POST /mcp).
use serde_json::{json, Value};

pub const PARSE: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
#[allow(dead_code)] // part of the RFC error-code table
pub const INTERNAL: i64 = -32603;

/// A call: (id, method, params).
#[derive(Debug, Clone)]
pub struct Call(pub Value, pub String, pub Value);

/// A parsed incoming message.
#[derive(Debug)]
pub enum Incoming {
    Call(Call),
    /// Notified without an id — never answered (JSON-RPC 2.0 rule). Params dropped.
    Notification(String, #[allow(dead_code)] Value),
}

/// Tolerant-but-honest envelope parse: `jsonrpc != "2.0"` (when present) is rejected,
/// a missing id makes it a notification.
pub fn parse_one(v: &Value) -> Result<Incoming, (i64, &'static str)> {
    let obj = v.as_object().ok_or((INVALID_REQUEST, "not an object"))?;
    if let Some(jr) = obj.get("jsonrpc") {
        if jr != "2.0" {
            return Err((INVALID_REQUEST, "jsonrpc must be exactly \"2.0\""));
        }
    }
    let method = obj
        .get("method")
        .and_then(Value::as_str)
        .ok_or((INVALID_REQUEST, "method missing"))?
        .to_string();
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    match obj.get("id") {
        Some(id) if !id.is_null() => Ok(Incoming::Call(Call(id.clone(), method, params))),
        _ => Ok(Incoming::Notification(method, params)),
    }
}

pub fn result(id: &Value, r: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":r})
}

pub fn error(id: &Value, code: i64, msg: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":msg}})
}

pub fn envelope(text: &str) -> Result<Vec<Value>, (Value, i64, String)> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| (Value::Null, PARSE, format!("parse error: {e}")))?;
    Ok(match v {
        Value::Array(items) => items,
        single @ Value::Object(_) => vec![single],
        _ => vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_vs_notification() {
        let call = parse_one(&json!({"jsonrpc":"2.0","id":1,"method":"ping","params":{}})).unwrap();
        assert!(matches!(call, Incoming::Call(Call(_, _, _))));
        let note = parse_one(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})).unwrap();
        assert!(matches!(note, Incoming::Notification(_, _)));
        assert!(parse_one(&json!({"method":3})).is_err());
    }

    #[test]
    fn batch_envelope() {
        let b = envelope(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","id":2,"method":"ping"}]"#).unwrap();
        assert_eq!(b.len(), 2);
        assert!(envelope("{nope").is_err());
    }
}
