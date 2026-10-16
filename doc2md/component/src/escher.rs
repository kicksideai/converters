// Office Art (Escher / MS-ODRAW) BLIP image extraction. The legacy binary Office
// formats (.doc, .xls, .ppt) embed raster images as OfficeArtBlip records. Rather
// than rely on a clean container layout (Word's Data stream interleaves PICF headers),
// this scans for valid BLIP record headers anywhere in the stream: an 8-byte header
// whose recVer/recInstance/recType form a documented BLIP signature and whose length
// fits the buffer. The recInstance whitelist per type makes false positives negligible.

pub struct Blip {
    pub ext: &'static str,
    pub data: Vec<u8>,
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

// (extension, valid recInstance values) per BLIP recType. Metafiles (EMF/WMF/PICT)
// are intentionally excluded: their payload is compressed and not a viewable raster.
fn blip_spec(rec_type: u16, rec_instance: u16) -> Option<&'static str> {
    let (ext, instances): (&str, &[u16]) = match rec_type {
        0xF01D => ("jpg", &[0x46A, 0x46B, 0x6E2, 0x6E3]),
        0xF01E => ("png", &[0x6E0, 0x6E1]),
        0xF01F => ("bmp", &[0x7A8, 0x7A9]),
        0xF029 => ("tiff", &[0x6E4, 0x6E5]),
        0xF02A => ("jpg", &[0x46A, 0x46B, 0x6E2, 0x6E3]),
        _ => return None,
    };
    instances.contains(&rec_instance).then_some(ext)
}

// (extension, valid recInstance values) per metafile BLIP recType.
fn metafile_spec(rec_type: u16, rec_instance: u16) -> Option<&'static str> {
    let (ext, instances): (&str, &[u16]) = match rec_type {
        0xF01A => ("emf", &[0x3D4, 0x3D5]),
        0xF01B => ("wmf", &[0x216, 0x217]),
        _ => return None,
    };
    instances.contains(&rec_instance).then_some(ext)
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

// OfficeArtMetafileHeader is 34 bytes: cbSize(4) rcBounds(16) ptSize(8) cbSave(4)
// compression(1) filter(1). compression 0x00 means the metafile is zlib-deflated.
fn extract_metafile(rec_instance: u16, ext: &'static str, data: &[u8]) -> Option<Blip> {
    let uids = 1 + (rec_instance & 1) as usize;
    let hdr = uids * 16;
    if data.len() < hdr + 34 {
        return None;
    }
    let compression = data[hdr + 32];
    let payload = &data[hdr + 34..];
    let bytes = if compression == 0 {
        inflate(payload)?
    } else {
        payload.to_vec()
    };
    if bytes.is_empty() {
        return None;
    }
    Some(Blip { ext, data: bytes })
}

fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 4 {
        return None;
    }
    let header_size = u32le(dib, 0) as usize;
    let bits_offset = 14 + header_size;
    let file_size = 14 + dib.len();
    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(bits_offset as u32).to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

pub fn extract_blips(buf: &[u8]) -> Vec<Blip> {
    let mut out = Vec::new();
    if buf.len() < 8 {
        return out;
    }
    let mut i = 0usize;
    while i + 8 <= buf.len() {
        let ver_inst = u16le(buf, i);
        let rec_type = u16le(buf, i + 2);
        let rec_len = u32le(buf, i + 4) as usize;
        let rec_ver = ver_inst & 0x0F;
        let rec_instance = (ver_inst >> 4) & 0x0FFF;
        let body = i + 8;
        // checked_add: on wasm32 usize is 32-bit, so a garbage rec_len could wrap
        // `body + rec_len` past the length guard and produce an inverted slice range.
        let end = body.checked_add(rec_len).filter(|&e| e <= buf.len());

        if let (0, true, Some(end)) = (rec_ver, rec_len >= 17, end) {
            let data = &buf[body..end];
            if let Some(ext) = blip_spec(rec_type, rec_instance) {
                let uids = 1 + (rec_instance & 1) as usize;
                let skip = uids * 16 + 1;
                if data.len() > skip {
                    let payload = &data[skip..];
                    let bytes = if rec_type == 0xF01F {
                        dib_to_bmp(payload)
                    } else {
                        Some(payload.to_vec())
                    };
                    if let Some(bytes) = bytes {
                        out.push(Blip { ext, data: bytes });
                        i = end;
                        continue;
                    }
                }
            } else if let Some(ext) = metafile_spec(rec_type, rec_instance) {
                if let Some(blip) = extract_metafile(rec_instance, ext, data) {
                    out.push(blip);
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_blip_record(payload: &[u8]) -> Vec<u8> {
        let rec_len = 16 + 1 + payload.len(); // 1 uid + tag + payload
        let mut v = Vec::new();
        v.extend_from_slice(&0x6E00u16.to_le_bytes()); // recInstance 0x6E0, recVer 0
        v.extend_from_slice(&0xF01Eu16.to_le_bytes()); // PNG
        v.extend_from_slice(&(rec_len as u32).to_le_bytes());
        v.extend_from_slice(&[0u8; 16]); // rgbUid1
        v.push(0); // tag
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn blip_spec_requires_known_type_and_instance() {
        assert_eq!(blip_spec(0xF01E, 0x6E0), Some("png"));
        assert_eq!(blip_spec(0xF01D, 0x46A), Some("jpg"));
        assert_eq!(blip_spec(0xF01E, 0x999), None); // wrong instance
        assert_eq!(blip_spec(0x1234, 0x6E0), None); // wrong type
    }

    #[test]
    fn dib_to_bmp_prepends_file_header() {
        let dib = [40u8, 0, 0, 0, 1, 2, 3, 4]; // BITMAPINFOHEADER size 40 + pixels
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(bmp.len(), 14 + dib.len());
        assert_eq!(
            u32::from_le_bytes([bmp[10], bmp[11], bmp[12], bmp[13]]),
            14 + 40
        );
    }

    #[test]
    fn extract_blips_finds_png_amid_noise() {
        let mut buf = vec![0xAB; 5];
        buf.extend_from_slice(&png_blip_record(b"PNGDATA"));
        buf.extend_from_slice(&[0u8; 4]);
        let blips = extract_blips(&buf);
        assert_eq!(blips.len(), 1);
        assert_eq!(blips[0].ext, "png");
        assert_eq!(blips[0].data, b"PNGDATA");
    }

    #[test]
    fn extract_blips_ignores_overflowing_length() {
        // recType PNG, recInstance valid, but rec_len = u32::MAX must not wrap/panic.
        let mut buf = Vec::new();
        buf.extend_from_slice(&0x6E00u16.to_le_bytes());
        buf.extend_from_slice(&0xF01Eu16.to_le_bytes());
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        buf.extend_from_slice(&[0u8; 32]);
        assert!(extract_blips(&buf).is_empty());
    }

    #[test]
    fn blip_spec_covers_all_raster_types() {
        assert_eq!(blip_spec(0xF01F, 0x7A8), Some("bmp"));
        assert_eq!(blip_spec(0xF029, 0x6E4), Some("tiff"));
        assert_eq!(blip_spec(0xF02A, 0x46A), Some("jpg"));
        assert_eq!(blip_spec(0xF01F, 0x6E4), None); // bmp type, wrong instance
    }

    #[test]
    fn metafile_spec_covers_emf_and_wmf() {
        assert_eq!(metafile_spec(0xF01A, 0x3D4), Some("emf"));
        assert_eq!(metafile_spec(0xF01B, 0x216), Some("wmf"));
        assert_eq!(metafile_spec(0xF01A, 0x216), None); // emf type, wmf instance
        assert_eq!(metafile_spec(0x1234, 0x3D4), None);
    }

    #[test]
    fn dib_to_bmp_rejects_too_short_and_sets_offset() {
        assert!(dib_to_bmp(&[1, 2, 3]).is_none());
        let dib = [12u8, 0, 0, 0, 9, 9]; // header size 12, 6 bytes total
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(
            u32::from_le_bytes([bmp[10], bmp[11], bmp[12], bmp[13]]),
            14 + 12
        );
        // bytes 2..6 hold the file size = 14 + dib.len()
        assert_eq!(
            u32::from_le_bytes([bmp[2], bmp[3], bmp[4], bmp[5]]),
            (14 + dib.len()) as u32
        );
    }

    #[test]
    fn extract_metafile_passthrough_uncompressed_and_two_uid() {
        // compression != 0 => payload passed through verbatim.
        let mut data = vec![0u8; 16];
        let mut header = vec![0u8; 34];
        header[32] = 0xFE; // not deflate
        data.extend_from_slice(&header);
        data.extend_from_slice(b"RAWMETA");
        let blip = extract_metafile(0x3D4, "wmf", &data).unwrap();
        assert_eq!(blip.data, b"RAWMETA");

        // instance 0x3D5 is odd => 2 UIDs => header starts at byte 32.
        let mut data2 = vec![0u8; 32];
        let mut header2 = vec![0u8; 34];
        header2[32] = 0xFE;
        data2.extend_from_slice(&header2);
        data2.extend_from_slice(b"TWOUID");
        assert_eq!(
            extract_metafile(0x3D5, "emf", &data2).unwrap().data,
            b"TWOUID"
        );
        // with only 1 UID worth of prefix the header would be misread -> wrong/empty
        assert!(extract_metafile(0x3D5, "emf", &data).is_none());
    }

    #[test]
    fn extract_metafile_inflates_deflated_emf() {
        use std::io::Write;
        let raw = b"ENHMETAFILE-CONTENT-0123456789-abcdefghij";
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(raw).unwrap();
        let compressed = enc.finish().unwrap();
        let mut data = vec![0u8; 16]; // 1 UID (instance 0x3D4 is even)
        let mut header = vec![0u8; 34]; // OfficeArtMetafileHeader
        header[32] = 0; // compression = deflate
        data.extend_from_slice(&header);
        data.extend_from_slice(&compressed);
        let blip = extract_metafile(0x3D4, "emf", &data).unwrap();
        assert_eq!(blip.ext, "emf");
        assert_eq!(blip.data, raw);
        // too small for header => None
        assert!(extract_metafile(0x3D4, "emf", &[0u8; 40]).is_none());
    }

    #[test]
    fn extract_blips_handles_two_uid_png() {
        // recInstance 0x6E1 => 2 UIDs => skip = 2*16 + 1 = 33.
        let payload = b"PNGPAYLOAD";
        let rec_len = 32 + 1 + payload.len();
        let mut rec = Vec::new();
        rec.extend_from_slice(&0x6E10u16.to_le_bytes()); // (0x6E1 << 4) | 0
        rec.extend_from_slice(&0xF01Eu16.to_le_bytes());
        rec.extend_from_slice(&(rec_len as u32).to_le_bytes());
        rec.extend_from_slice(&[0u8; 32]); // two UIDs
        rec.push(0); // tag
        rec.extend_from_slice(payload);
        let blips = extract_blips(&rec);
        assert_eq!(blips.len(), 1);
        assert_eq!(blips[0].data, payload);
    }

    #[test]
    fn extract_blips_extracts_metafile_record() {
        use std::io::Write;
        let raw = b"WMF-bytes-1234567890-abcdefghijklmnop";
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(raw).unwrap();
        let compressed = enc.finish().unwrap();
        let body_len = 16 + 34 + compressed.len();
        let mut rec = Vec::new();
        rec.extend_from_slice(&0x3D40u16.to_le_bytes()); // EMF instance 0x3D4, ver 0
        rec.extend_from_slice(&0xF01Au16.to_le_bytes());
        rec.extend_from_slice(&(body_len as u32).to_le_bytes());
        rec.extend_from_slice(&[0u8; 16]);
        let mut header = vec![0u8; 34];
        header[32] = 0;
        rec.extend_from_slice(&header);
        rec.extend_from_slice(&compressed);
        let blips = extract_blips(&rec);
        assert_eq!(blips.len(), 1);
        assert_eq!(blips[0].ext, "emf");
        assert_eq!(blips[0].data, raw);
    }
}
