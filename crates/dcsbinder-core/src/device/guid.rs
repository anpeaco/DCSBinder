//! DCS-form GUID (`{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}`).
//!
//! See `docs/FILE_FORMAT.md` for context. The bytes correspond to a Windows
//! `DirectInput` instance GUID; we just provide parse/format helpers and let the
//! `DirectInput` enumerator pass us 16-byte values it gets from `DIDEVICEINSTANCEW`.

use std::fmt;

/// A 16-byte GUID, displayed in DCS's canonical `{8-4-4-4-12}` upper-hex form.
///
/// Internal byte layout matches the Windows `GUID` struct exactly so that
/// `DIDEVICEINSTANCEW::guidInstance` bytes can be passed in directly without
/// re-ordering. Microsoft GUIDs are little-endian in their first three fields,
/// big-endian in the last two — the byte layout below preserves the
/// canonical hex-string interpretation that DCS, `regedit`, and Windows tools
/// all use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Guid(pub [u8; 16]);

#[derive(Debug, thiserror::Error)]
pub enum GuidParseError {
    #[error("expected `{{8-4-4-4-12}}` GUID or bare `8-4-4-4-12`, got `{0}`")]
    BadShape(String),
    #[error("non-hex character in GUID `{0}`")]
    NonHex(String),
}

impl Guid {
    /// All-zero GUID (`{00000000-0000-0000-0000-000000000000}`).
    #[must_use]
    pub const fn nil() -> Self {
        Self([0; 16])
    }

    /// Parse a DCS-style GUID string. Accepts both `{....-....-....-....-............}`
    /// and bare `....-....-....-....-............`. Case-insensitive.
    pub fn parse_dcs(s: &str) -> Result<Self, GuidParseError> {
        let trimmed = s.trim();
        let inner = trimmed
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .unwrap_or(trimmed);

        // Expect exactly 8-4-4-4-12 with dashes.
        let parts: Vec<&str> = inner.split('-').collect();
        if parts.len() != 5
            || parts[0].len() != 8
            || parts[1].len() != 4
            || parts[2].len() != 4
            || parts[3].len() != 4
            || parts[4].len() != 12
        {
            return Err(GuidParseError::BadShape(s.to_string()));
        }

        let mut bytes = [0u8; 16];
        let hex: String = parts.concat();
        debug_assert_eq!(hex.len(), 32);
        for (byte, chunk) in bytes.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
            let hi = hex_nibble(chunk[0]).ok_or_else(|| GuidParseError::NonHex(s.to_string()))?;
            let lo = hex_nibble(chunk[1]).ok_or_else(|| GuidParseError::NonHex(s.to_string()))?;
            *byte = (hi << 4) | lo;
        }
        Ok(Self(bytes))
    }

    /// Format as DCS canonical `{8-4-4-4-12}` upper-case hex (the form DCS uses
    /// in its bind filenames).
    #[must_use]
    pub fn to_dcs_string(self) -> String {
        let b = self.0;
        format!(
            "{{{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            b[0], b[1], b[2], b[3],
            b[4], b[5],
            b[6], b[7],
            b[8], b[9],
            b[10], b[11], b[12], b[13], b[14], b[15],
        )
    }

    /// Format without braces (`8-4-4-4-12` upper-case), useful for log lines
    /// that want the GUID inline without nested punctuation.
    #[must_use]
    pub fn to_bare_string(self) -> String {
        let full = self.to_dcs_string();
        full[1..full.len() - 1].to_string()
    }

    /// Raw underlying bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Construct from raw bytes (e.g. a Windows `GUID` struct's bytes).
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_dcs_string())
    }
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE_DCS: &str = "{4E50F3B0-2309-11ee-8015-444553540000}";

    #[test]
    fn parses_braced_and_bare() {
        let g1 = Guid::parse_dcs(REFERENCE_DCS).unwrap();
        let g2 = Guid::parse_dcs("4E50F3B0-2309-11ee-8015-444553540000").unwrap();
        assert_eq!(g1, g2);
    }

    #[test]
    fn round_trips_canonical_form() {
        let g = Guid::parse_dcs(REFERENCE_DCS).unwrap();
        assert_eq!(g.to_dcs_string(), REFERENCE_DCS.to_uppercase());
    }

    #[test]
    fn case_insensitive() {
        let upper = Guid::parse_dcs(REFERENCE_DCS).unwrap();
        let lower = Guid::parse_dcs(&REFERENCE_DCS.to_lowercase()).unwrap();
        assert_eq!(upper, lower);
    }

    #[test]
    fn known_byte_layout() {
        let g = Guid::parse_dcs(REFERENCE_DCS).unwrap();
        // Hex pairs (left-to-right) -> bytes (index 0..15).
        assert_eq!(g.as_bytes()[0], 0x4E);
        assert_eq!(g.as_bytes()[1], 0x50);
        assert_eq!(g.as_bytes()[2], 0xF3);
        assert_eq!(g.as_bytes()[3], 0xB0);
        assert_eq!(g.as_bytes()[4], 0x23);
        assert_eq!(g.as_bytes()[5], 0x09);
        assert_eq!(g.as_bytes()[6], 0x11);
        assert_eq!(g.as_bytes()[7], 0xEE);
        // Trailing DirectInput marker bytes: "DEST\0\0".
        assert_eq!(&g.as_bytes()[10..], &[0x44, 0x45, 0x53, 0x54, 0x00, 0x00]);
    }

    #[test]
    fn rejects_bad_shapes() {
        assert!(Guid::parse_dcs("notaguid").is_err());
        assert!(Guid::parse_dcs("{too-short}").is_err());
        assert!(Guid::parse_dcs("{4E50F3B0-2309-11ee-8015-44455354000Z}").is_err()); // non-hex
        assert!(Guid::parse_dcs("4E50F3B0_2309_11ee_8015_444553540000").is_err());
        // wrong separator
    }

    #[test]
    fn nil_is_all_zeros() {
        assert_eq!(
            Guid::nil().to_dcs_string(),
            "{00000000-0000-0000-0000-000000000000}"
        );
    }

    #[test]
    fn bare_string_strips_braces() {
        let g = Guid::parse_dcs(REFERENCE_DCS).unwrap();
        assert!(!g.to_bare_string().starts_with('{'));
        assert!(!g.to_bare_string().ends_with('}'));
        assert_eq!(g.to_bare_string().len(), 36); // 8+1+4+1+4+1+4+1+12
    }
}
