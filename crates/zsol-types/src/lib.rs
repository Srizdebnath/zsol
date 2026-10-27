//! zsol wire types — the executable image of docs/SPEC.md 0.2 (§4 object + frame digest,
//! §5 anchor memo, §7 presentation, §9 registry chain, §10 reason codes, §11 loan memo).
//! Amounts are zatoshi integers everywhere (SPEC P6). Primitives are standard only (SPEC P3):
//! BLAKE2b-256 (RFC 7693, personalised), Ed25519 (RFC 8032), ZIP-302 memos, b64u transport.

// SRS FR-1.5: reason codes and tiers are matched exhaustively, never with `_`.
#![deny(clippy::wildcard_enum_match_arm)]

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64U;
use base64::Engine as _;
use blake2::digest::consts::U32;
use blake2::{Blake2b, Digest};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const PROTOCOL: &str = "zsol/0.2";
/// SPEC §5 binding memo: `0xFF ‖ "zs02" ‖ D`. 0xFF = ZIP-302 "arbitrary data" marker.
pub const MEMO_ARBITRARY: u8 = 0xFF;
pub const ANCHOR_MEMO_TAG: [u8; 4] = *b"zs02";
/// SPEC §11 repayment memo: `0xFF ‖ "zsc1" ‖ loanId(16)`.
pub const LOAN_MEMO_TAG: [u8; 4] = *b"zsc1";
/// SPEC §4 frame-digest personalisation (16 bytes, NUL-padded).
pub const FRAME_PERSONAL: &[u8; 16] = b"zsol/0.2-frame\0\0";
/// SPEC §7 presentation domain separator.
pub const PRESENT_DOMAIN: &[u8] = b"zsol/0.2-present";
/// SPEC §6 C1 / §9 acceptance floor.
pub const DUST_FLOOR_ZAT: u64 = 10_000;
/// ZIP-302 memo field size.
pub const MEMO_LEN: usize = 512;

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
    #[error("invalid key or signature encoding")]
    KeyEncoding,
    #[error("invalid claim: {0}")]
    Claim(ReasonCode),
}

/// SPEC §10 reason codes (machine-readable, exhaustive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasonCode {
    Ok,
    ClaimOnly,
    ProtocolUnknown,
    NetworkMismatch,
    DustClaim,
    AnchorNotMined,
    NotSpentByAccount,
    MemoDigestMismatch,
    ClaimExceedsOutputs,
    AnchoredNoteSpent,
    StaleAttestation,
    PresentationInvalid,
    UfvkInvalid,
    ScanIncomplete,
    ChainUnreachable,
    RegistryUnreachable,
}

impl ReasonCode {
    pub const ALL: [ReasonCode; 16] = [
        Self::Ok,
        Self::ClaimOnly,
        Self::ProtocolUnknown,
        Self::NetworkMismatch,
        Self::DustClaim,
        Self::AnchorNotMined,
        Self::NotSpentByAccount,
        Self::MemoDigestMismatch,
        Self::ClaimExceedsOutputs,
        Self::AnchoredNoteSpent,
        Self::StaleAttestation,
        Self::PresentationInvalid,
        Self::UfvkInvalid,
        Self::ScanIncomplete,
        Self::ChainUnreachable,
        Self::RegistryUnreachable,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ClaimOnly => "claim-only",
            Self::ProtocolUnknown => "protocol-unknown",
            Self::NetworkMismatch => "network-mismatch",
            Self::DustClaim => "dust-claim",
            Self::AnchorNotMined => "anchor-not-mined",
            Self::NotSpentByAccount => "not-spent-by-account",
            Self::MemoDigestMismatch => "memo-digest-mismatch",
            Self::ClaimExceedsOutputs => "claim-exceeds-outputs",
            Self::AnchoredNoteSpent => "anchored-note-spent",
            Self::StaleAttestation => "stale-attestation",
            Self::PresentationInvalid => "presentation-invalid",
            Self::UfvkInvalid => "ufvk-invalid",
            Self::ScanIncomplete => "scan-incomplete",
            Self::ChainUnreachable => "chain-unreachable",
            Self::RegistryUnreachable => "registry-unreachable",
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
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ReasonCode::ALL
            .iter()
            .find(|r| r.as_str() == s)
            .copied()
            .ok_or(ZsolError::Json)
    }
}

/// SPEC §6 verification tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    Verified,
    ClaimOnly,
}

/// One row of the SPEC §6 check table (C1..C8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRow {
    pub check: String,
    pub ok: bool,
    pub code: ReasonCode,
    pub detail: String,
}

/// SPEC §4 attestation object (JSON, camelCase; amounts are zatoshi u64).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttestationObject {
    pub protocol: String,
    /// "test" | "main"
    pub network: String,
    pub agent_id: String,
    pub claim_zat: u64,
    /// "ironwood" | "sapling"
    pub pool: String,
    /// b64u of the 32-byte Ed25519 public key (SPEC §7).
    pub session_key: String,
    /// zsol-attn birthday height: the verifier's scan start.
    pub birthday: u32,
    pub issued_at: u64,
    pub max_age_hours: u32,
    /// 64 hex, display (RPC) byte order.
    pub anchor_txid: String,
    pub anchor_height: u32,
    /// b64u of the 37 significant memo bytes.
    pub memo_binding: String,
}

/// Fields excluded from the frame (SPEC §4): unknowable before the anchor is broadcast.
pub const FRAME_EXCLUDED: [&str; 3] = ["anchorTxid", "anchorHeight", "memoBinding"];

impl AttestationObject {
    /// Canonical bytes: key-sorted compact JSON (SPEC §4).
    pub fn canonical(&self) -> Result<Vec<u8>, ZsolError> {
        let mut v = serde_json::to_value(self).map_err(|_| ZsolError::Json)?;
        sort_json(&mut v);
        serde_json::to_vec(&v).map_err(|_| ZsolError::Json)
    }

    pub fn b64u(&self) -> Result<String, ZsolError> {
        self.canonical().map(|b| B64U.encode(b))
    }

    pub fn from_b64u(s: &str) -> Result<Self, ZsolError> {
        let bytes = B64U.decode(s.trim()).map_err(|_| ZsolError::Base64)?;
        serde_json::from_slice(&bytes).map_err(|_| ZsolError::Json)
    }

    /// canonical(frame) — the object minus [`FRAME_EXCLUDED`].
    pub fn frame_canonical(&self) -> Result<Vec<u8>, ZsolError> {
        let mut v = serde_json::to_value(self).map_err(|_| ZsolError::Json)?;
        if let serde_json::Value::Object(m) = &mut v {
            for k in FRAME_EXCLUDED {
                m.remove(k);
            }
        }
        sort_json(&mut v);
        serde_json::to_vec(&v).map_err(|_| ZsolError::Json)
    }

    /// SPEC §4: `D = BLAKE2b-256(personal = "zsol/0.2-frame\0\0", canonical(frame))`.
    pub fn frame_digest(&self) -> Result<[u8; 32], ZsolError> {
        Ok(frame_digest_bytes(&self.frame_canonical()?))
    }

    /// The 37-byte binding memo prefix this object commits to.
    pub fn expected_memo(&self) -> Result<[u8; AnchorMemo::LEN], ZsolError> {
        Ok(AnchorMemo {
            digest: self.frame_digest()?,
        }
        .encode())
    }

    /// Structural validation (SPEC §6 C1 + shape checks). Never trusts chain-visible fields.
    pub fn validate(&self, expected_network: &str) -> Result<(), ReasonCode> {
        if self.protocol != PROTOCOL {
            return Err(ReasonCode::ProtocolUnknown);
        }
        if self.network != expected_network {
            return Err(ReasonCode::NetworkMismatch);
        }
        if self.pool != "ironwood" && self.pool != "sapling" {
            return Err(ReasonCode::ProtocolUnknown);
        }
        validate_claim_zat(self.claim_zat)?;
        if self.agent_id.is_empty() || self.agent_id.len() > 64 {
            return Err(ReasonCode::ClaimOnly);
        }
        if decode_session_key(&self.session_key).is_err() {
            return Err(ReasonCode::PresentationInvalid);
        }
        if txid_internal(&self.anchor_txid).is_err() || self.anchor_height == 0 {
            return Err(ReasonCode::AnchorNotMined);
        }
        if self.max_age_hours == 0 {
            return Err(ReasonCode::StaleAttestation);
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

pub fn frame_digest_bytes(frame_canonical: &[u8]) -> [u8; 32] {
    let h = blake2b_simd::Params::new()
        .hash_length(32)
        .personal(FRAME_PERSONAL)
        .hash(frame_canonical);
    let mut out = [0u8; 32];
    out.copy_from_slice(h.as_bytes());
    out
}

/// Registry chain digest — SPEC §9:
/// `chainDigestₙ = BLAKE2b-256(chainDigestₙ₋₁ ‖ canonical(objectₙ))`, genesis = zero digest.
pub fn chain_digest(prev: [u8; 32], canonical_object: &[u8]) -> [u8; 32] {
    let mut h = Blake2b::<U32>::new();
    h.update(prev);
    h.update(canonical_object);
    h.finalize().into()
}

/// SPEC §5 binding memo prefix: `0xFF ‖ "zs02" ‖ D` (37 bytes), zero-padded to 512 on chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorMemo {
    pub digest: [u8; 32],
}

impl AnchorMemo {
    pub const LEN: usize = 37;

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut m = [0u8; Self::LEN];
        m[0] = MEMO_ARBITRARY;
        m[1..5].copy_from_slice(&ANCHOR_MEMO_TAG);
        m[5..37].copy_from_slice(&self.digest);
        m
    }

    /// Full 512-byte memo field as the wallet writes it.
    pub fn encode_padded(&self) -> [u8; MEMO_LEN] {
        let mut m = [0u8; MEMO_LEN];
        m[..Self::LEN].copy_from_slice(&self.encode());
        m
    }

    /// Parses the 37-byte prefix of a (possibly padded) memo field.
    pub fn decode(bytes: &[u8]) -> Result<Self, ZsolError> {
        if bytes.len() < Self::LEN || bytes[0] != MEMO_ARBITRARY || bytes[1..5] != ANCHOR_MEMO_TAG {
            return Err(ZsolError::MemoMalformed);
        }
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&bytes[5..37]);
        Ok(Self { digest })
    }
}

/// SPEC §11 repayment memo prefix: `0xFF ‖ "zsc1" ‖ loanId(16)` (21 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoanMemo {
    pub loan_id: [u8; 16],
}

impl LoanMemo {
    pub const LEN: usize = 21;

    pub fn encode_padded(&self) -> [u8; MEMO_LEN] {
        let mut m = [0u8; MEMO_LEN];
        m[0] = MEMO_ARBITRARY;
        m[1..5].copy_from_slice(&LOAN_MEMO_TAG);
        m[5..21].copy_from_slice(&self.loan_id);
        m
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ZsolError> {
        if bytes.len() < Self::LEN || bytes[0] != MEMO_ARBITRARY || bytes[1..5] != LOAN_MEMO_TAG {
            return Err(ZsolError::MemoMalformed);
        }
        let mut loan_id = [0u8; 16];
        loan_id.copy_from_slice(&bytes[5..21]);
        Ok(Self { loan_id })
    }
}

/// Display-order txid hex → internal byte order (the order hashed and signed).
pub fn txid_internal(display_hex: &str) -> Result<[u8; 32], ZsolError> {
    let v = hex::decode(display_hex).map_err(|_| ZsolError::KeyEncoding)?;
    if v.len() != 32 {
        return Err(ZsolError::KeyEncoding);
    }
    let mut out = [0u8; 32];
    for (i, b) in v.iter().rev().enumerate() {
        out[i] = *b;
    }
    Ok(out)
}

/// SPEC §7 verifier challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Challenge {
    /// b64u of 32 random bytes.
    pub nonce: String,
    pub verifier_id: String,
    pub expires_at: u64,
}

impl Challenge {
    pub fn nonce_bytes(&self) -> Result<[u8; 32], ZsolError> {
        let v = B64U.decode(&self.nonce).map_err(|_| ZsolError::Base64)?;
        v.try_into().map_err(|_| ZsolError::KeyEncoding)
    }
}

/// SPEC §7: `"zsol/0.2-present" ‖ D ‖ anchorTxid(internal) ‖ nonce ‖ u64le(expiresAt) ‖ utf8(verifierId)`.
pub fn presentation_message(
    obj: &AttestationObject,
    challenge: &Challenge,
) -> Result<Vec<u8>, ZsolError> {
    let mut m = Vec::with_capacity(16 + 32 + 32 + 32 + 8 + challenge.verifier_id.len());
    m.extend_from_slice(PRESENT_DOMAIN);
    m.extend_from_slice(&obj.frame_digest()?);
    m.extend_from_slice(&txid_internal(&obj.anchor_txid)?);
    m.extend_from_slice(&challenge.nonce_bytes()?);
    m.extend_from_slice(&challenge.expires_at.to_le_bytes());
    m.extend_from_slice(challenge.verifier_id.as_bytes());
    Ok(m)
}

pub fn decode_session_key(b64u: &str) -> Result<VerifyingKey, ZsolError> {
    let v = B64U.decode(b64u).map_err(|_| ZsolError::Base64)?;
    let arr: [u8; 32] = v.try_into().map_err(|_| ZsolError::KeyEncoding)?;
    VerifyingKey::from_bytes(&arr).map_err(|_| ZsolError::KeyEncoding)
}

pub fn encode_session_key(vk: &VerifyingKey) -> String {
    B64U.encode(vk.as_bytes())
}

/// Attestor side of SPEC §7. Returns the b64u signature.
pub fn sign_presentation(
    sk: &SigningKey,
    obj: &AttestationObject,
    challenge: &Challenge,
) -> Result<String, ZsolError> {
    let msg = presentation_message(obj, challenge)?;
    Ok(B64U.encode(sk.sign(&msg).to_bytes()))
}

/// Verifier side of SPEC §7 (C8). Strict verification (rejects malleable signatures).
pub fn verify_presentation(
    obj: &AttestationObject,
    challenge: &Challenge,
    sig_b64u: &str,
    now: u64,
) -> Result<(), ReasonCode> {
    if now > challenge.expires_at {
        return Err(ReasonCode::PresentationInvalid);
    }
    let vk = decode_session_key(&obj.session_key).map_err(|_| ReasonCode::PresentationInvalid)?;
    let raw = B64U
        .decode(sig_b64u)
        .map_err(|_| ReasonCode::PresentationInvalid)?;
    let arr: [u8; 64] = raw
        .try_into()
        .map_err(|_| ReasonCode::PresentationInvalid)?;
    let msg = presentation_message(obj, challenge).map_err(|_| ReasonCode::PresentationInvalid)?;
    vk.verify_strict(&msg, &Signature::from_bytes(&arr))
        .map_err(|_| ReasonCode::PresentationInvalid)
}

pub fn b64u_encode(bytes: &[u8]) -> String {
    B64U.encode(bytes)
}

pub fn b64u_decode(s: &str) -> Result<Vec<u8>, ZsolError> {
    B64U.decode(s.trim()).map_err(|_| ZsolError::Base64)
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
            let mut entries: Vec<(String, serde_json::Value)> =
                std::mem::take(m).into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, mut val) in entries {
                sort_json(&mut val);
                m.insert(k, val);
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(sort_json),
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

/// Independent golden: python3 `hashlib.blake2b(b'{"golden":"vector"}', digest_size=32,
/// person=b"zsol/0.2-frame\0\0").hexdigest()`.
pub const GOLDEN_FRAME_DIGEST_HEX: &str =
    "b4683e4df7db4da9a2416ec5890420ef1b62f2ec54d4ad0402bd66727d7d2256";

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sk() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn sample() -> AttestationObject {
        let mut o = AttestationObject {
            protocol: PROTOCOL.into(),
            network: "test".into(),
            agent_id: "agent-0001".into(),
            claim_zat: 10_000_000,
            pool: "ironwood".into(),
            session_key: encode_session_key(&sk().verifying_key()),
            birthday: 4_540_000,
            issued_at: 1_793_000_000,
            max_age_hours: 24,
            anchor_txid: "ab".repeat(32),
            anchor_height: 4_544_800,
            memo_binding: String::new(),
        };
        o.memo_binding = b64u_encode(&o.expected_memo().unwrap());
        o
    }

    fn challenge() -> Challenge {
        Challenge {
            nonce: b64u_encode(&[3u8; 32]),
            verifier_id: "underwriter-1".into(),
            expires_at: 2_000_000_000,
        }
    }

    #[test]
    fn golden_frame_digest_matches_python_hashlib() {
        assert_eq!(
            hex::encode(frame_digest_bytes(br#"{"golden":"vector"}"#)),
            GOLDEN_FRAME_DIGEST_HEX
        );
    }

    #[test]
    fn frame_excludes_exactly_the_post_broadcast_fields() {
        let o = sample();
        let mut x = o.clone();
        x.anchor_txid = "cd".repeat(32);
        x.anchor_height += 9;
        x.memo_binding = "AAAA".into();
        assert_eq!(o.frame_digest().unwrap(), x.frame_digest().unwrap());
        for edit in [
            |o: &mut AttestationObject| o.claim_zat += 1,
            |o: &mut AttestationObject| o.agent_id.push('x'),
            |o: &mut AttestationObject| o.network = "main".into(),
            |o: &mut AttestationObject| o.pool = "sapling".into(),
            |o: &mut AttestationObject| o.birthday += 1,
            |o: &mut AttestationObject| o.issued_at += 1,
            |o: &mut AttestationObject| o.max_age_hours += 1,
            |o: &mut AttestationObject| {
                o.session_key =
                    encode_session_key(&SigningKey::from_bytes(&[8u8; 32]).verifying_key())
            },
        ] {
            let mut e = o.clone();
            edit(&mut e);
            assert_ne!(o.frame_digest().unwrap(), e.frame_digest().unwrap());
        }
    }

    #[test]
    fn object_roundtrip_and_unknown_fields_rejected() {
        let o = sample();
        assert_eq!(AttestationObject::from_b64u(&o.b64u().unwrap()).unwrap(), o);
        let mut v = serde_json::to_value(&o).unwrap();
        v["ufvk"] = "uviewtest1...".into();
        let s = b64u_encode(&serde_json::to_vec(&v).unwrap());
        assert_eq!(AttestationObject::from_b64u(&s), Err(ZsolError::Json));
    }

    #[test]
    fn memo_codec_and_padding() {
        let m = AnchorMemo { digest: [9u8; 32] };
        let padded = m.encode_padded();
        assert_eq!(padded[0], 0xFF);
        assert_eq!(&padded[1..5], b"zs02");
        assert!(padded[37..].iter().all(|b| *b == 0));
        assert_eq!(AnchorMemo::decode(&padded).unwrap(), m);
        assert!(AnchorMemo::decode(&padded[..36]).is_err());
        let mut text = padded;
        text[0] = b'z';
        assert!(AnchorMemo::decode(&text).is_err());
        let l = LoanMemo { loan_id: [5u8; 16] };
        assert_eq!(LoanMemo::decode(&l.encode_padded()).unwrap(), l);
        assert!(LoanMemo::decode(&padded).is_err());
    }

    #[test]
    fn presentation_accepts_genuine_and_rejects_every_tamper() {
        let o = sample();
        let c = challenge();
        let sig = sign_presentation(&sk(), &o, &c).unwrap();
        assert_eq!(verify_presentation(&o, &c, &sig, 1_900_000_000), Ok(()));
        let bad = Err(ReasonCode::PresentationInvalid);
        let mut c2 = c.clone();
        c2.nonce = b64u_encode(&[4u8; 32]);
        assert_eq!(verify_presentation(&o, &c2, &sig, 0), bad);
        let mut c3 = c.clone();
        c3.verifier_id = "underwriter-2".into();
        assert_eq!(verify_presentation(&o, &c3, &sig, 0), bad);
        let mut o2 = o.clone();
        o2.anchor_txid = "cd".repeat(32);
        assert_eq!(verify_presentation(&o2, &c, &sig, 0), bad);
        let mut o3 = o.clone();
        o3.claim_zat += 1;
        assert_eq!(verify_presentation(&o3, &c, &sig, 0), bad);
        assert_eq!(verify_presentation(&o, &c, &sig, c.expires_at + 1), bad);
        let relay = sign_presentation(&SigningKey::from_bytes(&[1u8; 32]), &o, &c).unwrap();
        assert_eq!(verify_presentation(&o, &c, &relay, 0), bad);
    }

    /// RFC 8032 §7.1 TEST 1 (empty message) — the Ed25519 implementation is the standard one.
    #[test]
    fn rfc8032_test_vector_1() {
        let secret: [u8; 32] =
            hex::decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
                .unwrap()
                .try_into()
                .unwrap();
        let k = SigningKey::from_bytes(&secret);
        assert_eq!(
            hex::encode(k.verifying_key().as_bytes()),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        assert_eq!(
            hex::encode(k.sign(b"").to_bytes()),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
    }

    #[test]
    fn txid_byte_order_is_reversed() {
        let mut disp = "00".repeat(31);
        disp.push_str("ff");
        let internal = txid_internal(&disp).unwrap();
        assert_eq!(internal[0], 0xff);
        assert!(txid_internal("zz").is_err());
    }

    #[test]
    fn validate_paths() {
        let o = sample();
        assert_eq!(o.validate("test"), Ok(()));
        assert_eq!(o.validate("main"), Err(ReasonCode::NetworkMismatch));
        let mut x = o.clone();
        x.protocol = "zsol/0.1".into();
        assert_eq!(x.validate("test"), Err(ReasonCode::ProtocolUnknown));
        let mut x = o.clone();
        x.claim_zat = 9_999;
        assert_eq!(x.validate("test"), Err(ReasonCode::DustClaim));
        let mut x = o.clone();
        x.session_key = "short".into();
        assert_eq!(x.validate("test"), Err(ReasonCode::PresentationInvalid));
    }

    #[test]
    fn reason_codes_roundtrip_and_serde_match_as_str() {
        for r in ReasonCode::ALL {
            assert_eq!(r.as_str().parse::<ReasonCode>().unwrap(), r);
            assert_eq!(serde_json::to_value(r).unwrap(), r.as_str());
        }
    }

    #[test]
    fn chain_digest_depends_on_prev_and_object() {
        let z = [0u8; 32];
        assert_ne!(chain_digest(z, b"a"), chain_digest([1u8; 32], b"a"));
        assert_ne!(chain_digest(z, b"a"), chain_digest(z, b"b"));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10_000))]
        #[test]
        fn prop_memo_roundtrip(dg in proptest::array::uniform32(any::<u8>())) {
            let m = AnchorMemo { digest: dg };
            prop_assert_eq!(AnchorMemo::decode(&m.encode_padded()).unwrap(), m);
        }
    }
}
