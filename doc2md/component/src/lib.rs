use std::collections::HashMap;

#[cfg(test)]
mod pdf_performance;

mod archive;
mod doc;
mod escher;
mod markdown;
mod metafile;
mod ole;
mod pdf_image;
mod ppt;
mod rtf;
mod sheet;
mod spss;
mod text;
mod types;
mod xls;
mod xlsb;

wit_bindgen::generate!({
    world: "converter",
});

struct Component;

const PDF_MAGIC: &[u8] = b"%PDF";
const ZIP_MAGIC: &[u8] = &[0x50, 0x4B, 0x03, 0x04];
const OLE2_MAGIC: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0];
const RTF_MAGIC: &[u8] = b"{\\rtf";
const SPSS_MAGIC: &[u8] = b"$FL";

const ERR_UNSUPPORTED: u8 = 1;
const ERR_PARSE: u8 = 2;
const ERR_RENDER: u8 = 3;

enum Format {
    Pdf,
    Office,
    Ole2,
    Rtf,
    Spss,
    Text,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
struct Settings {
    extract_images: bool,
    page_markers: bool,
}

impl From<ConversionOptions> for Settings {
    fn from(options: ConversionOptions) -> Self {
        Self {
            extract_images: options.extract_images,
            page_markers: options.page_markers,
        }
    }
}

fn fail(code: u8, message: impl Into<String>) -> ConversionError {
    ConversionError {
        code,
        message: message.into(),
    }
}

fn sniff(blob: &[u8]) -> Format {
    if blob.starts_with(PDF_MAGIC) {
        Format::Pdf
    } else if blob.starts_with(ZIP_MAGIC) {
        Format::Office
    } else if blob.starts_with(OLE2_MAGIC) {
        Format::Ole2
    } else if blob.starts_with(RTF_MAGIC) {
        Format::Rtf
    } else if blob.starts_with(SPSS_MAGIC) {
        Format::Spss
    } else if markdown::is_text(blob) {
        Format::Text
    } else {
        Format::Unknown
    }
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn base_id(reference: &str) -> &str {
    match reference.rsplit_once('.') {
        Some((base, _)) => base,
        None => reference,
    }
}

// Replace unpdf's anonymous `<!-- [alt] -->` block-image placeholders, in document
// order, with linked `![alt](name)` references to the extracted image files.
fn link_placeholders(markdown: &str, ordered: &[Option<String>]) -> String {
    let mut result = String::with_capacity(markdown.len());
    let mut rest = markdown;
    let mut index = 0;
    while let Some(open) = rest.find("<!-- [") {
        let after_open = open + "<!-- [".len();
        let Some(close_rel) = rest[after_open..].find("] -->") else {
            break;
        };
        let alt = &rest[after_open..after_open + close_rel];
        let end = after_open + close_rel + "] -->".len();
        result.push_str(&rest[..open]);
        match ordered.get(index) {
            Some(Some(name)) => result.push_str(&format!("![{}]({})", alt, name)),
            _ => result.push_str(&rest[open..end]),
        }
        rest = &rest[end..];
        index += 1;
    }
    result.push_str(rest);
    result
}

// Some PDFs carry trailing padding (e.g. NUL bytes) after the final `%%EOF`, which
// trips parsers that scan backwards for `startxref`. Everything after the last
// `%%EOF` is ignorable per the PDF spec, so trim it before parsing.
fn trim_pdf_trailer(blob: &[u8]) -> &[u8] {
    const EOF: &[u8] = b"%%EOF";
    match blob
        .windows(EOF.len())
        .rposition(|w| w == EOF)
        .map(|p| p + EOF.len())
    {
        Some(end) if end < blob.len() => &blob[..end],
        _ => blob,
    }
}

fn strip_placeholders(markdown: &str) -> String {
    let mut result = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(open) = rest.find("<!-- [") {
        let after_open = open + "<!-- [".len();
        let Some(close_rel) = rest[after_open..].find("] -->") else {
            break;
        };
        let end = after_open + close_rel + "] -->".len();
        result.push_str(&rest[..open]);
        rest = &rest[end..];
    }
    result.push_str(rest);
    result
}

fn convert_pdf(blob: &[u8], settings: Settings) -> Result<types::Converted, ConversionError> {
    let blob = trim_pdf_trailer(blob);
    let options = unpdf::ParseOptions::new().with_resources(settings.extract_images);
    let doc = unpdf::parse_bytes_with_options(blob, options)
        .map_err(|e| fail(ERR_PARSE, e.to_string()))?;
    let marker_style = if settings.page_markers {
        unpdf::PageMarkerStyle::Comment
    } else {
        unpdf::PageMarkerStyle::None
    };
    let render = unpdf::RenderOptions::default().with_page_markers(marker_style);
    let mut markdown =
        unpdf::render::to_markdown(&doc, &render).map_err(|e| fail(ERR_RENDER, e.to_string()))?;

    if !settings.extract_images {
        return Ok(types::Converted {
            markdown: strip_placeholders(&markdown),
            images: Vec::new(),
        });
    }

    let mut keys: Vec<&String> = doc.resources.keys().collect();
    keys.sort_by_key(|id| (doc.resources[*id].page, *id));

    let mut files: HashMap<String, String> = HashMap::new();
    let mut images: Vec<types::Image> = Vec::new();
    for id in &keys {
        let resource = &doc.resources[*id];
        if !resource.is_image() {
            continue;
        }
        let Some((bytes, ext)) = pdf_image::image_file(resource) else {
            continue;
        };
        let name = format!("{}.{}", sanitize(id), ext);
        files.insert((*id).clone(), name.clone());
        images.push(types::Image { name, data: bytes, page: resource.page });
    }

    let mut ordered: Vec<Option<String>> = Vec::new();
    for page in &doc.pages {
        for block in &page.elements {
            if let unpdf::Block::Image { resource_id, .. } = block {
                let key = base_id(resource_id);
                ordered.push(files.get(key).cloned());
            }
        }
    }

    markdown = link_placeholders(&markdown, &ordered);

    Ok(types::Converted { markdown, images })
}

fn office_image_ext(mime: Option<&str>) -> &'static str {
    match mime {
        Some(m) if m.contains("png") => "png",
        Some(m) if m.contains("jpeg") || m.contains("jpg") => "jpg",
        Some(m) if m.contains("gif") => "gif",
        Some(m) if m.contains("tiff") => "tiff",
        Some(m) if m.contains("bmp") => "bmp",
        Some(m) if m.contains("svg") => "svg",
        _ => "bin",
    }
}

// Entry names from a ZIP/OPC container; empty when the blob is not a readable
// archive. Used to tell OOXML (`[Content_Types].xml`), the binary xlsb workbook
// (`xl/workbook.bin`), and a plain archive apart, all of which share ZIP magic.
fn zip_names(blob: &[u8]) -> Vec<String> {
    match zip::ZipArchive::new(std::io::Cursor::new(blob)) {
        Ok(archive) => archive.file_names().map(|s| s.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

fn convert_office(
    blob: &[u8],
    settings: Settings,
    budget: &mut Budget,
) -> Result<types::Converted, ConversionError> {
    let names = zip_names(blob);
    if names.iter().any(|n| n == "xl/workbook.bin") {
        return xlsb::convert(blob, settings.extract_images).map_err(|e| fail(ERR_PARSE, e));
    }
    if names.iter().any(|n| n == "mimetype") {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(blob))
            .map_err(|e| fail(ERR_PARSE, e.to_string()))?;
        let mut mime = String::new();
        zip.by_name("mimetype")
            .map_err(|e| fail(ERR_PARSE, e.to_string()))?
            .take(128)
            .read_to_string(&mut mime)
            .map_err(|e| fail(ERR_PARSE, e.to_string()))?;
        if mime == "application/vnd.oasis.opendocument.spreadsheet" {
            return Ok(types::Converted {
                markdown: sheet::ods_markdown(blob).map_err(|e| fail(ERR_PARSE, e))?,
                images: Vec::new(),
            });
        }
    }
    let spreadsheet = if names.iter().any(|n| n == "xl/workbook.xml") {
        Some(sheet::xlsx_markdown(blob).map_err(|e| fail(ERR_PARSE, e))?)
    } else {
        None
    };
    if !settings.extract_images {
        if let Some(markdown) = &spreadsheet {
            return Ok(types::Converted {
                markdown: markdown.clone(),
                images: Vec::new(),
            });
        }
    }
    if !names.iter().any(|n| n == "[Content_Types].xml") {
        return archive::convert(blob, settings, budget, dispatch).map_err(|e| fail(ERR_PARSE, e));
    }

    let mut doc = undoc::parse_bytes(blob).map_err(|e| fail(ERR_PARSE, e.to_string()))?;
    if !settings.extract_images {
        doc.resources.clear();
    }
    let markdown = match spreadsheet {
        Some(markdown) => markdown,
        None => undoc::render::to_markdown(
            &doc,
            &undoc::render::RenderOptions {
                section_markers: if settings.page_markers {
                    undoc::render::SectionMarkerStyle::Comment
                } else {
                    undoc::render::SectionMarkerStyle::None
                },
                ..undoc::render::RenderOptions::default()
            },
        )
        .map_err(|e| fail(ERR_RENDER, e.to_string()))?,
    };

    if !settings.extract_images {
        return Ok(types::Converted {
            markdown,
            images: Vec::new(),
        });
    }

    let mut keys: Vec<&String> = doc.resources.keys().collect();
    keys.sort();

    let mut images = Vec::new();
    for id in keys {
        let resource = &doc.resources[id];
        if !resource.is_image() {
            continue;
        }
        let ext = office_image_ext(resource.mime_type.as_deref());
        let name = resource
            .filename
            .clone()
            .unwrap_or_else(|| format!("{}.{}", sanitize(id), ext));
        images.push(types::Image {
            page: None,
            name,
            data: resource.data.clone(),
        });
    }

    Ok(types::Converted { markdown, images })
}

fn converted_to_document(c: types::Converted) -> ConvertedDocument {
    let mut images = Vec::with_capacity(c.images.len());
    for img in c.images {
        images.push(Image {
            page: img.page,
            name: img.name,
            data: img.data,
        });
    }
    ConvertedDocument {
        markdown: c.markdown,
        images,
    }
}

// Legacy OLE2 compound documents (.doc/.xls/.ppt) share one magic; route by the
// marker stream present in the container.
fn convert_ole2(blob: &[u8], settings: Settings) -> Result<types::Converted, ConversionError> {
    let mut comp = ole::open(blob).map_err(|e| fail(ERR_PARSE, e))?;
    let names = ole::stream_names(&mut comp);
    let has = |n: &str| names.iter().any(|s| s.eq_ignore_ascii_case(n));
    let converted = if has("WordDocument") {
        doc::convert(blob, settings.extract_images)
    } else if has("Workbook") || has("Book") {
        xls::convert(blob, settings.extract_images)
    } else if has("PowerPoint Document") {
        ppt::convert(blob, settings.extract_images)
    } else {
        return Err(fail(
            ERR_UNSUPPORTED,
            "unsupported OLE2 compound document (expected legacy doc/xls/ppt)",
        ));
    };
    converted.map_err(|e| fail(ERR_PARSE, e))
}

fn convert_rtf(blob: &[u8], settings: Settings) -> Result<types::Converted, ConversionError> {
    rtf::convert(blob, settings.extract_images).map_err(|e| fail(ERR_PARSE, e))
}

fn convert_text(blob: &[u8]) -> Result<types::Converted, ConversionError> {
    markdown::convert(blob).map_err(|e| fail(ERR_PARSE, e))
}

fn convert_spss(blob: &[u8]) -> Result<types::Converted, ConversionError> {
    spss::convert(blob).map_err(|e| fail(ERR_PARSE, e))
}

const MAX_ARCHIVE_DEPTH: u32 = 6;
const MAX_TOTAL_DECOMPRESSED: u64 = 128 * 1024 * 1024;

// Shared, per-conversion resource budget. `depth` bounds archive nesting (stack
// safety); `bytes_remaining` is a global ceiling on the total decompressed bytes
// any nested archive may produce, which is the real defense against zip bombs —
// the whole archive tree shares one budget, so a small input cannot expand without
// bound. Bytes are only ever spent, never refunded; depth is bracketed per level.
pub(crate) struct Budget {
    depth: u32,
    bytes_remaining: u64,
}

impl Budget {
    fn root() -> Self {
        Budget {
            depth: 0,
            bytes_remaining: MAX_TOTAL_DECOMPRESSED,
        }
    }

    pub(crate) fn too_deep(&self) -> bool {
        self.depth > MAX_ARCHIVE_DEPTH
    }

    // Charge the budget for `n` decompressed bytes; false if it would overrun.
    pub(crate) fn charge(&mut self, n: u64) -> bool {
        match self.bytes_remaining.checked_sub(n) {
            Some(rest) => {
                self.bytes_remaining = rest;
                true
            }
            None => false,
        }
    }

    pub(crate) fn remaining(&self) -> u64 {
        self.bytes_remaining
    }

    pub(crate) fn enter(&mut self) {
        self.depth += 1;
    }

    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }
}

// Single content-sniffing router. The shared `Budget` bounds both archive nesting
// depth and total decompressed output across the whole nested-archive tree.
fn dispatch(
    blob: &[u8],
    settings: Settings,
    budget: &mut Budget,
) -> Result<types::Converted, ConversionError> {
    if budget.too_deep() {
        return Err(fail(ERR_PARSE, "archive nesting too deep"));
    }
    let mut converted = match sniff(blob) {
        Format::Pdf => convert_pdf(blob, settings),
        Format::Office => convert_office(blob, settings, budget),
        Format::Ole2 => convert_ole2(blob, settings),
        Format::Rtf => convert_rtf(blob, settings),
        Format::Spss => convert_spss(blob),
        Format::Text => convert_text(blob),
        Format::Unknown => Err(fail(
            ERR_UNSUPPORTED,
            "unsupported or unrecognized format (expected PDF, OOXML docx/xlsx/pptx, legacy doc/xls/ppt, rtf, text/markdown, or a zip archive of these)",
        )),
    }?;
    metafile::convert_in_place(&mut converted.images);
    Ok(converted)
}

// Per-page text length and image-resource count. A scanned page reads as little or
// no text with one or more images, which is the signal a caller uses to OCR only the
// pages that need it instead of rendering the whole document.
fn collect_page_resources(bytes: &[u8]) -> Result<Vec<PageInfo>, ConversionError> {
    if bytes.starts_with(PDF_MAGIC) {
        let blob = trim_pdf_trailer(bytes);
        let options = unpdf::ParseOptions::new().with_resources(true);
        let doc = unpdf::parse_bytes_with_options(blob, options)
            .map_err(|e| fail(ERR_PARSE, e.to_string()))?;
        let mut out = Vec::with_capacity(doc.pages.len());
        for page in &doc.pages {
            let mut text = String::new();
            let mut image_blocks: u32 = 0;
            for block in &page.elements {
                if matches!(block, unpdf::Block::Image { .. }) {
                    image_blocks += 1;
                }
                block.append_plain_text(&mut text);
            }
            out.push(PageInfo {
                page: page.number,
                text_len: text.trim().chars().count() as u32,
                image_count: image_blocks.max(page.images.len() as u32).max(page.image_resource_count),
            });
        }
        return Ok(out);
    }

    // Non-PDF formats are not scanned-image documents; report a single logical page
    // from the converted output so callers get a uniform shape.
    let mut budget = Budget::root();
    let converted = dispatch(
        bytes,
        Settings {
            extract_images: true,
            page_markers: false,
        },
        &mut budget,
    )?;
    Ok(vec![PageInfo {
        page: 1,
        text_len: converted.markdown.trim().chars().count() as u32,
        image_count: converted.images.len() as u32,
    }])
}

impl Guest for Component {
    fn convert(request: ConversionRequest) -> Result<ConvertedDocument, ConversionError> {
        let settings = Settings::from(request.options);
        let mut budget = Budget::root();
        dispatch(&request.bytes, settings, &mut budget).map(converted_to_document)
    }

    fn page_resources(bytes: Vec<u8>) -> Result<Vec<PageInfo>, ConversionError> {
        collect_page_resources(&bytes)
    }
}

export!(Component);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_detects_each_magic() {
        assert!(matches!(sniff(b"%PDF-1.7"), Format::Pdf));
        assert!(matches!(
            sniff(&[0x50, 0x4B, 0x03, 0x04, 0]),
            Format::Office
        ));
        assert!(matches!(sniff(&[0xD0, 0xCF, 0x11, 0xE0, 0]), Format::Ole2));
        assert!(matches!(sniff(b"{\\rtf1"), Format::Rtf));
        assert!(matches!(sniff(b"$FL2\x00\x00"), Format::Spss));
        assert!(matches!(sniff(b"plain text"), Format::Text));
        assert!(matches!(
            sniff(&[0xFF, 0xD8, 0xFF, 0x00, 0x01]),
            Format::Unknown
        ));
    }

    #[test]
    fn trim_pdf_trailer_drops_padding_after_eof() {
        let mut padded = b"%PDF-1.4\n... body ...\nstartxref\n9\n%%EOF".to_vec();
        let clean_len = padded.len();
        padded.extend_from_slice(&[0u8; 32]); // trailing NUL padding
        assert_eq!(trim_pdf_trailer(&padded).len(), clean_len);
    }

    #[test]
    fn trim_pdf_trailer_leaves_clean_pdf_untouched() {
        let pdf = b"%PDF-1.4\nbody\nstartxref\n9\n%%EOF";
        assert_eq!(trim_pdf_trailer(pdf).len(), pdf.len());
    }

    fn settings(extract_images: bool) -> Settings {
        Settings {
            extract_images,
            page_markers: false,
        }
    }

    fn request(bytes: &[u8], extract_images: bool) -> ConversionRequest {
        ConversionRequest {
            bytes: bytes.to_vec(),
            options: ConversionOptions {
                extract_images,
                page_markers: false,
            },
        }
    }

    #[test]
    fn converted_to_document_emits_markdown_and_images() {
        let c = types::Converted {
            markdown: "# hi".into(),
            images: vec![
                types::Image {
                    page: None,
                    name: "a.png".into(),
                    data: vec![1, 2],
                },
                types::Image {
                    page: None,
                    name: "b.jpg".into(),
                    data: vec![3],
                },
            ],
        };
        let doc = converted_to_document(c);
        assert_eq!(doc.markdown, "# hi");
        assert_eq!(doc.images.len(), 2);
        assert_eq!(doc.images[0].name, "a.png");
        assert_eq!(doc.images[0].data, vec![1, 2]);
        assert_eq!(doc.images[1].name, "b.jpg");
        assert_eq!(doc.images[1].data, vec![3]);
    }

    #[test]
    fn office_image_ext_maps_mime_types() {
        assert_eq!(office_image_ext(Some("image/png")), "png");
        assert_eq!(office_image_ext(Some("image/jpeg")), "jpg");
        assert_eq!(office_image_ext(Some("x/jpg")), "jpg");
        assert_eq!(office_image_ext(Some("image/gif")), "gif");
        assert_eq!(office_image_ext(Some("image/tiff")), "tiff");
        assert_eq!(office_image_ext(Some("image/bmp")), "bmp");
        assert_eq!(office_image_ext(Some("image/svg+xml")), "svg");
        assert_eq!(office_image_ext(Some("application/octet-stream")), "bin");
        assert_eq!(office_image_ext(None), "bin");
    }

    #[test]
    fn guest_convert_accepts_raw_bytes() {
        let doc = <Component as Guest>::convert(request(
            include_bytes!("../tests/assets/doc.doc"),
            false,
        ))
        .unwrap();
        assert!(doc.markdown.contains("Hello from DOC legacy"));
        assert!(doc.images.is_empty());
        // Plain text now passes through; genuinely binary, unrecognized bytes are rejected.
        let text = <Component as Guest>::convert(request(b"not a document", false)).unwrap();
        assert_eq!(text.markdown, "not a document");
        assert!(<Component as Guest>::convert(request(&[0x00, 0xFF, 0x00, 0x01], false)).is_err());
    }

    #[test]
    fn convert_archive_expands_entries_under_headings() {
        let doc = <Component as Guest>::convert(request(
            include_bytes!("../tests/assets/archive.zip"),
            false,
        ))
        .unwrap();
        assert!(doc.markdown.contains("## notes.md"));
        assert!(doc.markdown.contains("Hello from ZIP entry"));
        assert!(doc.markdown.contains("## xls.xls"));
        assert!(doc.markdown.contains("Hello from XLS"));
        // Every entry converted, so no "Skipped" footer is appended.
        assert!(!doc.markdown.contains("Skipped"));
    }

    #[test]
    fn convert_archive_reports_skipped_unsupported_entries() {
        let doc = <Component as Guest>::convert(request(
            include_bytes!("../tests/assets/archive_skip.zip"),
            false,
        ))
        .unwrap();
        assert!(doc.markdown.contains("## a.md"));
        // Exactly one unsupported entry -> singular footer naming it.
        assert!(
            doc.markdown
                .contains("Skipped 1 entry: junk.bin (unsupported)"),
            "got: {}",
            doc.markdown
        );
    }

    #[test]
    fn convert_archive_recurses_into_nested_archives() {
        let doc = <Component as Guest>::convert(request(
            include_bytes!("../tests/assets/archive_nested.zip"),
            false,
        ))
        .unwrap();
        assert!(doc.markdown.contains("## archive.zip"));
        assert!(doc.markdown.contains("Hello from ZIP entry"));
        assert!(doc.markdown.contains("Hello from XLS"));
    }

    #[test]
    fn zip_bomb_oversized_member_is_skipped_without_decompressing() {
        // bomb_oversized.zip declares a 70 MiB member (> 64 MiB per-entry cap) that
        // compresses to a few KB. The good member must still convert; the bomb is
        // skipped by the size guard and never fully decompressed.
        let doc = <Component as Guest>::convert(request(
            include_bytes!("../tests/assets/bomb_oversized.zip"),
            false,
        ))
        .unwrap();
        assert!(doc.markdown.contains("## good.md"));
        assert!(doc.markdown.contains("real content"));
        assert!(
            doc.markdown.contains("huge.bin (too large)"),
            "got: {}",
            doc.markdown
        );
    }

    #[test]
    fn zip_bomb_high_ratio_member_is_skipped() {
        // ratio.bin is 50 MiB (under the 64 MiB cap and the 128 MiB budget), so only
        // the compression-ratio guard (ratio 1030 > 1000) can reject it.
        let doc = <Component as Guest>::convert(request(
            include_bytes!("../tests/assets/bomb_ratio.zip"),
            false,
        ))
        .unwrap();
        assert!(doc.markdown.contains("## ok.md"));
        assert!(
            doc.markdown
                .contains("ratio.bin (suspicious compression ratio)"),
            "got: {}",
            doc.markdown
        );
    }

    #[test]
    fn zip_with_too_many_entries_is_capped() {
        // many_entries.zip holds 4097 members; only MAX_ENTRIES are iterated and the
        // overflow is reported rather than silently ignored.
        let doc = <Component as Guest>::convert(request(
            include_bytes!("../tests/assets/many_entries.zip"),
            false,
        ))
        .unwrap();
        assert!(
            doc.markdown
                .contains("further entries beyond the 4096-entry cap"),
            "missing entry-cap note"
        );
    }

    #[test]
    fn budget_charges_and_blocks_overrun() {
        let mut b = Budget::root();
        let start = b.remaining();
        assert!(b.charge(1000));
        assert_eq!(b.remaining(), start - 1000);
        assert!(!b.charge(u64::MAX)); // would overrun -> rejected, budget unchanged
        assert_eq!(b.remaining(), start - 1000);
    }

    #[test]
    fn budget_depth_bracketing_and_limit() {
        let mut b = Budget::root();
        assert!(!b.too_deep());
        for _ in 0..=MAX_ARCHIVE_DEPTH {
            b.enter();
        }
        assert!(b.too_deep());
        b.leave();
        assert!(!b.too_deep());
    }

    #[test]
    fn convert_ole2_routes_doc_to_word_converter() {
        let bytes = include_bytes!("../tests/assets/doc.doc");
        let doc = convert_ole2(bytes, settings(true)).unwrap();
        assert!(doc.markdown.contains("Hello from DOC legacy"));
        assert!(!doc.images.is_empty());
    }

    #[test]
    fn convert_ole2_routes_xls_to_spreadsheet_converter() {
        let bytes = include_bytes!("../tests/assets/xls.xls");
        let doc = convert_ole2(bytes, settings(false)).unwrap();
        assert!(doc.markdown.contains("Hello from XLS"));
        assert!(doc.images.is_empty());
    }

    #[test]
    fn convert_rtf_produces_document() {
        let doc = convert_rtf(
            br"{\rtf1 Hello from RTF legacy\par {\pict\pngblip 89504e47}}",
            settings(true),
        )
        .unwrap();
        assert!(doc.markdown.contains("Hello from RTF legacy"));
        assert!(!doc.images.is_empty());
    }

    #[test]
    fn convert_ole2_rejects_non_ole_bytes() {
        assert!(convert_ole2(b"not ole2 at all .........", settings(false)).is_err());
    }

    #[test]
    fn sanitize_replaces_non_id_chars() {
        assert_eq!(sanitize("a/b.c d-e_f"), "a_b_c_d-e_f");
        assert_eq!(sanitize("Keep09"), "Keep09");
    }

    #[test]
    fn base_id_drops_last_extension_only() {
        assert_eq!(base_id("page1.Im2.jpg"), "page1.Im2");
        assert_eq!(base_id("noext"), "noext");
    }

    #[test]
    fn link_placeholders_links_in_order_and_keeps_unmatched() {
        let md = "a <!-- [Fig 1] --> b <!-- [Fig 2] --> c";
        let ordered = vec![Some("image1.png".to_string())];
        assert_eq!(
            link_placeholders(md, &ordered),
            "a ![Fig 1](image1.png) b <!-- [Fig 2] --> c"
        );
    }

    #[test]
    fn convert_pdf_extracts_markdown() {
        let doc = convert_pdf(
            include_bytes!("../tests/assets/sample.pdf"),
            settings(false),
        )
        .unwrap();
        assert!(doc.markdown.contains("Hello from PDF"));
        assert!(doc.images.is_empty());
    }

    #[test]
    fn convert_pdf_compressed_filter_chain_ascii85_flate_extracts_exact_text() {
        // Public regression fixture: PDF stream encoded with filter chain
        // [ /ASCII85Decode /FlateDecode ].
        let doc = convert_pdf(
            include_bytes!("../tests/assets/ascii85_flate.pdf"),
            settings(false),
        )
        .unwrap();
        assert_eq!(
            doc.markdown.trim(),
            "PUBLIC ASCII85 REGRESSION: The same text must survive every encoding."
        );
        assert_eq!(doc.markdown.len(), 69);
        assert!(doc.images.is_empty());
    }

    #[test]
    fn font_statistics_count_tie_selects_smaller_body() {
        // Equal-frequency HashMap keys used to pick body_size from iteration
        // order. Smaller font must win so the larger size can still be a heading.
        for _ in 0..50 {
            let mut stats = unpdf::parser::FontStatistics::default();
            for _ in 0..20 {
                stats.add_size(16.0);
            }
            for _ in 0..20 {
                stats.add_size(10.0);
            }
            stats.analyze();
            assert_eq!(stats.body_size, 10.0);
            assert_eq!(stats.get_heading_level(10.0, false), 0);
            assert!(stats.get_heading_level(16.0, false) > 0);

            let mut reversed = unpdf::parser::FontStatistics::default();
            for _ in 0..20 {
                reversed.add_size(10.0);
            }
            for _ in 0..20 {
                reversed.add_size(16.0);
            }
            reversed.analyze();
            assert_eq!(reversed.body_size, stats.body_size);
            assert_eq!(
                reversed.get_heading_level(16.0, false),
                stats.get_heading_level(16.0, false)
            );
        }
    }

    #[test]
    fn convert_pdf_equal_frequency_fonts_pick_smaller_body_deterministically() {
        // Public generated fixture: three 16pt heading lines alternating with
        // three 10pt body lines (equal span counts). Smaller font is body.
        let blob = include_bytes!("../tests/assets/synthetic_heading_tie.pdf");
        let first = convert_pdf(blob, settings(false)).unwrap().markdown;
        for _ in 0..20 {
            let again = convert_pdf(blob, settings(false)).unwrap().markdown;
            assert_eq!(
                again, first,
                "PDF heading classification must be stable across runs"
            );
        }

        let heading = |needle: &str| {
            first.lines().any(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with('#') && trimmed.contains(needle)
            })
        };
        let body = |needle: &str| {
            first.lines().any(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with('#') && trimmed.contains(needle)
            })
        };

        assert!(
            heading("Alpha Heading One"),
            "tied 16pt line must be a heading:\n{first}"
        );
        assert!(
            heading("Beta Heading Two"),
            "tied 16pt line must be a heading:\n{first}"
        );
        assert!(
            heading("Gamma Heading Three"),
            "tied 16pt line must be a heading:\n{first}"
        );
        assert!(
            body("First regular body paragraph line."),
            "tied 10pt body must not be a heading:\n{first}"
        );
        assert!(
            body("Second regular body paragraph line."),
            "tied 10pt body must not be a heading:\n{first}"
        );
        assert!(
            body("Third regular body paragraph line."),
            "tied 10pt body must not be a heading:\n{first}"
        );
    }

    #[test]
    fn page_resources_reports_text_pages_for_pdf() {
        let pages = collect_page_resources(include_bytes!("../tests/assets/sample.pdf")).unwrap();
        assert!(!pages.is_empty(), "expected at least one page");
        assert_eq!(pages[0].page, 1);
        assert!(
            pages[0].text_len > 0,
            "a text PDF page reports extracted text"
        );
        assert_eq!(pages[0].image_count, 0, "the sample PDF has no images");
    }

    #[test]
    fn page_resources_reports_single_page_for_non_pdf() {
        let pages = collect_page_resources(include_bytes!("../tests/assets/docx.docx")).unwrap();
        assert_eq!(
            pages.len(),
            1,
            "a non-PDF document reports one logical page"
        );
        assert_eq!(pages[0].page, 1);
        assert!(pages[0].text_len > 0);
    }

    #[test]
    fn convert_office_extracts_docx_text_and_image() {
        let doc = convert_office(
            include_bytes!("../tests/assets/docx.docx"),
            settings(true),
            &mut Budget::root(),
        )
        .unwrap();
        assert!(doc.markdown.contains("Hello from DOCX"));
        assert!(!doc.images.is_empty());
    }

    #[test]
    fn convert_office_skips_images_when_disabled() {
        let doc = convert_office(
            include_bytes!("../tests/assets/docx.docx"),
            settings(false),
            &mut Budget::root(),
        )
        .unwrap();
        assert!(doc.markdown.contains("Hello from DOCX"));
        assert!(doc.images.is_empty());
    }

    #[test]
    fn ods_has_sheet_table_and_origin_annotation() {
        let converted = convert_office(
            include_bytes!("../tests/assets/sheet.ods"),
            settings(false),
            &mut Budget::root(),
        )
        .unwrap();
        assert!(converted.markdown.contains("## Products"));
        assert!(converted.markdown.contains("| A-1 | Zinc |"));
        assert!(converted
            .markdown
            .contains("<!-- kickside:sheet header-row="));
    }

    #[test]
    fn xlsx_has_sheet_origin_annotation() {
        let converted = convert_office(
            include_bytes!("../tests/assets/xlsx.xlsx"),
            settings(false),
            &mut Budget::root(),
        )
        .unwrap();
        assert!(converted
            .markdown
            .contains("<!-- kickside:sheet header-row="));
    }

    #[test]
    fn convert_office_handles_xlsx_and_pptx() {
        let xlsx = convert_office(
            include_bytes!("../tests/assets/xlsx.xlsx"),
            settings(false),
            &mut Budget::root(),
        )
        .unwrap();
        assert!(xlsx.markdown.contains("Hello from XLSX"));
        let pptx = convert_office(
            include_bytes!("../tests/assets/pptx.pptx"),
            settings(false),
            &mut Budget::root(),
        )
        .unwrap();
        assert!(pptx.markdown.contains("Hello from PPTX"));
    }

    fn located(extract_images: bool) -> Settings {
        Settings {
            extract_images,
            page_markers: true,
        }
    }

    #[test]
    fn convert_pdf_marks_each_page_when_page_markers_are_requested() {
        let doc =
            convert_pdf(include_bytes!("../tests/assets/sample.pdf"), located(false)).unwrap();
        assert!(
            doc.markdown.starts_with("<!-- page 1 -->"),
            "{}",
            doc.markdown
        );
        assert!(doc.markdown.contains("Hello from PDF"));
    }

    #[test]
    fn convert_pdf_keeps_page_markers_when_images_are_linked() {
        let doc = convert_pdf(include_bytes!("../tests/assets/sample.pdf"), located(true)).unwrap();
        assert!(doc.markdown.contains("<!-- page 1 -->"), "{}", doc.markdown);
    }

    #[test]
    fn convert_pdf_emits_no_page_markers_by_default() {
        let doc = convert_pdf(
            include_bytes!("../tests/assets/sample.pdf"),
            settings(false),
        )
        .unwrap();
        assert!(!doc.markdown.contains("<!-- page"), "{}", doc.markdown);
    }

    #[test]
    fn convert_office_marks_each_slide_when_page_markers_are_requested() {
        let pptx = convert_office(
            include_bytes!("../tests/assets/pptx.pptx"),
            located(false),
            &mut Budget::root(),
        )
        .unwrap();
        assert!(pptx.markdown.contains("<!-- slide 1"), "{}", pptx.markdown);
        let plain = convert_office(
            include_bytes!("../tests/assets/pptx.pptx"),
            settings(false),
            &mut Budget::root(),
        )
        .unwrap();
        assert!(!plain.markdown.contains("<!-- slide"), "{}", plain.markdown);
    }

    #[test]
    fn exported_convert_honours_the_page_markers_option() {
        let doc = <Component as Guest>::convert(ConversionRequest {
            bytes: include_bytes!("../tests/assets/sample.pdf").to_vec(),
            options: ConversionOptions {
                extract_images: false,
                page_markers: true,
            },
        })
        .unwrap();
        assert!(doc.markdown.contains("<!-- page 1 -->"), "{}", doc.markdown);
    }
}

#[cfg(test)]
mod test_allocations {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
        static COUNT: Cell<usize> = const { Cell::new(0) };
    }
    struct CountingAllocator;
    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if ENABLED.try_with(Cell::get).unwrap_or(false) { COUNT.with(|n| n.set(n.get() + 1)); }
            System.alloc(layout)
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) { System.dealloc(ptr, layout) }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            if ENABLED.try_with(Cell::get).unwrap_or(false) { COUNT.with(|n| n.set(n.get() + 1)); }
            System.realloc(ptr, layout, size)
        }
    }
    pub(crate) fn count<T>(f: impl FnOnce() -> T) -> (usize, T) {
        COUNT.with(|n| n.set(0));
        ENABLED.with(|enabled| enabled.set(true));
        let result = f();
        ENABLED.with(|enabled| enabled.set(false));
        (COUNT.with(Cell::get), result)
    }
}
