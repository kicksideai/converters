use crate::types::Image;
use std::io::Read;

const WMF_PLACEABLE: &[u8] = &[0xD7, 0xCD, 0xC6, 0x9A];

// Embedded vector metafiles are small. This caps both the inflated size and the
// bytes handed to the third-party EMF/WMF players, so a crafted image cannot
// exhaust memory — this path runs for every extracted image of every format and
// is outside the archive decompression budget.
const MAX_METAFILE_BYTES: usize = 16 * 1024 * 1024;

// EMR_HEADER: record type 1 (u32 LE) followed, at offset 40, by the " EMF"
// signature (0x464D4520, little-endian on disk as 0x20 0x45 0x4D 0x46).
fn is_emf(d: &[u8]) -> bool {
    d.len() >= 44 && d[0..4] == [0x01, 0x00, 0x00, 0x00] && d[40..44] == [0x20, 0x45, 0x4D, 0x46]
}

// Placeable WMF carries the Aldus magic; a standard METAHEADER starts with the
// metafile type (1 = memory, 2 = disk) followed by a header size of 9 words.
fn is_wmf(d: &[u8]) -> bool {
    d.starts_with(WMF_PLACEABLE)
        || (d.len() >= 4
            && (d[0] == 0x01 || d[0] == 0x02)
            && d[1] == 0x00
            && d[2..4] == [0x09, 0x00])
}

fn raw_to_svg(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() > MAX_METAFILE_BYTES {
        return None;
    }
    if is_emf(data) {
        emf_core::converter::EMFConverter::new(
            data,
            emf_core::converter::SVGPlayer::new(),
            wmf_core::converter::SVGPlayer::new(),
        )
        .run()
        .ok()
    } else if is_wmf(data) {
        wmf_core::converter::WMFConverter::new(data, wmf_core::converter::SVGPlayer::new())
            .run()
            .ok()
    } else {
        None
    }
}

// A zlib stream: deflate compression method (CMF low nibble 8) with a CMF/FLG
// header that satisfies the mod-31 checksum.
fn is_zlib(d: &[u8]) -> bool {
    d.len() >= 2 && d[0] & 0x0F == 8 && (u16::from(d[0]) << 8 | u16::from(d[1])) % 31 == 0
}

// Office Art stores embedded metafiles zlib-deflated; calamine and other readers
// hand them back still compressed. Inflation is bounded so a small compressed blob
// cannot expand without limit.
fn maybe_inflate(data: &[u8]) -> Option<Vec<u8>> {
    if !is_zlib(data) {
        return None;
    }
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(MAX_METAFILE_BYTES as u64 + 1)
        .read_to_end(&mut out)
        .ok()?;
    if out.len() > MAX_METAFILE_BYTES {
        return None;
    }
    Some(out)
}

fn to_svg(data: &[u8]) -> Option<Vec<u8>> {
    if let Some(svg) = raw_to_svg(data) {
        return Some(svg);
    }
    let inflated = maybe_inflate(data)?;
    raw_to_svg(&inflated)
}

fn replace_ext(name: &str, ext: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.{ext}"),
        None => format!("{name}.{ext}"),
    }
}

// Convert any embedded EMF/WMF metafile image into viewable SVG, in place. Images
// that are not metafiles, or whose conversion fails, are left untouched so the
// raw bytes still reach the caller rather than being dropped.
pub fn convert_in_place(images: &mut [Image]) {
    for img in images.iter_mut() {
        if let Some(svg) = to_svg(&img.data) {
            img.data = svg;
            img.name = replace_ext(&img.name, "svg");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_emf() -> Vec<u8> {
        let mut emf = vec![0u8; 44];
        emf[0] = 0x01;
        emf[40..44].copy_from_slice(&[0x20, 0x45, 0x4D, 0x46]);
        emf
    }

    #[test]
    fn detects_emf_and_wmf_signatures() {
        assert!(is_emf(&make_emf()));
        assert!(is_wmf(WMF_PLACEABLE));
        assert!(is_wmf(&[0x01, 0x00, 0x09, 0x00, 0x00]));
        assert!(is_wmf(&[0x02, 0x00, 0x09, 0x00, 0x00]));
        assert!(!is_emf(b"not a metafile"));
        assert!(!is_wmf(b"PK\x03\x04"));
    }

    #[test]
    fn is_emf_requires_both_record_type_and_signature() {
        // Correct " EMF" signature but wrong leading record type.
        let mut bad_type = make_emf();
        bad_type[0] = 0x02;
        assert!(!is_emf(&bad_type));
        // Correct record type but wrong signature offset bytes.
        let mut bad_sig = make_emf();
        bad_sig[40] = 0x00;
        assert!(!is_emf(&bad_sig));
        // One byte too short.
        assert!(!is_emf(&make_emf()[..43]));
    }

    #[test]
    fn is_wmf_standard_header_needs_type_size_and_marker() {
        assert!(!is_wmf(&[0x03, 0x00, 0x09, 0x00])); // bad type
        assert!(!is_wmf(&[0x01, 0x01, 0x09, 0x00])); // non-zero high byte of type
        assert!(!is_wmf(&[0x01, 0x00, 0x08, 0x00])); // wrong header size
        assert!(!is_wmf(&[0x01, 0x00])); // too short
    }

    #[test]
    fn maybe_inflate_only_handles_zlib_magic() {
        use std::io::Write;
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(b"payload").unwrap();
        let zlib = enc.finish().unwrap();
        assert_eq!(maybe_inflate(&zlib).as_deref(), Some(b"payload".as_ref()));
        assert!(maybe_inflate(&[0x78]).is_none()); // too short
        assert!(maybe_inflate(&[0x79, 0xDA, 0x00]).is_none()); // not deflate method
        assert!(maybe_inflate(&[0x78, 0x00, 0x00]).is_none()); // fails mod-31 checksum
    }

    #[test]
    fn maybe_inflate_rejects_a_decompression_bomb() {
        use std::io::Write;
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        enc.write_all(&vec![0u8; MAX_METAFILE_BYTES + 1024])
            .unwrap();
        let bomb = enc.finish().unwrap();
        assert!(bomb.len() < 100_000, "zeros must compress tiny");
        assert!(
            maybe_inflate(&bomb).is_none(),
            "inflation past the cap must be rejected, not expanded"
        );
    }

    #[test]
    fn converts_zlib_compressed_emf_to_svg() {
        let mut imgs = vec![Image {
            name: "image1.emf".into(),
            data: include_bytes!("../tests/assets/metafile.emf").to_vec(),
        }];
        convert_in_place(&mut imgs);
        assert_eq!(imgs[0].name, "image1.svg");
        assert!(
            imgs[0].data.starts_with(b"<svg"),
            "expected SVG, got {:?}",
            String::from_utf8_lossy(&imgs[0].data[..imgs[0].data.len().min(16)])
        );
    }

    #[test]
    fn non_metafile_images_are_left_untouched() {
        let mut imgs = vec![Image {
            name: "photo.jpg".into(),
            data: vec![0xFF, 0xD8, 0xFF, 0xE0],
        }];
        convert_in_place(&mut imgs);
        assert_eq!(imgs[0].name, "photo.jpg");
        assert_eq!(imgs[0].data, vec![0xFF, 0xD8, 0xFF, 0xE0]);
    }
}
