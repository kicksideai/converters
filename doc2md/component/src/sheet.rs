use calamine::{Data, Range, Reader};
use std::io::Cursor;

fn cell(d: &Data) -> String {
    let s = match d {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            if f.fract() == 0.0 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => dt.to_string(),
        Data::Error(e) => format!("{e:?}"),
        other => other.to_string(),
    };
    s.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', " ")
        .trim()
        .to_string()
}

pub fn range_to_markdown(name: &str, range: &Range<Data>) -> String {
    if range.is_empty() {
        return String::new();
    }
    let mut markdown = String::new();
    markdown.push_str(&format!("## {name}\n\n"));
    let (row_start, column_start) = range.start().unwrap_or((0, 0));
    markdown.push_str(&format!(
        "<!-- kickside:sheet header-row={} column-start={} -->\n",
        row_start + 1,
        column_start + 1
    ));
    let width = range.width();
    for (idx, row) in range.rows().enumerate() {
        let cells: Vec<String> = (0..width)
            .map(|c| row.get(c).map(cell).unwrap_or_default())
            .collect();
        markdown.push_str("| ");
        markdown.push_str(&cells.join(" | "));
        markdown.push_str(" |\n");
        if idx == 0 {
            markdown.push('|');
            for _ in &cells {
                markdown.push_str(" --- |");
            }
            markdown.push('\n');
        }
    }
    markdown.push('\n');
    markdown
}

// All workbook formats emit the same table/origin contract. Read errors are
// conversion errors, so a damaged sheet cannot silently disappear from the KB.
pub fn workbook_markdown<R: Reader<Cursor<Vec<u8>>>>(wb: &mut R) -> Result<String, String> {
    let mut markdown = String::new();
    for name in wb.sheet_names().to_vec() {
        let range = wb
            .worksheet_range(&name)
            .map_err(|e| format!("sheet {name}: {e:?}"))?;
        markdown.push_str(&range_to_markdown(&name, &range));
    }
    Ok(markdown)
}

pub fn xlsx_markdown(blob: &[u8]) -> Result<String, String> {
    let mut wb = calamine::Xlsx::new(Cursor::new(blob.to_vec())).map_err(|e| e.to_string())?;
    workbook_markdown(&mut wb)
}

pub fn ods_markdown(blob: &[u8]) -> Result<String, String> {
    let mut wb = calamine::Ods::new(Cursor::new(blob.to_vec())).map_err(|e| e.to_string())?;
    workbook_markdown(&mut wb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_physical_header_row_for_ingestion() {
        let range = Range::from_sparse(vec![
            calamine::Cell::new((3, 2), Data::String("SKU".into())),
            calamine::Cell::new((4, 2), Data::String("A-1".into())),
        ]);
        assert!(range_to_markdown("Products", &range)
            .contains("<!-- kickside:sheet header-row=4 column-start=3 -->"));
    }

    #[test]
    fn cell_preserves_a_backslash_before_a_pipe() {
        assert_eq!(cell(&Data::String(r"path\|leaf".into())), r"path\\\|leaf");
    }

    #[test]
    fn cell_formats_each_data_kind() {
        assert_eq!(cell(&Data::Empty), "");
        assert_eq!(cell(&Data::Int(42)), "42");
        assert_eq!(cell(&Data::Float(4.0)), "4");
        assert_eq!(cell(&Data::Float(3.5)), "3.5");
        assert_eq!(cell(&Data::Bool(true)), "true");
        assert_eq!(cell(&Data::String("a|b\nc".into())), "a\\|b c");
    }

    #[test]
    fn range_to_markdown_renders_header_separator_and_rows() {
        let range = Range::from_sparse(vec![
            calamine::Cell::new((0, 0), Data::String("Name".into())),
            calamine::Cell::new((0, 1), Data::String("Qty".into())),
            calamine::Cell::new((1, 0), Data::String("Widget".into())),
            calamine::Cell::new((1, 1), Data::Int(42)),
        ]);
        let md = range_to_markdown("Sheet1", &range);
        assert!(md.contains("## Sheet1"));
        assert!(md.contains("| Name | Qty |"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("| Widget | 42 |"));
    }

    #[test]
    fn range_to_markdown_empty_yields_nothing() {
        let range: Range<Data> = Range::empty();
        assert_eq!(range_to_markdown("Sheet1", &range), "");
    }

    #[test]
    fn range_to_markdown_preserves_rectangular_rows_and_separates_only_header() {
        let range = Range::from_sparse(vec![
            calamine::Cell::new((0, 0), Data::String("H1".into())),
            calamine::Cell::new((0, 1), Data::String("H2".into())),
            calamine::Cell::new((1, 0), Data::String("only".into())),
            calamine::Cell::new((2, 0), Data::String("r2".into())),
        ]);
        let md = range_to_markdown("S", &range);
        assert_eq!(
            md,
            "## S\n\n<!-- kickside:sheet header-row=1 column-start=1 -->\n| H1 | H2 |\n| --- | --- |\n| only |  |\n| r2 |  |\n\n"
        );
    }
}
