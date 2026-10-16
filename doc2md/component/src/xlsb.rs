use crate::sheet;
use crate::types::{Converted, Image};
use calamine::{Reader, Xlsb};
use std::io::Cursor;

pub fn convert(blob: &[u8], extract_images: bool) -> Result<Converted, String> {
    let mut wb = Xlsb::new(Cursor::new(blob.to_vec())).map_err(|e| e.to_string())?;
    let markdown = sheet::workbook_markdown(&mut wb)?;

    let images = if extract_images {
        wb.pictures()
            .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_real_xlsb_fixture_renders_table() {
        let bytes = include_bytes!("../tests/assets/xlsb.xlsb");
        let c = convert(bytes, false).unwrap();
        assert!(c.markdown.contains("## "), "got: {}", c.markdown);
        assert!(c.markdown.contains("| --- |"));
    }
}
