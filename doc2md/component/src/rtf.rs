use crate::types::{Converted, Image};
use rtf_parser::{Lexer, Parser};

pub fn convert(blob: &[u8], extract_images: bool) -> Result<Converted, String> {
    let text = String::from_utf8_lossy(blob);
    // Picture groups hold hex/binary image data that must not leak into the text;
    // carve the images out and strip the groups before lexing for text.
    let (cleaned, picts) = strip_pict_groups(&text, extract_images);
    let cleaned = restore_par_breaks(&cleaned);
    let tokens = Lexer::scan(&cleaned).map_err(|e| format!("rtf lex: {e:?}"))?;
    let doc = Parser::new(tokens)
        .parse()
        .map_err(|e| format!("rtf parse: {e:?}"))?;
    let markdown = crate::text::tidy(&doc.get_text());

    let images = picts
        .into_iter()
        .enumerate()
        .map(|(i, (ext, data))| Image {
            name: format!("image{}.{}", i + 1, ext),
            data,
        })
        .collect();

    Ok(Converted { markdown, images })
}

fn hex_decode(hex: &str) -> Vec<u8> {
    let nibbles: Vec<u8> = hex
        .bytes()
        .filter_map(|c| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        })
        .collect();
    nibbles
        .chunks_exact(2)
        .map(|p| (p[0] << 4) | p[1])
        .collect()
}

// Carve embedded pictures out of `{\pict ...}` groups and return the RTF with those
// groups removed. Picture data is hex-encoded by default; the blip control word gives
// the image type.
fn strip_pict_groups(rtf: &str, extract_images: bool) -> (String, Vec<(String, Vec<u8>)>) {
    let bytes = rtf.as_bytes();
    let mut images = Vec::new();
    let mut cleaned = String::with_capacity(rtf.len());
    let mut i = 0usize;
    while i < rtf.len() {
        if let Some(rel) = rtf[i..].find("{\\pict") {
            let start = i + rel;
            cleaned.push_str(&rtf[i..start]);
            let mut depth = 0i32;
            let mut end = rtf.len();
            for (k, &b) in bytes[start..].iter().enumerate() {
                match b {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = start + k + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if extract_images {
                if let Some(img) = parse_pict(&rtf[start..end.min(rtf.len())]) {
                    images.push(img);
                }
            }
            i = end;
        } else {
            cleaned.push_str(&rtf[i..]);
            break;
        }
    }
    (cleaned, images)
}

// rtf-parser drops `\par` (no newline); only an escaped newline becomes a line break.
// Rewrite each `\par` control word to an escaped newline so paragraphs are preserved.
fn restore_par_breaks(rtf: &str) -> String {
    let b = rtf.as_bytes();
    let mut out = String::with_capacity(rtf.len() + 16);
    let mut i = 0usize;
    while i < b.len() {
        if rtf[i..].starts_with("\\par")
            && !b
                .get(i + 4)
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or(false)
        {
            out.push('\\');
            out.push('\n');
            i += 4;
            continue;
        }
        let Some(ch) = rtf[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn parse_pict(group: &str) -> Option<(String, Vec<u8>)> {
    let mut ext: Option<&str> = None;
    let mut hex = String::new();
    let chars: Vec<char> = group.chars().collect();
    let mut j = 0usize;
    while j < chars.len() {
        let c = chars[j];
        if c == '\\' {
            j += 1;
            let mut word = String::new();
            while j < chars.len() && chars[j].is_ascii_alphabetic() {
                word.push(chars[j]);
                j += 1;
            }
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '-') {
                j += 1;
            }
            if j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            match word.as_str() {
                "pngblip" => ext = Some("png"),
                "jpegblip" => ext = Some("jpg"),
                "dibitmap" | "wbitmap" => ext = Some("bmp"),
                "emfblip" => ext = Some("emf"),
                w if w.starts_with("wmetafile") => ext = Some("wmf"),
                _ => {}
            }
        } else if c == '{' || c == '}' {
            j += 1;
        } else if c.is_ascii_hexdigit() {
            hex.push(c);
            j += 1;
        } else {
            j += 1;
        }
    }
    let data = hex_decode(&hex);
    if data.is_empty() {
        return None;
    }
    Some((ext.unwrap_or("bin").to_string(), data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_decode_pairs_nibbles() {
        assert_eq!(hex_decode("48656c6c6f"), b"Hello");
        assert_eq!(hex_decode("4 8\n65"), b"He"); // whitespace ignored
    }

    #[test]
    fn restore_par_breaks_only_targets_par() {
        assert_eq!(restore_par_breaks(r"x\par y"), "x\\\n y");
        assert_eq!(restore_par_breaks(r"x\pard y"), r"x\pard y");
        assert_eq!(restore_par_breaks(r"x\pardeftab y"), r"x\pardeftab y");
    }

    #[test]
    fn parse_pict_decodes_png_group() {
        let (ext, data) = parse_pict(r"{\pict\pngblip\picw1 48656c6c6f}").unwrap();
        assert_eq!(ext, "png");
        assert_eq!(data, b"Hello");
    }

    #[test]
    fn strip_pict_groups_removes_data_and_extracts_image() {
        let rtf = r"{\rtf1 text {\pict\jpegblip 48656c6c6f} more}";
        let (cleaned, imgs) = strip_pict_groups(rtf, true);
        assert!(!cleaned.contains("48656c6c6f"));
        assert!(cleaned.contains("text"));
        assert!(cleaned.contains("more"));
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].0, "jpg");
        assert_eq!(imgs[0].1, b"Hello");
    }

    #[test]
    fn hex_decode_handles_uppercase_and_byte_values() {
        assert_eq!(hex_decode("FF00A5"), vec![0xFF, 0x00, 0xA5]);
    }

    #[test]
    fn parse_pict_recognizes_every_blip_type() {
        assert_eq!(parse_pict(r"{\pict\jpegblip 00}").unwrap().0, "jpg");
        assert_eq!(parse_pict(r"{\pict\dibitmap 00}").unwrap().0, "bmp");
        assert_eq!(parse_pict(r"{\pict\emfblip 00}").unwrap().0, "emf");
        assert_eq!(parse_pict(r"{\pict\wmetafile8 00}").unwrap().0, "wmf");
        assert_eq!(parse_pict(r"{\pict 00}").unwrap().0, "bin");
    }

    #[test]
    fn strip_pict_groups_preserves_text_and_handles_multiple() {
        let (cleaned, imgs) = strip_pict_groups(r"A{\pict\pngblip 01}B{\pict\jpegblip 02}C", true);
        assert_eq!(cleaned, "ABC");
        assert_eq!(imgs.len(), 2);
        assert_eq!(imgs[0].0, "png");
        assert_eq!(imgs[1].0, "jpg");
        assert_eq!(imgs[0].1, vec![0x01]);
        assert_eq!(imgs[1].1, vec![0x02]);
    }

    #[test]
    fn convert_extracts_text_paragraphs_and_image() {
        let rtf = br"{\rtf1\ansi Hello from RTF legacy\par Second line of rtf\par {\pict\pngblip 89504e470d0a1a0a}}";
        let c = convert(rtf, true).unwrap();
        assert!(c.markdown.contains("Hello from RTF legacy"));
        assert!(c.markdown.contains("Second line of rtf"));
        assert!(!c.markdown.contains("89504e47")); // hex must not leak
        assert_eq!(c.images.len(), 1);
        assert_eq!(c.images[0].name, "image1.png");
    }

    #[test]
    fn convert_skips_image_decode_when_disabled() {
        let rtf = br"{\rtf1\ansi Hello\par {\pict\pngblip 89504e470d0a1a0a}}";
        let c = convert(rtf, false).unwrap();
        assert!(c.markdown.contains("Hello"));
        assert!(!c.markdown.contains("89504e47"));
        assert!(c.images.is_empty());
    }
}
