use crate::escher;
use crate::ole;
use crate::sheet;
use crate::types::{Converted, Image};
use calamine::{Reader, Xls};
use std::io::Cursor;

pub fn convert(blob: &[u8], extract_images: bool) -> Result<Converted, String> {
    let mut wb = Xls::new(Cursor::new(blob.to_vec())).map_err(|e| e.to_string())?;
    let markdown = sheet::workbook_markdown(&mut wb)?;

    let images = if extract_images {
        collect_images(&mut wb, blob)
            .into_iter()
            .enumerate()
            .map(|(i, (ext, data))| Image {
                name: format!("image{}.{}", i + 1, ext.trim_start_matches('.')),
                data,
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(Converted { markdown, images })
}

fn collect_images(wb: &mut Xls<Cursor<Vec<u8>>>, blob: &[u8]) -> Vec<(String, Vec<u8>)> {
    if let Some(pics) = wb.pictures() {
        if !pics.is_empty() {
            return pics;
        }
    }
    // Fallback: carve Office Art BLIPs out of the Workbook stream.
    if let Ok(mut comp) = ole::open(blob) {
        if let Some(stream) = ole::read_named(&mut comp, &["Workbook", "Book"]) {
            return escher::extract_blips(&stream)
                .into_iter()
                .map(|b| (b.ext.to_string(), b.data))
                .collect();
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_real_xls_fixture_renders_table() {
        let bytes = include_bytes!("../tests/assets/xls.xls");
        let c = convert(bytes, true).unwrap();
        assert!(c.markdown.contains("## Sheet1"), "got: {}", c.markdown);
        assert!(c.markdown.contains("Hello from XLS"));
        assert!(c.markdown.contains("| --- |"));
        assert!(c.markdown.contains("42"));
    }

    #[test]
    fn convert_real_xls_fixture_skips_images_when_disabled() {
        let bytes = include_bytes!("../tests/assets/xls.xls");
        let c = convert(bytes, false).unwrap();
        assert!(c.markdown.contains("Hello from XLS"));
        assert!(c.images.is_empty());
    }
}
