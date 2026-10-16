use crate::escher;
use crate::ole;
use crate::text::{cp1252, push_char, tidy};
use crate::types::{Converted, Image};

// FIB field offsets in the WordDocument stream (Word 97-2003, nFib 0x00C1).
const OFF_FLAGS: usize = 0x000A;
const OFF_CCP_TEXT: usize = 0x004C;
const OFF_FC_CLX: usize = 0x01A2;
const OFF_LCB_CLX: usize = 0x01A6;
const FLAG_WHICH_TBL: u16 = 0x0200;

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

struct Piece {
    cp_start: u32,
    cp_end: u32,
    fc: u32,
    compressed: bool,
}

fn parse_piece_table(table: &[u8], fc_clx: usize, lcb_clx: usize) -> Result<Vec<Piece>, String> {
    // All offsets come from on-disk u32 fields; on wasm32 (32-bit usize) unchecked
    // additions can wrap and produce inverted slice ranges, so add defensively.
    let clx_end = fc_clx
        .checked_add(lcb_clx)
        .filter(|&e| e <= table.len())
        .ok_or("clx out of range")?;
    let clx = &table[fc_clx..clx_end];
    let mut i = 0usize;
    // Skip any leading Prc entries (clxt == 0x01).
    while i < clx.len() && clx[i] == 0x01 {
        if i + 3 > clx.len() {
            return Err("bad prc".into());
        }
        let cb = u16le(clx, i + 1) as usize;
        i = i.saturating_add(3).saturating_add(cb);
    }
    if i >= clx.len() || clx[i] != 0x02 {
        return Err("no pcdt".into());
    }
    if i + 5 > clx.len() {
        return Err("truncated pcdt".into());
    }
    let lcb_pcd = u32le(clx, i + 1) as usize;
    let plc_start = i + 5;
    let plc_end = plc_start
        .checked_add(lcb_pcd)
        .map_or(clx.len(), |e| e.min(clx.len()));
    let plc = &clx[plc_start..plc_end];
    // Bound the piece count by the bytes actually present, not the claimed length.
    let n = if plc.len() >= 4 {
        (plc.len() - 4) / 12
    } else {
        0
    };
    if n == 0 {
        return Err("empty piece table".into());
    }
    let cp_off = 0;
    let pcd_off = (n + 1) * 4;
    let mut pieces = Vec::with_capacity(n);
    for k in 0..n {
        let cp_start = u32le(plc, cp_off + k * 4);
        let cp_end = u32le(plc, cp_off + (k + 1) * 4);
        let pcd = &plc[pcd_off + k * 8..pcd_off + k * 8 + 8];
        let fc_compressed = u32le(pcd, 2);
        let compressed = fc_compressed & 0x4000_0000 != 0;
        let fc = fc_compressed & 0x3FFF_FFFF;
        pieces.push(Piece {
            cp_start,
            cp_end,
            fc,
            compressed,
        });
    }
    Ok(pieces)
}

fn extract_text(word: &[u8], pieces: &[Piece], ccp_text: u32) -> String {
    let mut out = String::new();
    // Field state: 0x13 begins a field instruction (skip), 0x14 separates the
    // displayable result (keep), 0x15 ends it.
    let mut in_field_instruction = false;
    let mut emit = |c: char, out: &mut String| match c {
        '\u{0013}' => in_field_instruction = true,
        '\u{0014}' => in_field_instruction = false,
        '\u{0015}' => in_field_instruction = false,
        _ if in_field_instruction => {}
        _ => push_char(out, c),
    };
    for p in pieces {
        if p.cp_start >= ccp_text {
            break;
        }
        let count = (p.cp_end.min(ccp_text) - p.cp_start) as usize;
        if p.compressed {
            let base = (p.fc / 2) as usize;
            for k in 0..count {
                if let Some(&b) = word.get(base + k) {
                    emit(cp1252(b), &mut out);
                }
            }
        } else {
            let base = p.fc as usize;
            for k in 0..count {
                let o = base + k * 2;
                if o + 1 < word.len() {
                    let u = u16le(word, o);
                    if let Some(c) = char::from_u32(u as u32) {
                        emit(c, &mut out);
                    }
                }
            }
        }
    }
    out
}

pub fn convert(blob: &[u8], extract_images: bool) -> Result<Converted, String> {
    let mut comp = ole::open(blob)?;
    let word = ole::read_named(&mut comp, &["WordDocument"]).ok_or("no WordDocument stream")?;
    if word.len() < OFF_LCB_CLX + 4 {
        return Err("WordDocument too small".into());
    }
    let flags = u16le(&word, OFF_FLAGS);
    let table_name = if flags & FLAG_WHICH_TBL != 0 {
        "1Table"
    } else {
        "0Table"
    };
    let ccp_text = u32le(&word, OFF_CCP_TEXT);
    let fc_clx = u32le(&word, OFF_FC_CLX) as usize;
    let lcb_clx = u32le(&word, OFF_LCB_CLX) as usize;

    let table = ole::read_named(&mut comp, &[table_name])
        .or_else(|| ole::read_named(&mut comp, &["1Table", "0Table"]))
        .ok_or("no table stream")?;

    let pieces = parse_piece_table(&table, fc_clx, lcb_clx)?;
    let markdown = tidy(&extract_text(&word, &pieces, ccp_text));

    let images = if extract_images {
        match ole::read_named(&mut comp, &["Data"]) {
            Some(data) => escher::extract_blips(&data)
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

    // CLX with a single compressed piece of `len` chars starting at byte `fc_bytes`.
    fn clx_one_compressed(fc_bytes: u32, len: u32) -> Vec<u8> {
        let mut plc = Vec::new();
        plc.extend_from_slice(&0u32.to_le_bytes()); // aCP[0]
        plc.extend_from_slice(&len.to_le_bytes()); // aCP[1]
        let fc_compressed = 0x4000_0000 | (fc_bytes * 2); // compressed flag + fc
        plc.extend_from_slice(&[0u8, 0u8]); // pcd prefix
        plc.extend_from_slice(&fc_compressed.to_le_bytes());
        plc.extend_from_slice(&[0u8, 0u8]); // pcd tail
        let mut clx = vec![0x02];
        clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
        clx.extend_from_slice(&plc);
        clx
    }

    #[test]
    fn parse_piece_table_reads_compressed_piece() {
        let clx = clx_one_compressed(0, 5);
        let pieces = parse_piece_table(&clx, 0, clx.len()).unwrap();
        assert_eq!(pieces.len(), 1);
        assert!(pieces[0].compressed);
        assert_eq!(pieces[0].fc, 0);
        assert_eq!(pieces[0].cp_end - pieces[0].cp_start, 5);
    }

    #[test]
    fn parse_piece_table_rejects_out_of_range_clx() {
        assert!(parse_piece_table(&[0u8; 4], usize::MAX, 8).is_err());
    }

    #[test]
    fn extract_text_decodes_and_strips_field_instruction() {
        let clx = clx_one_compressed(0, 5);
        let pieces = parse_piece_table(&clx, 0, clx.len()).unwrap();
        // word = "Hello" at byte 0
        assert_eq!(extract_text(b"Hello", &pieces, 5), "Hello");
    }

    #[test]
    fn extract_text_skips_hyperlink_field_codes() {
        // A\x13INSTR\x14disp\x15 -> "A" + (instruction dropped) + "disp"
        let word = b"A\x13B\x14C\x15";
        let clx = clx_one_compressed(0, word.len() as u32);
        let pieces = parse_piece_table(&clx, 0, clx.len()).unwrap();
        assert_eq!(extract_text(word, &pieces, word.len() as u32), "AC");
    }

    // Build a Pcdt CLX from (char_len, byte_offset, compressed) pieces.
    fn clx_pieces(pieces: &[(u32, u32, bool)]) -> Vec<u8> {
        let mut plc = Vec::new();
        let mut cp = 0u32;
        plc.extend_from_slice(&cp.to_le_bytes());
        for (len, _, _) in pieces {
            cp += len;
            plc.extend_from_slice(&cp.to_le_bytes());
        }
        for (_, byte_off, compressed) in pieces {
            let fc = if *compressed {
                0x4000_0000 | (byte_off * 2)
            } else {
                *byte_off
            };
            plc.extend_from_slice(&[0u8, 0u8]);
            plc.extend_from_slice(&fc.to_le_bytes());
            plc.extend_from_slice(&[0u8, 0u8]);
        }
        let mut clx = vec![0x02];
        clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
        clx.extend_from_slice(&plc);
        clx
    }

    #[test]
    fn extract_text_mixes_compressed_and_uncompressed_pieces() {
        let mut word = vec![b'X'; 64];
        word[4] = b'H'; // compressed piece base = fc/2 = 8/2 = 4
        word[5] = b'i';
        word[16] = b'Z'; // would be read if fc were doubled instead of halved
                         // uncompressed piece base = 10, UTF-16LE "Yo"
        word[10] = b'Y';
        word[11] = 0;
        word[12] = b'o';
        word[13] = 0;
        let clx = clx_pieces(&[(2, 4, true), (2, 10, false)]);
        let pieces = parse_piece_table(&clx, 0, clx.len()).unwrap();
        assert_eq!(pieces.len(), 2);
        assert!(pieces[0].compressed);
        assert!(!pieces[1].compressed);
        assert_eq!(extract_text(&word, &pieces, 4), "HiYo");
    }

    #[test]
    fn parse_piece_table_skips_prc_prefix() {
        let pcdt = clx_pieces(&[(3, 0, true)]);
        let mut clx = vec![0x01, 0x02, 0x00, 0xAA, 0xBB]; // Prc: clxt=1, cb=2, 2 bytes
        clx.extend_from_slice(&pcdt);
        let pieces = parse_piece_table(&clx, 0, clx.len()).unwrap();
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].cp_end - pieces[0].cp_start, 3);
    }

    #[test]
    fn parse_piece_table_requires_pcdt_marker() {
        // clxt byte 0x09 is neither Prc (0x01) nor Pcdt (0x02).
        let mut clx = vec![0x09u8];
        clx.extend_from_slice(&clx_pieces(&[(1, 0, true)])[..]);
        assert!(parse_piece_table(&clx, 0, clx.len()).is_err());
    }

    #[test]
    fn convert_real_doc_fixture_extracts_text_and_image() {
        let bytes = include_bytes!("../tests/assets/doc.doc");
        let c = convert(bytes, true).unwrap();
        assert!(
            c.markdown.contains("Hello from DOC legacy"),
            "got: {}",
            c.markdown
        );
        assert!(c.markdown.contains("Second paragraph here"));
        assert_eq!(c.images.len(), 1);
        assert!(c.images[0].name.ends_with(".png"));
        assert!(!c.images[0].data.is_empty());
    }

    #[test]
    fn convert_real_doc_fixture_skips_images_when_disabled() {
        let bytes = include_bytes!("../tests/assets/doc.doc");
        let c = convert(bytes, false).unwrap();
        assert!(c.markdown.contains("Hello from DOC legacy"));
        assert!(c.images.is_empty());
    }
}
