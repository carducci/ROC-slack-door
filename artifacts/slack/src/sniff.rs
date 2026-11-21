//! A file's media type, decided from its leading bytes alone. A filename and the type Slack
//! reports are never asked.
//!
//! This stands in for `ikigai-sniff`, which this repository cannot yet depend on (the draft
//! PR's question): it knows the image types a channel may list, and nothing else, so anything
//! it does not recognise has no type and is refused.

/// The media types `sniff` can confirm.
pub const SNIFFABLE: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// The media type `bytes` begin as, if it is one of `SNIFFABLE`.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_image_type_is_known_by_its_signature() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), Some("image/png"));
        assert_eq!(sniff(b"\xff\xd8\xff\xe0\0\x10JFIF"), Some("image/jpeg"));
        assert_eq!(sniff(b"GIF89a\x01\0\x01\0"), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\x24\0\0\0WEBPVP8 "), Some("image/webp"));
    }

    #[test]
    fn an_executable_or_anything_else_has_no_type() {
        assert_eq!(sniff(b"MZ\x90\0\x03\0\0\0"), None, "a Windows executable");
        assert_eq!(sniff(b"\x7fELF\x02\x01\x01\0"), None, "an ELF executable");
        assert_eq!(sniff(b"<html><body>sign in</body></html>"), None);
        assert_eq!(sniff(b"\x89PNG"), None, "a truncated signature");
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVE"), None, "a RIFF that is not WebP");
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn every_type_it_confirms_is_listed() {
        for bytes in [&b"\x89PNG\r\n\x1a\n"[..], b"\xff\xd8\xff", b"GIF87a", b"RIFF0000WEBP"] {
            assert!(SNIFFABLE.contains(&sniff(bytes).unwrap()));
        }
    }
}
