//! The recovery key in a form a person can write down.
//!
//! 32 random bytes become 52 characters of Crockford base32 (no `I`, `L`,
//! `O`, or `U`, so handwriting is unambiguous) plus a 4-character checksum
//! from SHA-256 of the key, shown as 14 groups of 4:
//! `XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-CCCC`.
//! Parsing is case-insensitive, ignores separators, maps `I`/`L` to `1`
//! and `O` to `0`, and rejects any single mistyped character through the
//! checksum.

use zeroize::Zeroizing;

use super::CryptoError;
use super::keys::RecoveryKey;
use super::primitives::{ct_eq, sha256};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const DATA_CHARS: usize = 52;
const CHECK_CHARS: usize = 4;
const GROUP: usize = 4;

/// Why an entered recovery key was rejected. Never carries the entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryKeyError {
    #[error("recovery key must have {expected} characters, got {found}")]
    Length { expected: usize, found: usize },
    #[error("recovery key has an invalid character at position {0}")]
    Character(usize),
    #[error("recovery key checksum does not match; check for a typo")]
    Checksum,
}

fn checksum(key: &[u8; 32]) -> [u8; CHECK_CHARS] {
    // First 20 bits of the hash, as four base32 digits.
    let h = sha256(key);
    let bits = (u32::from(h[0]) << 12) | (u32::from(h[1]) << 4) | (u32::from(h[2]) >> 4);
    let mut out = [0u8; CHECK_CHARS];
    for (i, slot) in out.iter_mut().enumerate() {
        let shift = 5 * (CHECK_CHARS - 1 - i);
        *slot = ALPHABET[((bits >> shift) & 31) as usize];
    }
    out
}

fn encode_data(key: &[u8; 32]) -> [u8; DATA_CHARS] {
    let mut out = [0u8; DATA_CHARS];
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut i = 0;
    for byte in key {
        acc = (acc << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out[i] = ALPHABET[((acc >> bits) & 31) as usize];
            i += 1;
        }
    }
    // 256 = 51 * 5 + 1: one bit left, padded with four zero bits.
    out[i] = ALPHABET[((acc << (5 - bits)) & 31) as usize];
    out
}

/// Render the key for display or printing.
pub fn render(key: &RecoveryKey) -> String {
    let data = encode_data(key.as_bytes());
    let check = checksum(key.as_bytes());
    let all: Vec<u8> = data.iter().chain(check.iter()).copied().collect();
    all.chunks(GROUP)
        .map(|g| std::str::from_utf8(g).expect("ascii"))
        .collect::<Vec<_>>()
        .join("-")
}

fn digit(c: u8) -> Option<u8> {
    let c = match c.to_ascii_uppercase() {
        b'I' | b'L' => b'1',
        b'O' => b'0',
        other => other,
    };
    ALPHABET.iter().position(|d| *d == c).map(|p| p as u8)
}

/// Parse what a person typed. Separators and case are ignored.
pub fn parse(entered: &str) -> Result<RecoveryKey, RecoveryKeyError> {
    let mut digits: Zeroizing<Vec<u8>> =
        Zeroizing::new(Vec::with_capacity(DATA_CHARS + CHECK_CHARS));
    for (pos, c) in entered.bytes().enumerate() {
        if c == b'-' || c == b' ' || c == b'\n' || c == b'\t' {
            continue;
        }
        digits.push(digit(c).ok_or(RecoveryKeyError::Character(pos))?);
    }
    if digits.len() != DATA_CHARS + CHECK_CHARS {
        return Err(RecoveryKeyError::Length {
            expected: DATA_CHARS + CHECK_CHARS,
            found: digits.len(),
        });
    }
    let mut key = Zeroizing::new([0u8; 32]);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut i = 0;
    for d in &digits[..DATA_CHARS] {
        acc = (acc << 5) | u32::from(*d);
        bits += 5;
        if bits >= 8 && i < 32 {
            bits -= 8;
            key[i] = ((acc >> bits) & 0xff) as u8;
            i += 1;
        }
    }
    // The last character carries one data bit and four padding bits.
    if acc & ((1 << bits) - 1) != 0 {
        return Err(RecoveryKeyError::Checksum);
    }
    let expected = checksum(&key);
    let entered_check: Vec<u8> = digits[DATA_CHARS..]
        .iter()
        .map(|d| ALPHABET[*d as usize])
        .collect();
    if !ct_eq(&expected, &entered_check) {
        return Err(RecoveryKeyError::Checksum);
    }
    Ok(RecoveryKey::from_bytes(*key))
}

impl From<RecoveryKeyError> for CryptoError {
    fn from(e: RecoveryKeyError) -> Self {
        CryptoError::RecoveryKey(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_parse_round_trip() {
        for _ in 0..50 {
            let key = RecoveryKey::generate().unwrap();
            let text = render(&key);
            assert_eq!(text.len(), 14 * 4 + 13);
            assert!(text.split('-').all(|g| g.len() == 4));
            let back = parse(&text).unwrap();
            assert_eq!(back.as_bytes(), key.as_bytes());
            let sloppy = text.to_lowercase().replace('-', " ");
            assert_eq!(parse(&sloppy).unwrap().as_bytes(), key.as_bytes());
        }
    }

    #[test]
    fn a_mistyped_character_is_rejected() {
        let key = RecoveryKey::generate().unwrap();
        let text = render(&key);
        let bytes = text.as_bytes();
        for pos in (0..bytes.len()).step_by(7) {
            if bytes[pos] == b'-' {
                continue;
            }
            let replacement = if bytes[pos] == b'7' { b'8' } else { b'7' };
            let mut wrong = bytes.to_vec();
            wrong[pos] = replacement;
            let wrong = String::from_utf8(wrong).unwrap();
            assert_eq!(
                parse(&wrong).unwrap_err(),
                RecoveryKeyError::Checksum,
                "position {pos}"
            );
        }
        assert!(matches!(
            parse(&text[..20]),
            Err(RecoveryKeyError::Length { .. })
        ));
        let mut bad = text.clone();
        bad.replace_range(0..1, "U");
        assert_eq!(parse(&bad).unwrap_err(), RecoveryKeyError::Character(0));
    }

    #[test]
    fn confusable_characters_are_normalized() {
        let key = RecoveryKey::from_bytes([0u8; 32]);
        let text = render(&key);
        let confusable = text.replacen('0', "O", 3).replacen('1', "l", 1);
        assert_eq!(parse(&confusable).unwrap().as_bytes(), key.as_bytes());
    }
}
