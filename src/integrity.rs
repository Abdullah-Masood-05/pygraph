//! HMAC-SHA256 payload signing and single-payload framing.
//!
//! Wire format of a signed payload:
//!
//! ```text
//! header (11 bytes, flags byte has FLAG_HMAC set) | tables + records | tag (32 bytes)
//! ```
//!
//! The tag is HMAC-SHA256(key, every preceding byte of the payload, header
//! included). Unsigned payloads are unchanged (no flag, no tag).
//!
//! The format has no total-length field, so the end of a payload is only
//! known after its structure has been walked. [`decode_one`] therefore first
//! tries the common case "the tag is the last 32 bytes of the buffer" and
//! verifies it before parsing anything beyond the header. Only if that fails
//! (trailing bytes, multi-record streams) does it run the bounds-checked,
//! Python-free phase-1 structural scan to locate the tag, and it verifies the
//! tag before any Python object is created.

use hmac::{Hmac, KeyInit, Mac};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyMemoryView};
use sha2::Sha256;

use crate::format::decoder::{self, DecodeError, DecodedGraph};
use crate::format::{FLAG_HMAC, MAGIC_LEGACY, MAGIC_WRITE};

type HmacSha256 = Hmac<Sha256>;

pub const TAG_LEN: usize = 32;
/// Offset of the flags byte: magic (4) + format version (2) + schema version (4).
pub const FLAGS_OFFSET: usize = 10;
const HEADER_LEN: usize = FLAGS_OFFSET + 1;

pub const MSG_SIGNED_NO_KEY: &str = "payload is signed; pass hmac_key";
pub const MSG_NOT_SIGNED: &str = "payload is not signed";
pub const MSG_MISMATCH: &str = "HMAC verification failed: payload was tampered with or the key is wrong";
pub const MSG_EMPTY_KEY: &str = "hmac_key must be a non-empty bytes-like object";

/// An `hmac_key` argument: any bytes-like object (bytes, bytearray,
/// memoryview, other buffers), copied.
pub struct HmacKey(pub Vec<u8>);

impl<'py> FromPyObject<'py> for HmacKey {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        if let Ok(b) = ob.downcast::<PyBytes>() {
            return Ok(HmacKey(b.as_bytes().to_vec()));
        }
        if let Ok(b) = ob.downcast::<PyByteArray>() {
            return Ok(HmacKey(b.to_vec()));
        }
        PyMemoryView::from(ob)
            .and_then(|mv| mv.call_method0("tobytes"))
            .and_then(|b| Ok(HmacKey(b.downcast::<PyBytes>()?.as_bytes().to_vec())))
            .map_err(|_| {
                pyo3::exceptions::PyTypeError::new_err(
                    "hmac_key must be a bytes-like object",
                )
            })
    }
}

/// `Option<HmacKey>` -> `Option<&[u8]>`.
pub fn key_slice(key: &Option<HmacKey>) -> Option<&[u8]> {
    key.as_ref().map(|k| k.0.as_slice())
}

fn mac(key: &[u8]) -> HmacSha256 {
    // HMAC accepts keys of any length.
    <HmacSha256 as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length")
}

/// HMAC-SHA256 tag of `data`.
pub fn compute_tag(key: &[u8], data: &[u8]) -> [u8; TAG_LEN] {
    let mut m = mac(key);
    m.update(data);
    m.finalize().into_bytes().into()
}

/// Constant-time check of `tag` against HMAC-SHA256(key, data).
pub fn verify_tag(key: &[u8], data: &[u8], tag: &[u8]) -> bool {
    let mut m = mac(key);
    m.update(data);
    m.verify_slice(tag).is_ok()
}

/// Turns an unsigned encoder output into a signed payload: sets FLAG_HMAC in
/// the header and appends the tag.
pub fn sign(unsigned: &[u8], key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(unsigned.len() + TAG_LEN);
    out.extend_from_slice(unsigned);
    out[FLAGS_OFFSET] |= FLAG_HMAC;
    let tag = compute_tag(key, &out);
    out.extend_from_slice(&tag);
    out
}

/// Returns the header flags if `data` starts with a complete, valid header.
fn header_flags(data: &[u8]) -> Option<u8> {
    if data.len() < HEADER_LEN {
        return None;
    }
    let magic = &data[..4];
    if magic != MAGIC_WRITE && magic != MAGIC_LEGACY {
        return None;
    }
    Some(data[FLAGS_OFFSET])
}

pub enum Failure {
    Decode(DecodeError),
    Hmac(&'static str),
}

impl From<DecodeError> for Failure {
    fn from(e: DecodeError) -> Self {
        Failure::Decode(e)
    }
}

/// Validates (phase 1) exactly one payload at the start of `data`, checking
/// the HMAC first when the payload is signed. Returns the decoded graph and
/// the number of bytes the payload occupies (including the tag).
///
/// `whole_buffer`: the caller expects `data` to hold a single payload
/// (`loads`). Then a signed payload whose tag cannot be located is reported
/// as an HMAC failure; otherwise (stream reading) as a truncation, so that a
/// streaming caller can read more input and retry.
///
/// Pure Rust; safe to call with the GIL released.
pub fn decode_one<'a>(
    data: &'a [u8],
    key: Option<&[u8]>,
    whole_buffer: bool,
) -> Result<(DecodedGraph<'a>, usize), Failure> {
    let signed = matches!(header_flags(data), Some(f) if f & FLAG_HMAC != 0);
    let key = match (signed, key) {
        (false, None) => {
            let g = decoder::decode(data)?;
            let end = g.end;
            return Ok((g, end));
        }
        (false, Some(_)) => {
            if header_flags(data).is_none() {
                // Not even a valid header: report the structural problem.
                decoder::decode(data)?;
            }
            return Err(Failure::Hmac(MSG_NOT_SIGNED));
        }
        (true, None) => return Err(Failure::Hmac(MSG_SIGNED_NO_KEY)),
        (true, Some(k)) => k,
    };

    // Fast path: the tag is the last TAG_LEN bytes; verify before parsing.
    if data.len() >= HEADER_LEN + TAG_LEN {
        let split = data.len() - TAG_LEN;
        if verify_tag(key, &data[..split], &data[split..]) {
            let g = decoder::decode(&data[..split])?;
            return Ok((g, data.len()));
        }
    }

    // Slow path: locate the end of the records, then verify the tag there.
    let g = match decoder::decode(data) {
        Ok(g) => g,
        Err(e) => {
            return Err(if whole_buffer {
                Failure::Hmac(MSG_MISMATCH)
            } else {
                Failure::Decode(e)
            })
        }
    };
    let end = g.end;
    if data.len() - end < TAG_LEN {
        return Err(if whole_buffer {
            Failure::Hmac(MSG_MISMATCH)
        } else {
            Failure::Decode(DecodeError::Truncated("HMAC tag"))
        });
    }
    if !verify_tag(key, &data[..end], &data[end..end + TAG_LEN]) {
        return Err(Failure::Hmac(MSG_MISMATCH));
    }
    Ok((g, end + TAG_LEN))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4231_case_2() {
        let tag = compute_tag(b"Jefe", b"what do ya want for nothing?");
        let hex: String = tag.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn sign_sets_flag_and_verifies() {
        let mut unsigned = Vec::new();
        crate::format::write_header(&mut unsigned, 0);
        unsigned.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0x01]);
        let signed = sign(&unsigned, b"k");
        assert_eq!(signed.len(), unsigned.len() + TAG_LEN);
        assert_eq!(signed[FLAGS_OFFSET] & FLAG_HMAC, FLAG_HMAC);
        let (_, used) = decode_one(&signed, Some(b"k"), true).ok().unwrap();
        assert_eq!(used, signed.len());
        // Trailing bytes: slow path finds the tag.
        let mut stream = signed.clone();
        stream.extend_from_slice(&unsigned);
        let (_, used) = decode_one(&stream, Some(b"k"), false).ok().unwrap();
        assert_eq!(used, signed.len());
        assert!(matches!(
            decode_one(&signed, Some(b"wrong"), true),
            Err(Failure::Hmac(MSG_MISMATCH))
        ));
        assert!(matches!(
            decode_one(&signed, None, true),
            Err(Failure::Hmac(MSG_SIGNED_NO_KEY))
        ));
        assert!(matches!(
            decode_one(&unsigned, Some(b"k"), true),
            Err(Failure::Hmac(MSG_NOT_SIGNED))
        ));
        let (_, used) = decode_one(&unsigned, None, true).ok().unwrap();
        assert_eq!(used, unsigned.len());
    }
}
