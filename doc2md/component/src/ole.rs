use std::io::{Cursor, Read};

pub fn open(blob: &[u8]) -> Result<cfb::CompoundFile<Cursor<Vec<u8>>>, String> {
    cfb::CompoundFile::open(Cursor::new(blob.to_vec())).map_err(|e| e.to_string())
}

// Root-level stream names only. Format routing must ignore streams inside embedded
// OLE objects (e.g. an .xls containing an embedded Word object also has a nested
// WordDocument stream), which would otherwise misroute the document.
pub fn stream_names(comp: &mut cfb::CompoundFile<Cursor<Vec<u8>>>) -> Vec<String> {
    let root = std::path::Path::new("/");
    comp.walk()
        .filter(|e| e.is_stream())
        .filter(|e| e.path().parent() == Some(root))
        .map(|e| e.name().to_string())
        .collect()
}

pub fn read_named(
    comp: &mut cfb::CompoundFile<Cursor<Vec<u8>>>,
    candidates: &[&str],
) -> Option<Vec<u8>> {
    // Only consider streams in the root storage. Legacy docs can embed OLE objects
    // (a packaged Word doc, etc.) whose sub-storages contain identically-named streams;
    // reading those instead of the root document yields the wrong content.
    let root = std::path::Path::new("/");
    let paths: Vec<String> = comp
        .walk()
        .filter(|e| e.is_stream())
        .filter(|e| e.path().parent() == Some(root))
        .filter(|e| candidates.iter().any(|c| e.name().eq_ignore_ascii_case(c)))
        .map(|e| e.path().to_string_lossy().to_string())
        .collect();
    for path in paths {
        if let Ok(mut s) = comp.open_stream(&path) {
            let mut buf = Vec::new();
            if s.read_to_end(&mut buf).is_ok() {
                return Some(buf);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_rejects_non_ole2() {
        assert!(open(b"not an ole2 file").is_err());
    }

    #[test]
    fn reads_root_streams_of_real_doc() {
        let bytes = include_bytes!("../tests/assets/doc.doc").to_vec();
        let mut comp = open(&bytes).unwrap();
        let names = stream_names(&mut comp);
        assert!(names.iter().any(|n| n.eq_ignore_ascii_case("WordDocument")));
        assert!(read_named(&mut comp, &["WordDocument"]).is_some());
        assert!(read_named(&mut comp, &["wOrDdOcUmEnT"]).is_some()); // case-insensitive
        assert!(read_named(&mut comp, &["NoSuchStream"]).is_none());
    }
}
