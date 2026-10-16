use crate::escher;
use crate::ole;
use crate::text::{cp1252, push_char, tidy};
use crate::types::{Converted, Image};

const TEXT_CHARS_ATOM: u16 = 0x0FA0;
const TEXT_BYTES_ATOM: u16 = 0x0FA8;

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

// Walk PowerPoint Document records, collecting slide text from TextBytesAtom (CP1252)
// and TextCharsAtom (UTF-16LE). Containers (recVer 0x0F) recurse.
fn walk(buf: &[u8], depth: u32, out: &mut String) {
    if depth > 24 {
        return;
    }
    let mut pos = 0usize;
    while pos + 8 <= buf.len() {
        let ver_inst = u16le(buf, pos);
        let rec_type = u16le(buf, pos + 2);
        let rec_len = u32le(buf, pos + 4) as usize;
        let rec_ver = ver_inst & 0x0F;
        let body = pos + 8;
        let end = body.saturating_add(rec_len);
        if end > buf.len() {
            break;
        }
        let data = &buf[body..end];
        if rec_ver == 0x0F {
            walk(data, depth + 1, out);
        } else if rec_type == TEXT_BYTES_ATOM {
            for &b in data {
                push_char(out, cp1252(b));
            }
            out.push('\n');
        } else if rec_type == TEXT_CHARS_ATOM {
            for o in (0..data.len().saturating_sub(1)).step_by(2) {
                if let Some(c) = char::from_u32(u16le(data, o) as u32) {
                    push_char(out, c);
                }
            }
            out.push('\n');
        }
        pos = end;
    }
}

pub fn convert(blob: &[u8], extract_images: bool) -> Result<Converted, String> {
    let mut comp = ole::open(blob)?;
    let stream =
        ole::read_named(&mut comp, &["PowerPoint Document"]).ok_or("no PowerPoint Document")?;

    let mut text = String::new();
    walk(&stream, 0, &mut text);
    let markdown = tidy(&text);

    let images = if extract_images {
        match ole::read_named(&mut comp, &["Pictures"]) {
            Some(pics) => escher::extract_blips(&pics)
                .into_iter()
                .enumerate()
                .map(|(i, b)| Image {
                    name: format!("image{}.{}", i + 1, b.ext),
                    data: b.data,
                })
                .collect(),
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };

    Ok(Converted { markdown, images })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(rec_ver_inst: u16, rec_type: u16, data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&rec_ver_inst.to_le_bytes());
        v.extend_from_slice(&rec_type.to_le_bytes());
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn walk_collects_byte_and_char_text_atoms_in_containers() {
        let mut wide = Vec::new();
        for c in "Wide".chars() {
            wide.extend_from_slice(&(c as u16).to_le_bytes());
        }
        let mut inner = atom(0, TEXT_BYTES_ATOM, b"Bytes");
        inner.extend(atom(0, TEXT_CHARS_ATOM, &wide));
        inner.extend(atom(0, 0x0FBA, b"ignoredcstring")); // not a kept text atom
        let container = atom(0x000F, 0x0FF0, &inner); // recVer 0x0F => recurse

        let mut out = String::new();
        walk(&container, 0, &mut out);
        assert!(out.contains("Bytes"));
        assert!(out.contains("Wide"));
        assert!(!out.contains("ignoredcstring"));
    }

    #[test]
    fn walk_stops_on_truncated_record() {
        let mut rec = Vec::new();
        rec.extend_from_slice(&0u16.to_le_bytes());
        rec.extend_from_slice(&TEXT_BYTES_ATOM.to_le_bytes());
        rec.extend_from_slice(&999u32.to_le_bytes()); // length exceeds buffer
        rec.extend_from_slice(b"short");
        let mut out = String::new();
        walk(&rec, 0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn convert_real_ppt_fixture_extracts_text_and_image() {
        let bytes = include_bytes!("../tests/assets/ppt.ppt");
        let c = convert(bytes, true).unwrap();
        assert!(
            c.markdown.contains("Hello from PPT legacy"),
            "got: {}",
            c.markdown
        );
        assert_eq!(c.images.len(), 1);
        assert!(c.images[0].name.ends_with(".png"));
    }

    #[test]
    fn convert_real_ppt_fixture_skips_images_when_disabled() {
        let bytes = include_bytes!("../tests/assets/ppt.ppt");
        let c = convert(bytes, false).unwrap();
        assert!(c.markdown.contains("Hello from PPT legacy"));
        assert!(c.images.is_empty());
    }
}
