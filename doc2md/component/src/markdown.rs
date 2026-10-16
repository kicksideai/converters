use crate::types::Converted;

const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

// A blob is treated as text when it is valid UTF-8 with no NUL bytes, after an
// optional UTF-8 BOM. Binary formats (images, DWG, legacy Office) carry NUL bytes
// or invalid UTF-8 and are excluded.
pub fn is_text(blob: &[u8]) -> bool {
    let body = blob.strip_prefix(BOM).unwrap_or(blob);
    !body.contains(&0) && std::str::from_utf8(body).is_ok()
}

// Plain text and Markdown pass through faithfully: strip a UTF-8 BOM, normalize
// CRLF to LF, and drop a trailing newline. No tidying, so Markdown hard breaks
// and intentional spacing survive.
pub fn convert(blob: &[u8]) -> Result<Converted, String> {
    let body = blob.strip_prefix(BOM).unwrap_or(blob);
    let raw = std::str::from_utf8(body).map_err(|e| e.to_string())?;
    let markdown = raw.replace("\r\n", "\n").trim_end_matches('\n').to_string();
    Ok(Converted {
        markdown,
        images: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_text_accepts_utf8_and_rejects_binary() {
        assert!(is_text(b"# Title\n\nhello"));
        assert!(is_text(&[0xEF, 0xBB, 0xBF, b'h', b'i']));
        assert!(!is_text(&[0x00, 0x01, 0x02]));
        assert!(!is_text(&[0xFF, 0xFE, 0x41]));
    }

    #[test]
    fn convert_strips_bom_and_normalizes_newlines() {
        let blob = [BOM, b"# H\r\n\r\nbody\r\n".as_ref()].concat();
        let c = convert(&blob).unwrap();
        assert_eq!(c.markdown, "# H\n\nbody");
        assert!(c.images.is_empty());
    }

    #[test]
    fn convert_preserves_markdown_hard_breaks() {
        let c = convert(b"line one  \nline two").unwrap();
        assert_eq!(c.markdown, "line one  \nline two");
    }
}
