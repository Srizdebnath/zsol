//! zsol wire types — the executable image of docs/SPEC.md (§4 object, §5 anchor memo, §7 registry chain).
//! Amounts are zatoshi integers everywhere (SPEC P6). No invented cryptography (SPEC P3):
//! BLAKE2b-256 digests + b64u transport only.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64U;
use base64::Engine as _;
use blake2::digest::consts::U32;
use blake2::{Blake2b, Digest};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const PROTOCOL: &str = "zsol/0.1";
pub const ANCHOR_MEMO_TAG: [u8; 4] = *b"zs01";
pub const ANCHOR_MEMO_VER: u8 = 0x01;
/// SPEC §7 registry acceptance floor (also used by zsol-mcp pre-send sanity).
pub const DUST_FLOOR_ZAT: u64 = 10_000;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ZsolError {
    #[error("base64url decode failed")]
    Base64,
    #[error("json parse failed")]
    Json,
    #[error("digest mismatch")]
    DigestMismatch,
    #[error("memo malformed")]
    MemoMalformed,
    #[error("invalid claim: {0}")]
    Claim(ReasonCode),
}

/// SPEC §9 reason code taxonomy (machine-readable, exhaustive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasonCode {
    Ok,
    ClaimOnly,
    AnchorNotMined,
    NotSelfSpend,
    ClaimExceedsOutputs,
    MemoDigestMismatch,
    BalanceBelowClaim,
    StaleAttestation,
    UivkUnavailable,
    ProtocolUnknown,
    RegistryUnreachable,
    WalletUnreachable,
    DustClaim,
    AmountMismatch,
}

impl ReasonCode {
    /// Exhaustive constant; compile-time reviewed via the FromStr roundtrip test.
    pub const ALL: [ReasonCode; 14] = [
        Self::Ok,
        Self::ClaimOnly,
        Self::AnchorNotMined,
        Self::NotSelfSpend,
        Self::ClaimExceedsOutputs,
        Self::MemoDigestMismatch,
        Self::BalanceBelowClaim,
        Self::StaleAttestation,
        Self::UivkUnavailable,
        Self::ProtocolUnknown,
        Self::RegistryUnreachable,
        Self::WalletUnreachable,
        Self::DustClaim,
        Self::AmountMismatch,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ClaimOnly => "claim-only",
            Self::AnchorNotMined => "anchor-not-mined",
            Self::NotSelfSpend => "not-self-spend",
            Self::ClaimExceedsOutputs => "claim-exceeds-outputs",
            Self::MemoDigestMismatch => "memo-digest-mismatch",
            Self::BalanceBelowClaim => "balance-below-claim",
            Self::StaleAttestation => "stale-attestation",
            Self::UivkUnavailable => "uivk-unavailable",
            Self::ProtocolUnknown => "protocol-unknown",
            Self::RegistryUnreachable => "registry-unreachable",
            Self::WalletUnreachable => "wallet-unreachable",
            Self::DustClaim => "dust-claim",
            Self::AmountMismatch => "amount-mismatch",
        }
    }
}

impl fmt::Display for ReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReasonCode {
    type Err = ZsolError;
    /// Generic parse failures surface as ZsolError::Json (may be sharpened later by T-38).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ReasonCode::ALL
            .iter()
            .find(|r| r.as_str() == s)
            .copied()
            .ok_or(ZsolError::Json)
    }
}

/// SPEC §4 — attestation object (JSON, camelCase; amounts are zatoshi u64).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttestationObject {
    pub protocol: String,
    pub agent_id: String,
    pub claim_zat: u64,
    pub anchor_height: u64,
    pub anchor_txid: String,
    pub account_ua: String,
    /// Raw anchor-memo bytes (SPEC §5), b64u transport form.
    pub memo_binding: String,
    pub issued_at: u64,
    pub max_age_hours: u32,
}

impl AttestationObject {
    /// Canonical bytes: key-sorted compact JSON (SPEC §4 "canonical form").
    pub fn canonical(&self) -> Result<Vec<u8>, ZsolError> {
        let mut v = serde_json::to_value(self).map_err(|_| ZsolError::Json)?;
        sort_json(&mut v);
        serde_json::to_vec(&v).map_err(|_| ZsolError::Json)
    }

    pub fn b64u(&self) -> Result<String, ZsolError> {
        serde_json::to_vec(self)
            .map(|b| B64U.encode(&b))
            .map_err(|_| ZsolError::Json)
    }

    pub fn from_b64u(s: &str) -> Result<Self, ZsolError> {
        let bytes = B64U.decode(s).map_err(|_| ZsolError::Base64)?;
        serde_json::from_slice(&bytes).map_err(|_| ZsolError::Json)
    }

    /// D(object) = BLAKE2b-256(PROTOCOL ‖ 0x1f ‖ canonical(object \ {memoBinding})) — SPEC §4.
    /// The exclusion breaks the memo fixed-point circularity (the memo carries D, so D cannot
    /// depend on the memo); memoBinding is instead byte-compared against the scanned on-chain
    /// memo by the verifier (SPEC §6).
    pub fn digest(&self) -> Result<[u8; 32], ZsolError> {
        let mut v = serde_json::to_value(self).map_err(|_| ZsolError::Json)?;
        sort_json(&mut v);
        if let serde_json::Value::Object(m) = &mut v {
            m.remove("memoBinding");
        }
        let input = serde_json::to_vec(&v).map_err(|_| ZsolError::Json)?;
        Ok(digest_bytes(&input, b""))
    }

    /// Structural validation; the dust floor is enforced here and again by the registry
    /// (SPEC §7 registry acceptance). Never trusts network-visible fields blindly.
    pub fn validate(&self) -> Result<(), ZsolError> {
        if self.protocol != PROTOCOL {
            return Err(ZsolError::Claim(ReasonCode::ProtocolUnknown));
        }
        validate_claim_zat(self.claim_zat).map_err(ZsolError::Claim)?;
        let hex_ok = self.anchor_txid.len() == 64
            && self.anchor_txid.bytes().all(|c| c.is_ascii_hexdigit());
        if !hex_ok {
            return Err(ZsolError::Claim(ReasonCode::AnchorNotMined));
        }
        if self.max_age_hours == 0 {
            return Err(ZsolError::Claim(ReasonCode::StaleAttestation));
        }
        if self.agent_id.is_empty() || self.account_ua.is_empty() {
            return Err(ZsolError::Claim(ReasonCode::ClaimOnly));
        }
        Ok(())
    }
}

pub fn validate_claim_zat(claim: u64) -> Result<u64, ReasonCode> {
    if claim < DUST_FLOOR_ZAT {
        Err(ReasonCode::DustClaim)
    } else {
        Ok(claim)
    }
}

/// Generic digest over byte parts, domain-separated by PROTOCOL (T-10).
pub fn digest_bytes(part1: &[u8], part2: &[u8]) -> [u8; 32] {
    let mut h = Blake2b::<U32>::new();
    h.update(PROTOCOL.as_bytes());
    h.update([0x1f]);
    h.update(part1);
    if !part2.is_empty() {
        h.update([0x1f]);
        h.update(part2);
    }
    h.finalize().into()
}

/// Registry chain digest — SPEC §7 exact formula:
/// `chainDigestₙ = BLAKE2b-256(chainDigestₙ₋₁ ‖ canonical(objectₙ))`, genesis = zero digest.
/// NOTE: plain concatenation (raw 32-byte prev ‖ canonical bytes) — no separator byte,
/// so independent verifiers reproduce registry digests byte-for-byte (FR-3.2).
pub fn chain_digest(prev: [u8; 32], canonical_object: &[u8]) -> [u8; 32] {
    let mut h = Blake2b::<U32>::new();
    h.update(prev);
    h.update(canonical_object);
    h.finalize().into()
}

/// Anchor memo (SPEC §5): `zs01(4) ‖ ver(1) ‖ height_LE64(8) ‖ digest(32)` = 45-byte prefix.
/// The wallet may right-pad memo space (ZIP-302/623 semantics); verifiers parse the prefix only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorMemo {
    pub height: u64,
    pub digest: [u8; 32],
}

impl AnchorMemo {
    pub const PREFIX_LEN: usize = 45;

    pub fn encode(&self) -> [u8; Self::PREFIX_LEN] {
        let mut m = [0u8; Self::PREFIX_LEN];
        m[0..4].copy_from_slice(&ANCHOR_MEMO_TAG);
        m[4] = ANCHOR_MEMO_VER;
        m[5..13].copy_from_slice(&self.height.to_le_bytes());
        m[13..45].copy_from_slice(&self.digest);
        m
    }

    pub fn decode_prefix(bytes: &[u8]) -> Result<Self, ZsolError> {
        if bytes.len() < Self::PREFIX_LEN {
            return Err(ZsolError::MemoMalformed);
        }
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&bytes[0..4]);
        if tag != ANCHOR_MEMO_TAG || bytes[4] != ANCHOR_MEMO_VER {
            return Err(ZsolError::MemoMalformed);
        }
        let mut h = [0u8; 8];
        h.copy_from_slice(&bytes[5..13]);
        let mut d = [0u8; 32];
        d.copy_from_slice(&bytes[13..45]);
        Ok(Self {
            height: u64::from_le_bytes(h),
            digest: d,
        })
    }

    /// Memo ↔ object binding (SPEC §5): tag/ver assumed by decode; height + digest must match.
    pub fn verify_binding(&self, expected_height: u64, object_digest: &[u8; 32]) -> Result<(), ZsolError> {
        if self.height != expected_height || self.digest != *object_digest {
            Err(ZsolError::DigestMismatch)
        } else {
            Ok(())
        }
    }
}

pub fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sort_json(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            let mut keys: Vec<String> = m.keys().cloned().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                let mut val = m.remove(&k).unwrap();
                sort_json(&mut val);
                out.insert(k, val);
            }
            *v = serde_json::Value::Object(out);
        }
        serde_json::Value::Array(a) => {
            for e in a.iter_mut() {
                sort_json(e);
            }
        }
        _ => {}
    }
}

/// Independent reference golden (computed with python3 hashlib.blake2b digest_size=32):
/// BLAKE2b-256(b"zsol/0.1\x1fgolden\x1fvector")
pub const GOLDEN_DIGEST_HEX: &str = "c3fa8cc5f9621c95bec0a532fa298f8bd7ef42596cba589d69c5644ca6b3fce7";

#[cfg(test)]
mod tests {
    use super::*;
    use hex::FromHex;
    use proptest::prelude::*;

    /// Self-consistent fixture: builds the object WITHOUT the memo binding, digests it,
    /// then binds the real anchor memo — mirroring the attestor flow (fixed point via
    /// memoBinding exclusion, SPEC §4).
    fn sample() -> AttestationObject {
        let mut o = AttestationObject {
            protocol: PROTOCOL.into(),
            agent_id: "agent-0001".into(),
            claim_zat: 1_000_000, // 0.01 TAZ (SRS §10.3)
            anchor_height: 4_509_856,
            anchor_txid: "0".repeat(64),
            account_ua: "u1test-diversified-ua".into(),
            memo_binding: String::new(),
            issued_at: 1_769_337_600,
            max_age_hours: 24,
        };
        let digest = o.digest().unwrap();
        let memo = AnchorMemo {
            height: o.anchor_height,
            digest,
        };
        o.memo_binding = B64U.encode(&memo.encode());
        o
    }

    /// T-10 cross-implementation golden: RustCrypto must equal python3 hashlib (independent).
    #[test]
    fn ac_digest_golden_vector() {
        let d = digest_bytes(b"golden", b"vector");
        assert_eq!(hex::encode(d), GOLDEN_DIGEST_HEX);
    }

    /// T-10: the digest input EXCLUDES memoBinding (SPEC §4) — this is what makes the
    /// memo fixed point computable; objects differing only in memoBinding share D.
    #[test]
    fn ac_digest_excludes_memo_binding() {
        let o = sample();
        let mut other = o.clone();
        other.memo_binding = "AAAA".into();
        assert_eq!(o.digest().unwrap(), other.digest().unwrap());
        // binding property preserved: any OTHER field edit shifts D
        let mut edited = o.clone();
        edited.claim_zat += 1;
        assert_ne!(o.digest().unwrap(), edited.digest().unwrap());
        let mut edited = o.clone();
        edited.agent_id = "agent-evil".into();
        assert_ne!(o.digest().unwrap(), edited.digest().unwrap());
    }

    /// T-10: canonical/digest determinism + sensitivity (SRS FR-1.2).
    #[test]
    fn ac_digest_deterministic_sensitive() {
        let o = sample();
        let c = o.canonical().unwrap();
        let o2 = sample();
        assert_eq!(c, o2.canonical().unwrap());
        let d = o.digest().unwrap();
        let mut tampered = o.clone();
        tampered.claim_zat += 1;
        assert_eq!(d, o2.digest().unwrap());
        assert_ne!(d, tampered.digest().unwrap());
    }

    /// FR-1.1 serde + b64u round-trip exactness.
    #[test]
    fn ac_object_roundtrip_b64u() {
        let o = sample();
        let s = o.b64u().unwrap();
        assert_eq!(AttestationObject::from_b64u(&s).unwrap(), o);
    }

    /// T-11/T-16: memo codec — encode/decode roundtrip incl. padded memo tolerance.
    #[test]
    fn ac_memo_roundtrip_and_padding_tolerance() {
        let memo = AnchorMemo { height: 77, digest: [9u8; 32] };
        let bytes = memo.encode();
        assert_eq!(AnchorMemo::decode_prefix(&bytes).unwrap(), memo);
        // wallet right-padding (SPEC §5): trailing 100 bytes ignored by prefix parse
        let mut padded = bytes.to_vec();
        padded.extend_from_slice(&[0u8; 100]);
        assert_eq!(AnchorMemo::decode_prefix(&padded).unwrap(), memo);
    }

    /// T-11: malformed/tagged/height-tampered cases (SRS FR-1.3).
    #[test]
    fn ac_memo_malformed_and_binding() {
        assert!(AnchorMemo::decode_prefix(&[0u8; 10]).is_err());
        let mut ok = sample();
        let memo = AnchorMemo::decode_prefix(&B64U.decode(&ok.memo_binding).unwrap()).unwrap();
        memo.verify_binding(ok.anchor_height, &ok.digest().unwrap()).unwrap();
        // tamper the memo digest inside the object ⇒ binding fails (FR-1.3 ac-3 shape)
        let mut bad = [9u8; 32];
        bad[0] ^= 1;
        assert_eq!(
            memo.verify_binding(ok.anchor_height, &bad),
            Err(ZsolError::DigestMismatch)
        );
        // wrong height ⇒ DigestMismatch too
        assert_eq!(
            memo.verify_binding(ok.anchor_height + 1, &ok.digest().unwrap()),
            Err(ZsolError::DigestMismatch)
        );
    }

    /// T-12: reason-code parse round-trip over the exhaustive set.
    #[test]
    fn ac_reason_code_roundtrip_all() {
        for r in ReasonCode::ALL {
            assert_eq!(r.as_str().parse::<ReasonCode>().unwrap(), r);
            assert_eq!(r.to_string(), r.as_str());
        }
        assert!("not-a-reason".parse::<ReasonCode>().is_err());
    }

    /// T-13: chain digest continuity + prev-dependence (SRS FR-1.5).
    #[test]
    fn ac_chain_digest_indentities() {
        let zero = [0u8; 32];
        let d1 = chain_digest(zero, b"a");
        assert_eq!(d1, chain_digest(zero, b"a"));
        assert_ne!(d1, chain_digest([1u8; 32], b"a"));
        assert_ne!(d1, chain_digest(zero, b"b"));
    }

    /// T-14: dust floor enforcement.
    #[test]
    fn ac_dust_floor() {
        assert_eq!(validate_claim_zat(9_999), Err(ReasonCode::DustClaim));
        assert_eq!(validate_claim_zat(10_000), Ok(10_000));
    }

    /// Object structural validation (protocol tag, txid shape, agent id).
    #[test]
    fn ac_object_validate_paths() {
        let mut o = sample();
        o.validate().unwrap();
        o.protocol = "other/1".into();
        assert_eq!(o.validate(), Err(ZsolError::Claim(ReasonCode::ProtocolUnknown)));
        let mut o = sample();
        o.anchor_txid = "zz".into();
        assert_eq!(o.validate(), Err(ZsolError::Claim(ReasonCode::AnchorNotMined)));
        let mut o = sample();
        o.max_age_hours = 0;
        assert_eq!(o.validate(), Err(ZsolError::Claim(ReasonCode::StaleAttestation)));
    }

    /// T-11 fuzz: ≥ 1k random (height, digest) roundtrips through encode/decode.
    proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(1_000))]
    #[test]
    fn prop_memo_roundtrip(height in any::<u64>(), dg in proptest::collection::vec(any::<u8>(), 32)) {
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&dg);
        let memo = AnchorMemo { height, digest };
        let dec = AnchorMemo::decode_prefix(&memo.encode()).unwrap();
        prop_assert_eq!(dec, memo);
    }
}

    /// Keep hex import honest even if tests above change shape.
    #[test]
    fn hex_fromhex_available() {
        let g = Vec::<u8>::from_hex(GOLDEN_DIGEST_HEX).unwrap();
        assert_eq!(g.len(), 32);
    }
}
