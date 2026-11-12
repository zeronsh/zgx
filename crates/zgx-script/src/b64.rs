//! Bytes a script hands over as base64 text: a Luau string from a server or
//! a host's call carries text, so a font's or a picture's bytes come
//! encoded.
use base64::Engine as _;

/// `encoded`'s bytes (standard base64, padded), refused before decoding if
/// they would be more than `limit`: `what` names them in the error.
pub(crate) fn decode(encoded: &[u8], limit: usize, what: &str) -> Result<Vec<u8>, String> {
    if encoded.len() / 4 * 3 > limit {
        return Err(format!("{what} is {} MiB at most", limit >> 20));
    }
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| format!("{what}: `base64` isn't base64 ({e})"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_padded_base64_within_its_limit() {
        assert_eq!(super::decode(b"AP+A", 3, "x").unwrap(), [0, 255, 128]);
        assert!(
            super::decode(b"AP+AAP+A", 3, "x")
                .unwrap_err()
                .contains("at most")
        );
        assert!(
            super::decode(b"not base64!", 64, "x")
                .unwrap_err()
                .contains("isn't base64")
        );
    }
}
