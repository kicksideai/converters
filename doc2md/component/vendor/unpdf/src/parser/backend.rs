//! PDF backend abstraction layer.
//!
//! Provides a trait-based interface for PDF operations, isolating
//! the concrete PDF parser from the layout analysis logic.

use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

use crate::error::{Error, Result};
use crate::model::{
    parse_calculator, ColorSpace, FieldType, FieldValue, FormField, Function, SampleLayout,
};

use super::encoding::{build_encoding_map, decode_with_encoding_map, BaseEncoding};
use super::font::{
    is_likely_binary, parse_to_unicode_cmap, parse_truetype_cmap_table, ToUnicodeMap,
};

/// Page identifier: (object number, generation number).
pub type PageId = (u32, u16);

/// Font information returned by the backend.
#[derive(Debug, Clone)]
pub struct BackendFontInfo {
    /// Font resource name (key in the page's font dictionary).
    pub name: Vec<u8>,
    /// Base font name (e.g., "Helvetica-Bold").
    pub base_font: String,
}

/// A value from a PDF content stream operand.
#[derive(Debug, Clone)]
pub enum PdfValue {
    Integer(i64),
    Real(f32),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Array(Vec<PdfValue>),
    Bool(bool),
    Dictionary(HashMap<Vec<u8>, PdfValue>),
    Other,
}

/// A single operation from a PDF content stream.
#[derive(Debug, Clone)]
pub struct ContentOp {
    pub operator: String,
    pub operands: Vec<PdfValue>,
}

/// Raw metadata from the PDF backend.
#[derive(Debug, Clone, Default)]
pub struct PdfMetadataRaw {
    pub version: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub creation_date: Option<String>,
    pub mod_date: Option<String>,
    pub encrypted: bool,
}

/// A raw outline (bookmark) item from the PDF.
#[derive(Debug, Clone)]
pub struct RawOutlineItem {
    pub title: String,
    pub page: Option<u32>,
    pub level: u8,
    pub children: Vec<RawOutlineItem>,
}

/// A raw XObject (image) extracted from a PDF page.
#[derive(Debug, Clone)]
pub struct RawXObject {
    pub name: String,
    pub subtype: String,
    pub data: Vec<u8>,
    pub filter: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bits_per_component: Option<u8>,
    pub color_space: Option<(String, Option<ColorSpace>)>,
    /// Set when `data` holds fully decoded samples in a resolved color space.
    pub samples: Option<SampleLayout>,
}

/// Decoded resources and their page-composition signal. Reusing an object does
/// not decode it again, but still adds a paint to the count.
#[derive(Default)]
pub struct RawPageXObjects {
    pub objects: Vec<RawXObject>,
    pub paint_count: u32,
}

/// Abstract interface for PDF document access.
///
/// Implementations provide page enumeration, font info, content stream
/// decoding, and text decoding — without exposing any concrete PDF library types.
pub trait PdfBackend: Send + Sync {
    /// Return all pages as (page_number → PageId).
    fn pages(&self) -> BTreeMap<u32, PageId>;

    /// Return font info for a given page.
    fn page_fonts(&self, page: PageId) -> Result<Vec<BackendFontInfo>>;

    /// Return the raw (decompressed) content stream bytes for a page.
    fn page_content(&self, page: PageId) -> Result<Vec<u8>>;

    /// Parse raw content stream bytes into a sequence of operations.
    fn decode_content(&self, data: &[u8]) -> Result<Vec<ContentOp>>;

    fn decode_page_content(&self, _page: PageId, data: &[u8]) -> Result<Vec<ContentOp>> {
        self.decode_content(data)
    }

    /// Decode a text byte sequence using the font's encoding on the given page.
    /// Falls back to simple decoding if the font or encoding is unavailable.
    fn decode_text(&self, page: PageId, font_name: &[u8], bytes: &[u8]) -> String;

    /// Return raw metadata (version, info dict fields, encryption status).
    fn metadata(&self) -> PdfMetadataRaw;

    /// Return page dimensions (width, height) in points.
    /// Falls back to Letter size (612, 792) if MediaBox is absent.
    fn page_dimensions(&self, page: PageId) -> (f32, f32);

    /// Return the document outline (bookmarks) as a tree.
    /// Implementations must handle cycle detection and depth limits.
    fn outline(&self) -> Result<Vec<RawOutlineItem>>;

    /// Return XObjects (images) from a page.
    fn page_xobjects(&self, page: PageId) -> Result<RawPageXObjects>;

    /// Extract AcroForm fields from the document.
    fn acroform_fields(&self) -> Vec<FormField> {
        vec![]
    }
}

// Re-export decode_text_simple as pub for external consumers.
pub use super::font::decode_text_simple;

/// Helper: extract a number from a [`PdfValue`].
pub fn get_number_from_value(val: &PdfValue) -> Option<f32> {
    match val {
        PdfValue::Integer(i) => Some(*i as f32),
        PdfValue::Real(r) => Some(*r),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// RawBackend — concrete implementation backed by custom parser
// ---------------------------------------------------------------------------

use super::raw::content as raw_content;
use super::raw::stream as raw_stream;
use super::raw::tokenizer::{
    dict_get as raw_dict_get, PdfDict as RawPdfDict, PdfObject as RawPdfObject,
    PdfStream as RawPdfStream,
};
use super::raw::RawDocument;

#[derive(Clone, Copy, Default)]
struct PaintGraphics<'a> {
    font: Option<&'a RawPdfDict>,
    fill_pattern: Option<&'a RawPdfObject>,
    stroke_pattern: Option<&'a RawPdfObject>,
    text_mode: i64,
}

/// Concrete [`PdfBackend`] backed by the custom `RawDocument` parser.
pub struct RawBackend {
    doc: RawDocument,
    font_resolver: RawFontResolver,
    #[cfg(test)]
    image_decodes: std::sync::atomic::AtomicUsize,
}

impl RawBackend {
    /// Load from a file path.
    pub fn load_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let data = std::fs::read(path).map_err(Error::Io)?;
        Self::load_bytes(&data)
    }

    /// Load from an in-memory byte slice.
    pub fn load_bytes(data: &[u8]) -> Result<Self> {
        let doc = RawDocument::load(data)?;
        Ok(Self {
            doc,
            font_resolver: RawFontResolver::new(),
            #[cfg(test)]
            image_decodes: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Load from a reader.
    pub fn load_reader<R: std::io::Read>(mut reader: R) -> Result<Self> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        Self::load_bytes(&data)
    }

    /// Check if the document is encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.doc.is_encrypted()
    }
}

impl PdfBackend for RawBackend {
    fn pages(&self) -> BTreeMap<u32, PageId> {
        self.doc.pages()
    }

    fn page_fonts(&self, page: PageId) -> Result<Vec<BackendFontInfo>> {
        self.font_resolver.page_fonts(&self.doc, page)
    }

    fn page_content(&self, page_id: PageId) -> Result<Vec<u8>> {
        let page_dict = self
            .doc
            .get_dict(page_id)
            .map_err(|e| Error::PdfParse(e.to_string()))?;

        let Some(contents) = raw_dict_get(page_dict, b"Contents") else {
            return Ok(Vec::new());
        };

        let contents = self.doc.resolve(contents);

        match contents {
            RawPdfObject::Reference(n, g) => {
                let obj = self
                    .doc
                    .get_object((*n, *g))
                    .ok_or_else(|| Error::PdfParse("Content stream not found".to_string()))?;
                let resolved = self.doc.resolve(obj);
                if let Some(stream) = resolved.as_stream() {
                    return raw_stream::decompress(stream, |o| self.doc.resolve(o));
                }
                Err(Error::PdfParse("Invalid content stream".to_string()))
            }
            RawPdfObject::Stream(stream) => raw_stream::decompress(stream, |o| self.doc.resolve(o)),
            RawPdfObject::Array(arr) => {
                let mut content = Vec::new();
                for item in arr {
                    let resolved = self.doc.resolve(item);
                    let stream_obj = match resolved {
                        RawPdfObject::Stream(s) => s,
                        RawPdfObject::Reference(n, g) => {
                            if let Some(obj) = self.doc.get_object((*n, *g)) {
                                let obj = self.doc.resolve(obj);
                                match obj.as_stream() {
                                    Some(s) => s,
                                    None => continue,
                                }
                            } else {
                                continue;
                            }
                        }
                        _ => continue,
                    };
                    if let Ok(data) = raw_stream::decompress(stream_obj, |o| self.doc.resolve(o)) {
                        content.extend_from_slice(&data);
                        content.push(b' ');
                    }
                }
                Ok(content)
            }
            _ => Err(Error::PdfParse("Invalid content stream".to_string())),
        }
    }

    fn decode_content(&self, data: &[u8]) -> Result<Vec<ContentOp>> {
        raw_content::parse_content_stream(data)
    }

    fn decode_page_content(&self, page: PageId, data: &[u8]) -> Result<Vec<ContentOp>> {
        self.decode_with_resources(data, self.page_resources(page))
    }

    fn decode_text(&self, page: PageId, font_name: &[u8], bytes: &[u8]) -> String {
        let mut text = self
            .font_resolver
            .decode_text(&self.doc, page, font_name, bytes);
        // Keep mapped sentinels through decoder selection so empty runs do not fall back.
        text.retain(|c| c != '\0');
        text
    }

    fn metadata(&self) -> PdfMetadataRaw {
        let trailer = self.doc.trailer();
        let mut meta = PdfMetadataRaw {
            version: self.doc.version.clone(),
            encrypted: self.doc.is_encrypted(),
            ..Default::default()
        };

        if let Some(info_ref) = raw_dict_get(trailer, b"Info") {
            if let Some((n, g)) = info_ref.as_reference() {
                if let Ok(info_dict) = self.doc.get_dict((n, g)) {
                    meta.title = raw_get_string(&self.doc, info_dict, b"Title");
                    meta.author = raw_get_string(&self.doc, info_dict, b"Author");
                    meta.subject = raw_get_string(&self.doc, info_dict, b"Subject");
                    meta.keywords = raw_get_string(&self.doc, info_dict, b"Keywords");
                    meta.creator = raw_get_string(&self.doc, info_dict, b"Creator");
                    meta.producer = raw_get_string(&self.doc, info_dict, b"Producer");
                    meta.creation_date = raw_get_string(&self.doc, info_dict, b"CreationDate");
                    meta.mod_date = raw_get_string(&self.doc, info_dict, b"ModDate");
                }
            }
        }

        meta
    }

    fn page_dimensions(&self, page: PageId) -> (f32, f32) {
        if let Some(dims) = self.find_media_box(page) {
            return dims;
        }
        (612.0, 792.0)
    }

    fn outline(&self) -> Result<Vec<RawOutlineItem>> {
        const MAX_DEPTH: u8 = 64;
        let mut items = Vec::new();
        let mut visited = std::collections::HashSet::new();

        let catalog = self.doc.catalog()?;
        if let Some(outlines_obj) = raw_dict_get(catalog, b"Outlines") {
            let outlines_obj = self.doc.resolve(outlines_obj);
            let outlines_dict = match outlines_obj {
                RawPdfObject::Dict(d) => Some(d),
                RawPdfObject::Reference(n, g) => self.doc.get_dict((*n, *g)).ok(),
                _ => None,
            };

            if let Some(outlines_dict) = outlines_dict {
                if let Some(first) = raw_dict_get(outlines_dict, b"First") {
                    if let Some(first_ref) = first.as_reference() {
                        self.collect_outline_items(
                            first_ref,
                            0,
                            MAX_DEPTH,
                            &mut items,
                            &mut visited,
                        );
                    }
                }
            }
        }

        Ok(items)
    }

    fn page_xobjects(&self, page: PageId) -> Result<RawPageXObjects> {
        let mut xobjects = RawPageXObjects::default();

        let empty_resources = RawPdfDict::new();
        let res_dict = self.page_resources(page).unwrap_or(&empty_resources);
        let mut seen = std::collections::HashSet::new();
        let mut active = std::collections::HashSet::new();
        let content = self.page_content(page)?;
        let ops = self.decode_with_resources(&content, Some(res_dict))?;
        self.painted_xobjects(&ops, res_dict, PaintGraphics::default(), &mut seen, &mut active, &mut xobjects)?;
        let page_dict = self.doc.get_dict(page)?;
        if let Some(annotations) =
            raw_dict_get(page_dict, b"Annots").and_then(|r| self.doc.resolve(r).as_array())
        {
            for annotation in annotations {
                let Some(dict) = raw_resolve_dict(&self.doc, annotation) else {
                    continue;
                };
                let flags = raw_dict_get(dict, b"F")
                    .and_then(|r| r.as_i64())
                    .unwrap_or(0);
                if flags & 3 != 0 {
                    continue;
                }
                let normal = raw_dict_get(dict, b"AP")
                    .and_then(|r| raw_resolve_dict(&self.doc, r))
                    .and_then(|dict| raw_dict_get(dict, b"N"));
                if let Some(normal) = normal {
                    let appearance = if let Some(states) = self.doc.resolve(normal).as_dict() {
                        raw_dict_get(dict, b"AS")
                            .and_then(|r| r.as_name())
                            .and_then(|name| raw_dict_get(states, name))
                    } else {
                        Some(normal)
                    };
                    if let Some(appearance) = appearance {
                        self.painted_stream(appearance, res_dict, PaintGraphics::default(), &mut seen, &mut active, &mut xobjects)?;
                    }
                }
            }
        }
        Ok(xobjects)
    }

    fn acroform_fields(&self) -> Vec<FormField> {
        self.extract_acroform_fields()
    }
}

impl RawBackend {
    fn page_resources(&self, mut page: PageId) -> Option<&RawPdfDict> {
        let mut seen = std::collections::HashSet::new();
        while seen.insert(page) {
            let dict = self.doc.get_dict(page).ok()?;
            if let Some(resources) = raw_dict_get(dict, b"Resources") {
                return raw_resolve_dict(&self.doc, resources);
            }
            page = raw_dict_get(dict, b"Parent")?.as_reference()?;
        }
        None
    }

    fn decode_with_resources(
        &self,
        data: &[u8],
        resources: Option<&RawPdfDict>,
    ) -> Result<Vec<ContentOp>> {
        let named = resources
            .and_then(|r| raw_dict_get(r, b"ColorSpace"))
            .and_then(|r| raw_resolve_dict(&self.doc, r));
        raw_content::parse_content_stream_with_colors(data, |color| {
            resolve_color_space(&self.doc, color, named, 0).map(|space| space.components())
        })
    }

    /// Follow image-bearing content streams in paint order. Object references
    /// identify extracted resources independently of names in resource dictionaries.
    fn painted_xobjects<'a>(
        &'a self,
        ops: &[ContentOp],
        res_dict: &'a RawPdfDict,
        mut graphics: PaintGraphics<'a>,
        seen: &mut std::collections::HashSet<(u32, u16)>,
        active: &mut std::collections::HashSet<(u32, u16)>,
        xobjects: &mut RawPageXObjects,
    ) -> Result<()> {
        let resource = |category: &[u8], name: &[u8]| {
            raw_dict_get(res_dict, category)
                .and_then(|r| raw_resolve_dict(&self.doc, r))
                .and_then(|dict| raw_dict_get(dict, name))
        };
        let mut saved_states = Vec::new();
        for op in ops {
            match op.operator.as_str() {
                "BI" => {
                    xobjects.paint_count += 1;
                    let stencil = match op.operands.last() {
                        Some(PdfValue::Dictionary(dict)) => matches!(
                            dict.get(b"IM".as_slice()).or_else(|| dict.get(b"ImageMask".as_slice())),
                            Some(PdfValue::Bool(true))
                        ),
                        _ => false,
                    };
                    if stencil {
                        if let Some(pattern) = graphics.fill_pattern {
                            self.painted_stream(pattern, res_dict, graphics, seen, active, xobjects)?;
                        }
                    }
                }
                "Do" => {
                    let Some(PdfValue::Name(name)) = op.operands.last() else {
                        continue;
                    };
                    if let Some(object) = resource(b"XObject", name) {
                        self.painted_stream(object, res_dict, graphics, seen, active, xobjects)?;
                    }
                }
                "scn" | "SCN" => {
                    let pattern = match op.operands.last() {
                        Some(PdfValue::Name(name)) => resource(b"Pattern", name),
                        _ => None,
                    };
                    if op.operator == "scn" {
                        graphics.fill_pattern = pattern;
                    } else {
                        graphics.stroke_pattern = pattern;
                    }
                }
                "cs" | "sc" | "g" | "rg" | "k" => graphics.fill_pattern = None,
                "CS" | "SC" | "G" | "RG" | "K" => graphics.stroke_pattern = None,
                "f" | "F" | "f*" | "S" | "s" | "B" | "B*" | "b" | "b*" => {
                    if !matches!(op.operator.as_str(), "S" | "s") {
                        if let Some(pattern) = graphics.fill_pattern {
                            self.painted_stream(pattern, res_dict, graphics, seen, active, xobjects)?;
                        }
                    }
                    if !matches!(op.operator.as_str(), "f" | "F" | "f*") {
                        if let Some(pattern) = graphics.stroke_pattern {
                            self.painted_stream(pattern, res_dict, graphics, seen, active, xobjects)?;
                        }
                    }
                }
                "gs" => {
                    let Some(PdfValue::Name(name)) = op.operands.last() else {
                        continue;
                    };
                    let state =
                        resource(b"ExtGState", name).and_then(|r| raw_resolve_dict(&self.doc, r));
                    if let Some(state) = state {
                        if let Some(mask) = raw_dict_get(state, b"SMask")
                            .and_then(|r| raw_resolve_dict(&self.doc, r))
                            .and_then(|dict| raw_dict_get(dict, b"G"))
                        {
                            self.painted_stream(mask, res_dict, graphics, seen, active, xobjects)?;
                        }
                        if let Some(value) = raw_dict_get(state, b"Font")
                            .and_then(|r| self.doc.resolve(r).as_array())
                            .and_then(|array| array.first())
                        {
                            graphics.font = raw_resolve_dict(&self.doc, value);
                        }
                    }
                }
                "Tf" => {
                    if let Some(PdfValue::Name(name)) = op.operands.first() {
                        graphics.font = resource(b"Font", name).and_then(|r| raw_resolve_dict(&self.doc, r));
                    }
                }
                "q" => saved_states.push(graphics),
                "Q" => {
                    if let Some(saved) = saved_states.pop() {
                        graphics = saved;
                    }
                }
                "Tr" => {
                    if let Some(PdfValue::Integer(mode)) = op.operands.first() {
                        graphics.text_mode = *mode;
                    }
                }
                "Tj" | "TJ" | "'" | "\"" => {
                    let has_text = op.operands.iter().any(|operand| match operand {
                        PdfValue::Str(bytes) => !bytes.is_empty(),
                        PdfValue::Array(values) => values.iter().any(|value| {
                            matches!(value, PdfValue::Str(bytes) if !bytes.is_empty())
                        }),
                        _ => false,
                    });
                    if !has_text {
                        continue;
                    }
                    let type3 = graphics.font.is_some_and(|font| {
                        raw_dict_get(font, b"Subtype").and_then(|s| s.as_name()) == Some(b"Type3")
                    });
                    if !type3 && matches!(graphics.text_mode, 0 | 2 | 4 | 6) {
                        if let Some(pattern) = graphics.fill_pattern {
                            self.painted_stream(pattern, res_dict, graphics, seen, active, xobjects)?;
                        }
                    }
                    if !type3 && matches!(graphics.text_mode, 1 | 2 | 5 | 6) {
                        if let Some(pattern) = graphics.stroke_pattern {
                            self.painted_stream(pattern, res_dict, graphics, seen, active, xobjects)?;
                        }
                    }
                    if let Some(font) = graphics.font {
                        for operand in &op.operands {
                            self.painted_glyphs(operand, font, res_dict, graphics, seen, active, xobjects)?;
                        }
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }

    fn painted_stream<'a>(
        &'a self,
        object: &RawPdfObject,
        resources: &'a RawPdfDict,
        graphics: PaintGraphics<'a>,
        seen: &mut std::collections::HashSet<(u32, u16)>,
        active: &mut std::collections::HashSet<(u32, u16)>,
        xobjects: &mut RawPageXObjects,
    ) -> Result<()> {
        let Some(reference) = object.as_reference() else {
            return Ok(());
        };
        let Some(stream) = self.doc.resolve(object).as_stream() else {
            return Ok(());
        };
        xobjects.paint_count += 1;
        let subtype = raw_dict_get(&stream.dict, b"Subtype").and_then(|s| s.as_name());
        let name = format!("object_{}_{}", reference.0, reference.1);
        if subtype == Some(b"Image") {
            let stencil = matches!(raw_dict_get(&stream.dict, b"ImageMask").map(|value| self.doc.resolve(value)),
                Some(RawPdfObject::Bool(true)));
            if stencil {
                if let Some(pattern) = graphics.fill_pattern {
                    self.painted_stream(pattern, resources, graphics, seen, active, xobjects)?;
                }
            }
            if seen.insert(reference) {
                if let Some(image) = self.raw_xobject(name, stream, resources) {
                    xobjects.objects.push(image);
                }
            }
        } else {
            if !active.insert(reference) {
                return Ok(());
            }
            if subtype == Some(b"Form") && seen.insert(reference) {
                if let Some(form) = self.raw_xobject(name, stream, resources) {
                    xobjects.objects.push(form);
                }
            }
            let resources = raw_dict_get(&stream.dict, b"Resources")
                .and_then(|r| raw_resolve_dict(&self.doc, r))
                .unwrap_or(resources);
            let content = raw_stream::decompress(stream, |o| self.doc.resolve(o))?;
            let ops = self.decode_with_resources(&content, Some(resources))?;
            self.painted_xobjects(&ops, resources, graphics, seen, active, xobjects)?;
            active.remove(&reference);
        }
        Ok(())
    }

    fn painted_glyphs<'a>(
        &'a self,
        operand: &PdfValue,
        font: &RawPdfDict,
        resources: &'a RawPdfDict,
        graphics: PaintGraphics<'a>,
        seen: &mut std::collections::HashSet<(u32, u16)>,
        active: &mut std::collections::HashSet<(u32, u16)>,
        xobjects: &mut RawPageXObjects,
    ) -> Result<()> {
        if raw_dict_get(font, b"Subtype").and_then(|s| s.as_name()) != Some(b"Type3") {
            return Ok(());
        }
        if let PdfValue::Array(values) = operand {
            for value in values {
                self.painted_glyphs(value, font, resources, graphics, seen, active, xobjects)?;
            }
        }
        let PdfValue::Str(bytes) = operand else {
            return Ok(());
        };
        let Some(glyphs) =
            raw_dict_get(font, b"CharProcs").and_then(|r| raw_resolve_dict(&self.doc, r))
        else {
            return Ok(());
        };
        let encoding_obj = raw_dict_get(font, b"Encoding").map(|r| self.doc.resolve(r));
        let encoding = encoding_obj.and_then(|r| raw_resolve_dict(&self.doc, r));
        let differences = encoding
            .map(|dict| self.font_resolver.parse_differences(&self.doc, dict))
            .unwrap_or_default();
        let base = encoding
            .and_then(|dict| raw_dict_get(dict, b"BaseEncoding"))
            .or(encoding_obj)
            .and_then(|r| r.as_name())
            .and_then(BaseEncoding::from_name)
            .unwrap_or(BaseEncoding::Standard);
        let resources = raw_dict_get(font, b"Resources")
            .and_then(|r| raw_resolve_dict(&self.doc, r))
            .unwrap_or(resources);
        for byte in bytes {
            let explicit = differences
                .iter()
                .rev()
                .find(|(code, _)| code == byte)
                .map(|(_, name)| name.as_bytes());
            let name = explicit.unwrap_or_else(|| base.glyph_name(*byte).as_bytes());
            if let Some(object) =
                raw_dict_get(glyphs, name).or_else(|| raw_dict_get(glyphs, b".notdef"))
            {
                self.painted_stream(object, resources, graphics, seen, active, xobjects)?;
            }
        }
        Ok(())
    }

    /// Decode one Image or Form XObject stream; other subtypes yield `None`.
    fn raw_xobject(
        &self,
        name: String,
        stream: &RawPdfStream,
        res_dict: &RawPdfDict,
    ) -> Option<RawXObject> {
        let dict = &stream.dict;

        let subtype = raw_dict_get(dict, b"Subtype")
            .and_then(|s| s.as_name())
            .map(|n| String::from_utf8_lossy(n).to_string())
            .unwrap_or_default();

        if subtype != "Image" && subtype != "Form" {
            return None;
        }

        let filter = raw_dict_get(dict, b"Filter")
            .and_then(|f| match self.doc.resolve(f) {
                RawPdfObject::Array(filters) if filters.len() == 1 => {
                    self.doc.resolve(&filters[0]).as_name()
                }
                filter => filter.as_name(),
            })
            .map(|n| String::from_utf8_lossy(n).to_string());

        #[cfg(test)]
        if subtype == "Image" {
            self.image_decodes
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let decoded = match filter.as_deref() {
            Some("DCTDecode") | Some("JPXDecode") => None,
            _ => raw_stream::decompress(stream, |o| self.doc.resolve(o)).ok(),
        };
        let samples_decoded = decoded.is_some();
        let data = decoded.unwrap_or_else(|| stream.raw_data.clone());

        let width = raw_dict_get(dict, b"Width")
            .and_then(|w| w.as_i64())
            .map(|w| w as u32);
        let height = raw_dict_get(dict, b"Height")
            .and_then(|h| h.as_i64())
            .map(|h| h as u32);
        let image_mask = matches!(
            raw_dict_get(dict, b"ImageMask").map(|m| self.doc.resolve(m)),
            Some(RawPdfObject::Bool(true))
        );
        let bits = if image_mask {
            Some(1)
        } else {
            raw_dict_get(dict, b"BitsPerComponent")
                .and_then(|b| self.doc.resolve(b).as_i64())
                .map(|b| b as u8)
        };

        let color_space =
            raw_dict_get(dict, b"ColorSpace").and_then(|cs| match self.doc.resolve(cs) {
                RawPdfObject::Name(n) => Some(String::from_utf8_lossy(n).to_string()),
                RawPdfObject::Array(arr) => arr
                    .first()
                    .and_then(|o| o.as_name())
                    .map(|n| String::from_utf8_lossy(n).to_string()),
                _ => None,
            });

        let resolved_color_space = if image_mask {
            Some(ColorSpace::Gray)
        } else {
            let named = raw_dict_get(res_dict, b"ColorSpace")
                .and_then(|cs| raw_resolve_dict(&self.doc, cs));
            raw_dict_get(dict, b"ColorSpace")
                .and_then(|cs| resolve_color_space(&self.doc, cs, named, 0))
        };
        let samples = resolved_color_space
            .clone()
            .filter(|_| subtype == "Image" && samples_decoded)
            .map(|color_space| SampleLayout {
                color_space,
                decode: raw_dict_get(dict, b"Decode").and_then(|d| raw_numbers(&self.doc, d)),
            });

        Some(RawXObject {
            name,
            subtype,
            data,
            filter,
            width,
            height,
            bits_per_component: bits,
            color_space: color_space.map(|name| (name, resolved_color_space)),
            samples,
        })
    }
}

impl RawBackend {
    /// Extract AcroForm fields from the document.
    fn extract_acroform_fields(&self) -> Vec<FormField> {
        let catalog = match self.doc.catalog() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let acroform = match raw_dict_get(catalog, b"AcroForm") {
            Some(obj) => self.doc.resolve(obj),
            None => return vec![],
        };

        let acroform_dict = match acroform {
            RawPdfObject::Dict(d) => d,
            RawPdfObject::Reference(n, g) => match self.doc.get_dict((*n, *g)) {
                Ok(d) => d,
                Err(_) => return vec![],
            },
            _ => return vec![],
        };

        let fields = match raw_dict_get(acroform_dict, b"Fields") {
            Some(obj) => self.doc.resolve(obj),
            None => return vec![],
        };

        let field_refs = match fields {
            RawPdfObject::Array(arr) => arr,
            RawPdfObject::Reference(n, g) => match self.doc.get_object((*n, *g)) {
                Some(obj) => match self.doc.resolve(obj).as_array() {
                    Some(arr) => arr,
                    None => return vec![],
                },
                None => return vec![],
            },
            _ => return vec![],
        };

        let mut result = Vec::new();
        for field_ref in field_refs {
            if let Some(id) = field_ref.as_reference() {
                self.traverse_field_tree(id, String::new(), None, &mut result);
            }
        }
        result
    }

    fn traverse_field_tree(
        &self,
        field_id: PageId,
        parent_name: String,
        inherited_ft: Option<Vec<u8>>,
        result: &mut Vec<FormField>,
    ) {
        let dict = match self.doc.get_dict(field_id) {
            Ok(d) => d,
            Err(_) => return,
        };

        // Build qualified name
        let partial_name = raw_dict_get(dict, b"T")
            .and_then(|o| o.as_str_bytes())
            .map(|s| String::from_utf8_lossy(s).to_string());

        let qualified_name = match &partial_name {
            Some(name) if parent_name.is_empty() => name.clone(),
            Some(name) => format!("{}.{}", parent_name, name),
            None => parent_name.clone(),
        };

        // Get field type (may be inherited from parent)
        let ft = raw_dict_get(dict, b"FT")
            .and_then(|o| o.as_name())
            .map(|n| n.to_vec())
            .or(inherited_ft.clone());

        // Check for Kids (non-terminal field)
        if let Some(kids) = raw_dict_get(dict, b"Kids") {
            let kids = self.doc.resolve(kids);
            if let Some(kids_arr) = kids.as_array() {
                for kid in kids_arr {
                    if let Some(kid_id) = kid.as_reference() {
                        self.traverse_field_tree(
                            kid_id,
                            qualified_name.clone(),
                            ft.clone(),
                            result,
                        );
                    }
                }
                return;
            }
        }

        // Terminal field — extract value
        let ft_bytes = match &ft {
            Some(ft) => ft.as_slice(),
            None => return,
        };

        let ff = raw_dict_get(dict, b"Ff")
            .and_then(|o| o.as_i64())
            .unwrap_or(0) as u32;

        let field_type = match ft_bytes {
            b"Tx" => FieldType::Text,
            b"Btn" => {
                if ff & (1 << 16) != 0 {
                    FieldType::RadioButton
                } else if ff & (1 << 17) != 0 {
                    FieldType::PushButton
                } else {
                    FieldType::Checkbox
                }
            }
            b"Ch" => {
                if ff & (1 << 17) != 0 {
                    FieldType::Dropdown
                } else {
                    FieldType::ListBox
                }
            }
            b"Sig" => FieldType::Signature,
            _ => return,
        };

        let value = self.extract_field_value(dict, &field_type);
        let default_value =
            raw_dict_get(dict, b"DV").and_then(|o| self.pdf_obj_to_field_value(o, &field_type));

        result.push(FormField {
            name: qualified_name,
            field_type,
            value,
            default_value,
        });
    }

    fn extract_field_value(&self, dict: &RawPdfDict, field_type: &FieldType) -> Option<FieldValue> {
        let v = raw_dict_get(dict, b"V")?;
        self.pdf_obj_to_field_value(v, field_type)
    }

    fn pdf_obj_to_field_value(
        &self,
        obj: &RawPdfObject,
        field_type: &FieldType,
    ) -> Option<FieldValue> {
        let obj = self.doc.resolve(obj);
        match field_type {
            FieldType::Text => obj
                .as_str_bytes()
                .map(|s| FieldValue::Text(String::from_utf8_lossy(s).to_string())),
            FieldType::Checkbox | FieldType::RadioButton => {
                obj.as_name().map(|n| FieldValue::Boolean(n != b"Off"))
            }
            FieldType::Dropdown | FieldType::ListBox => {
                if let Some(s) = obj.as_str_bytes() {
                    Some(FieldValue::Choice(String::from_utf8_lossy(s).to_string()))
                } else if let Some(arr) = obj.as_array() {
                    let choices: Vec<String> = arr
                        .iter()
                        .filter_map(|o| o.as_str_bytes())
                        .map(|s| String::from_utf8_lossy(s).to_string())
                        .collect();
                    Some(FieldValue::Choices(choices))
                } else {
                    None
                }
            }
            FieldType::PushButton | FieldType::Signature => None,
        }
    }

    /// Find MediaBox for a page, walking up the page tree for inherited values.
    fn find_media_box(&self, page_id: PageId) -> Option<(f32, f32)> {
        let dict = self.doc.get_dict(page_id).ok()?;

        if let Some(media_box) = raw_dict_get(dict, b"MediaBox") {
            if let Some(dims) = extract_dimensions_from_array(media_box) {
                return Some(dims);
            }
        }

        // Walk up to parent
        if let Some(parent) = raw_dict_get(dict, b"Parent") {
            if let Some(parent_id) = parent.as_reference() {
                return self.find_media_box_in_ancestor(parent_id);
            }
        }

        None
    }

    fn find_media_box_in_ancestor(&self, id: PageId) -> Option<(f32, f32)> {
        let dict = self.doc.get_dict(id).ok()?;

        if let Some(media_box) = raw_dict_get(dict, b"MediaBox") {
            if let Some(dims) = extract_dimensions_from_array(media_box) {
                return Some(dims);
            }
        }

        if let Some(parent) = raw_dict_get(dict, b"Parent") {
            if let Some(parent_id) = parent.as_reference() {
                return self.find_media_box_in_ancestor(parent_id);
            }
        }

        None
    }

    /// Collect outline items by following First/Next chain.
    fn collect_outline_items(
        &self,
        item_ref: PageId,
        level: u8,
        max_depth: u8,
        items: &mut Vec<RawOutlineItem>,
        visited: &mut std::collections::HashSet<PageId>,
    ) {
        if !visited.insert(item_ref) || level > max_depth {
            return;
        }

        if let Ok(item_dict) = self.doc.get_dict(item_ref) {
            let title = raw_get_string(&self.doc, item_dict, b"Title").unwrap_or_default();
            let page = self.resolve_outline_dest(item_dict);

            let mut outline_item = RawOutlineItem {
                title,
                page,
                level,
                children: Vec::new(),
            };

            if let Some(first) = raw_dict_get(item_dict, b"First") {
                if let Some(first_ref) = first.as_reference() {
                    self.collect_outline_items(
                        first_ref,
                        level + 1,
                        max_depth,
                        &mut outline_item.children,
                        visited,
                    );
                }
            }

            items.push(outline_item);

            if let Some(next) = raw_dict_get(item_dict, b"Next") {
                if let Some(next_ref) = next.as_reference() {
                    self.collect_outline_items(next_ref, level, max_depth, items, visited);
                }
            }
        }
    }

    /// Resolve an outline destination to a page number.
    fn resolve_outline_dest(&self, item_dict: &RawPdfDict) -> Option<u32> {
        let pages = self.doc.pages();

        // Try Dest
        if let Some(dest) = raw_dict_get(item_dict, b"Dest") {
            let dest = self.doc.resolve(dest);
            if let Some(arr) = dest.as_array() {
                if let Some(first) = arr.first() {
                    if let Some(page_ref) = first.as_reference() {
                        for (num, id) in pages.iter() {
                            if *id == page_ref {
                                return Some(*num);
                            }
                        }
                    }
                }
            }
        }

        // Try A (action) dictionary
        if let Some(action) = raw_dict_get(item_dict, b"A") {
            let action = self.doc.resolve(action);
            let action_dict = match action {
                RawPdfObject::Dict(d) => Some(d),
                RawPdfObject::Reference(n, g) => self.doc.get_dict((*n, *g)).ok(),
                _ => None,
            };

            if let Some(action_dict) = action_dict {
                if let Some(dest) = raw_dict_get(action_dict, b"D") {
                    let dest = self.doc.resolve(dest);
                    if let Some(arr) = dest.as_array() {
                        if let Some(first) = arr.first() {
                            if let Some(page_ref) = first.as_reference() {
                                for (num, id) in pages.iter() {
                                    if *id == page_ref {
                                        return Some(*num);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// RawFontResolver — font resolution for RawBackend
// ---------------------------------------------------------------------------

struct RawFontResolver {
    cmap_cache: RwLock<HashMap<PageId, Option<ToUnicodeMap>>>,
    encoding_cache: RwLock<HashMap<PageId, Option<HashMap<u8, char>>>>,
    cid_system_info_cache: RwLock<HashMap<PageId, Option<(String, String)>>>,
}

impl RawFontResolver {
    fn new() -> Self {
        Self {
            cmap_cache: RwLock::new(HashMap::new()),
            encoding_cache: RwLock::new(HashMap::new()),
            cid_system_info_cache: RwLock::new(HashMap::new()),
        }
    }

    fn decode_text(
        &self,
        doc: &RawDocument,
        page: PageId,
        font_name: &[u8],
        bytes: &[u8],
    ) -> String {
        let font_obj_id = self.find_font_dict(doc, page, font_name);
        let mut is_identity_h = false;

        // 1. Try ToUnicode CMap first
        if let Some(fid) = font_obj_id {
            if let Some(cmap) = self.get_to_unicode_map(doc, fid) {
                let decoded = cmap.decode(bytes);
                if !decoded.is_empty() {
                    return decoded;
                }
            }
            is_identity_h = self.is_identity_cid_font(doc, fid);
        }

        // 2. Try embedded TrueType cmap table (for Identity-H CID fonts without ToUnicode)
        if let Some(fid) = font_obj_id {
            if let Some(cmap) = self.get_embedded_cmap(doc, fid) {
                let decoded = cmap.decode(bytes);
                if !decoded.is_empty() {
                    return decoded;
                }
            }
        }

        // 3. Try CIDSystemInfo-based CMap resource lookup (for Identity-H CID fonts)
        if is_identity_h {
            if let Some(fid) = font_obj_id {
                if let Some((registry, ordering)) = self.get_cid_system_info_cached(doc, fid) {
                    if let Some(decoded) = crate::parser::cmap_table::decode_with_cid_system_info(
                        &registry, &ordering, bytes,
                    ) {
                        if !decoded.is_empty() {
                            return decoded;
                        }
                    }
                }
            }
        }

        // 4. Try encoding dictionary (BaseEncoding + Differences)
        if let Some(fid) = font_obj_id {
            if let Some(enc_map) = self.get_encoding_map(doc, fid) {
                let decoded = decode_with_encoding_map(bytes, &enc_map);
                if !decoded.is_empty() {
                    return decoded;
                }
            }
        }

        // For Identity-H/V fonts, simple fallback always produces garbage
        if is_identity_h {
            return String::new();
        }

        // 5. Final fallback
        let simple = decode_text_simple(bytes);
        if is_likely_binary(&simple) {
            String::new()
        } else {
            simple
        }
    }

    /// Find the font dictionary object ID for a given font name on a page.
    fn find_font_dict(&self, doc: &RawDocument, page: PageId, font_name: &[u8]) -> Option<PageId> {
        // Try the page's own Resources
        if let Some(fid) = self.find_font_in_resources(doc, page, font_name) {
            return Some(fid);
        }

        // Walk up the Pages tree for inherited Resources
        if let Ok(page_dict) = doc.get_dict(page) {
            if let Some(parent) = raw_dict_get(page_dict, b"Parent") {
                if let Some(parent_id) = parent.as_reference() {
                    return self.find_font_in_ancestor(doc, parent_id, font_name);
                }
            }
        }

        None
    }

    fn find_font_in_resources(
        &self,
        doc: &RawDocument,
        obj_id: PageId,
        font_name: &[u8],
    ) -> Option<PageId> {
        let dict = doc.get_dict(obj_id).ok()?;
        let resources = raw_dict_get(dict, b"Resources")?;
        self.find_font_in_resource_obj(doc, resources, font_name)
    }

    fn find_font_in_resource_obj(
        &self,
        doc: &RawDocument,
        resources: &RawPdfObject,
        font_name: &[u8],
    ) -> Option<PageId> {
        let res_dict = raw_resolve_dict(doc, resources)?;
        let font_obj = raw_dict_get(res_dict, b"Font")?;
        let font_dict = raw_resolve_dict(doc, font_obj)?;
        let font_entry = raw_dict_get(font_dict, font_name)?;
        font_entry.as_reference()
    }

    fn find_font_in_ancestor(
        &self,
        doc: &RawDocument,
        ancestor_id: PageId,
        font_name: &[u8],
    ) -> Option<PageId> {
        let dict = doc.get_dict(ancestor_id).ok()?;

        if let Some(resources) = raw_dict_get(dict, b"Resources") {
            if let Some(fid) = self.find_font_in_resource_obj(doc, resources, font_name) {
                return Some(fid);
            }
        }

        if let Some(parent) = raw_dict_get(dict, b"Parent") {
            if let Some(parent_id) = parent.as_reference() {
                return self.find_font_in_ancestor(doc, parent_id, font_name);
            }
        }

        None
    }

    /// Get or parse the ToUnicode CMap for a font.
    fn get_to_unicode_map(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<ToUnicodeMap> {
        {
            let cache = self.cmap_cache.read().unwrap();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }

        let result = self.parse_font_to_unicode(doc, font_obj_id);
        self.cmap_cache
            .write()
            .unwrap()
            .insert(font_obj_id, result.clone());
        result
    }

    fn parse_font_to_unicode(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<ToUnicodeMap> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;
        let to_unicode = raw_dict_get(font_dict, b"ToUnicode")?;
        let to_unicode = doc.resolve(to_unicode);

        let stream = match to_unicode {
            RawPdfObject::Stream(s) => s,
            RawPdfObject::Reference(n, g) => {
                let obj = doc.get_object((*n, *g))?;
                let resolved = doc.resolve(obj);
                resolved.as_stream()?
            }
            _ => return None,
        };

        let data = raw_stream::decompress(stream, |o| doc.resolve(o))
            .unwrap_or_else(|_| stream.raw_data.clone());
        parse_to_unicode_cmap(&data)
    }

    /// Check if a font uses Identity-H or Identity-V CID encoding.
    fn is_identity_cid_font(&self, doc: &RawDocument, font_obj_id: PageId) -> bool {
        let font_dict = match doc.get_dict(font_obj_id) {
            Ok(d) => d,
            Err(_) => return false,
        };

        raw_dict_get(font_dict, b"Encoding")
            .and_then(|e| e.as_name())
            .map(|n| n == b"Identity-H" || n == b"Identity-V")
            .unwrap_or(false)
    }

    /// Get the CIDFont's object ID from a Type0 font.
    fn get_cid_font_id(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<PageId> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;
        let descendants = raw_dict_get(font_dict, b"DescendantFonts")?;
        let descendants = doc.resolve(descendants);

        let arr = descendants.as_array()?;
        arr.first()?.as_reference()
    }

    /// Extract CIDSystemInfo (Registry, Ordering) from a CIDFont.
    fn get_cid_system_info(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<(String, String)> {
        let cid_font_id = self.get_cid_font_id(doc, font_obj_id)?;
        let cid_font_dict = doc.get_dict(cid_font_id).ok()?;

        let csi = raw_dict_get(cid_font_dict, b"CIDSystemInfo")?;
        let csi = doc.resolve(csi);
        let csi_dict = match csi {
            RawPdfObject::Dict(d) => d,
            RawPdfObject::Reference(n, g) => doc.get_dict((*n, *g)).ok()?,
            _ => return None,
        };

        let registry = raw_dict_get(csi_dict, b"Registry")
            .and_then(|o| o.as_str_bytes())
            .map(|s| String::from_utf8_lossy(s).to_string())?;

        let ordering = raw_dict_get(csi_dict, b"Ordering")
            .and_then(|o| o.as_str_bytes())
            .map(|s| String::from_utf8_lossy(s).to_string())?;

        Some((registry, ordering))
    }

    fn get_cid_system_info_cached(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<(String, String)> {
        {
            let cache = self.cid_system_info_cache.read().unwrap();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }
        let result = self.get_cid_system_info(doc, font_obj_id);
        self.cid_system_info_cache
            .write()
            .unwrap()
            .insert(font_obj_id, result.clone());
        result
    }

    /// Get or parse embedded TrueType cmap for Identity-H fonts.
    fn get_embedded_cmap(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<ToUnicodeMap> {
        let cid_font_id = self.get_cid_font_id(doc, font_obj_id)?;

        {
            let cache = self.cmap_cache.read().unwrap();
            if let Some(cached) = cache.get(&cid_font_id) {
                return cached.clone();
            }
        }

        let result = self.parse_embedded_truetype_cmap(doc, font_obj_id);
        self.cmap_cache
            .write()
            .unwrap()
            .insert(cid_font_id, result.clone());
        result
    }

    fn parse_embedded_truetype_cmap(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<ToUnicodeMap> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;

        // Check Identity-H/V encoding
        let encoding = raw_dict_get(font_dict, b"Encoding")
            .and_then(|e| e.as_name())
            .map(|n| String::from_utf8_lossy(n).to_string())?;

        if encoding != "Identity-H" && encoding != "Identity-V" {
            return None;
        }

        // Get CIDFont from DescendantFonts
        let cid_font_id = self.get_cid_font_id(doc, font_obj_id)?;
        let cid_font_dict = doc.get_dict(cid_font_id).ok()?;

        // Get FontDescriptor
        let fd_ref = raw_dict_get(cid_font_dict, b"FontDescriptor")?.as_reference()?;
        let fd_dict = doc.get_dict(fd_ref).ok()?;

        // Get FontFile2 (TrueType)
        let ff2 = raw_dict_get(fd_dict, b"FontFile2")?;
        let ff2 = doc.resolve(ff2);
        let font_stream = match ff2 {
            RawPdfObject::Stream(s) => s,
            RawPdfObject::Reference(n, g) => {
                let obj = doc.get_object((*n, *g))?;
                let resolved = doc.resolve(obj);
                resolved.as_stream()?
            }
            _ => return None,
        };

        let font_data = raw_stream::decompress(font_stream, |o| doc.resolve(o))
            .unwrap_or_else(|_| font_stream.raw_data.clone());
        parse_truetype_cmap_table(&font_data)
    }

    /// Get or parse the encoding map for a font.
    fn get_encoding_map(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<HashMap<u8, char>> {
        {
            let cache = self.encoding_cache.read().unwrap();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }

        let result = self.parse_encoding_dict(doc, font_obj_id);
        self.encoding_cache
            .write()
            .unwrap()
            .insert(font_obj_id, result.clone());
        result
    }

    /// Parse the /Encoding entry from a font dictionary.
    ///
    /// The /Encoding can be:
    /// - A Name (e.g., /WinAnsiEncoding) → use that base encoding directly
    /// - A Dict with /BaseEncoding and /Differences → build a custom encoding map
    fn parse_encoding_dict(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<HashMap<u8, char>> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;
        let encoding_obj = raw_dict_get(font_dict, b"Encoding")?;
        let encoding_obj = doc.resolve(encoding_obj);

        match encoding_obj {
            // Simple name: /WinAnsiEncoding, /MacRomanEncoding, /StandardEncoding
            RawPdfObject::Name(name) => {
                let base = BaseEncoding::from_name(name)?;
                Some(build_encoding_map(Some(base), &[]))
            }
            // Encoding dictionary with optional BaseEncoding and Differences
            RawPdfObject::Dict(dict) => {
                let base = raw_dict_get(dict, b"BaseEncoding")
                    .and_then(|b| b.as_name())
                    .and_then(BaseEncoding::from_name);

                let differences = self.parse_differences(doc, dict);
                Some(build_encoding_map(base, &differences))
            }
            RawPdfObject::Reference(n, g) => {
                let obj = doc.get_object((*n, *g))?;
                let resolved = doc.resolve(obj);
                match resolved {
                    RawPdfObject::Name(name) => {
                        let base = BaseEncoding::from_name(name)?;
                        Some(build_encoding_map(Some(base), &[]))
                    }
                    RawPdfObject::Dict(dict) => {
                        let base = raw_dict_get(dict, b"BaseEncoding")
                            .and_then(|b| b.as_name())
                            .and_then(BaseEncoding::from_name);
                        let differences = self.parse_differences(doc, dict);
                        Some(build_encoding_map(base, &differences))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Parse a /Differences array from an encoding dictionary.
    ///
    /// Format: `[code1 /name1 /name2 ... codeN /nameN ...]`
    /// Each integer sets the starting code, and subsequent names map consecutive codes.
    fn parse_differences(&self, doc: &RawDocument, dict: &RawPdfDict) -> Vec<(u8, String)> {
        let mut result = Vec::new();
        let diff_obj = match raw_dict_get(dict, b"Differences") {
            Some(d) => d,
            None => return result,
        };
        let diff_obj = doc.resolve(diff_obj);
        let arr = match diff_obj.as_array() {
            Some(a) => a,
            None => return result,
        };

        let mut current_code: u32 = 0;
        for item in arr {
            let item = doc.resolve(item);
            match item {
                RawPdfObject::Integer(n) => {
                    current_code = *n as u32;
                }
                RawPdfObject::Name(name) => {
                    if current_code <= 255 {
                        let glyph_name = String::from_utf8_lossy(name).to_string();
                        result.push((current_code as u8, glyph_name));
                    }
                    current_code += 1;
                }
                _ => {}
            }
        }

        result
    }

    /// Collect font info for a page (with inherited resources fallback).
    fn page_fonts(&self, doc: &RawDocument, page: PageId) -> Result<Vec<BackendFontInfo>> {
        if let Some(fonts) = self.collect_fonts_from_page(doc, page) {
            if !fonts.is_empty() {
                return Ok(fonts);
            }
        }

        // Try inherited resources
        if let Ok(page_dict) = doc.get_dict(page) {
            if let Some(parent) = raw_dict_get(page_dict, b"Parent") {
                if let Some(parent_id) = parent.as_reference() {
                    if let Some(fonts) = self.collect_fonts_from_ancestor(doc, parent_id) {
                        return Ok(fonts);
                    }
                }
            }
        }

        Ok(Vec::new())
    }

    fn collect_fonts_from_page(
        &self,
        doc: &RawDocument,
        page: PageId,
    ) -> Option<Vec<BackendFontInfo>> {
        let dict = doc.get_dict(page).ok()?;
        let resources = raw_dict_get(dict, b"Resources")?;
        self.collect_fonts_from_resource_obj(doc, resources)
    }

    fn collect_fonts_from_ancestor(
        &self,
        doc: &RawDocument,
        ancestor_id: PageId,
    ) -> Option<Vec<BackendFontInfo>> {
        let dict = doc.get_dict(ancestor_id).ok()?;

        if let Some(resources) = raw_dict_get(dict, b"Resources") {
            if let Some(fonts) = self.collect_fonts_from_resource_obj(doc, resources) {
                if !fonts.is_empty() {
                    return Some(fonts);
                }
            }
        }

        if let Some(parent) = raw_dict_get(dict, b"Parent") {
            if let Some(parent_id) = parent.as_reference() {
                return self.collect_fonts_from_ancestor(doc, parent_id);
            }
        }

        None
    }

    fn collect_fonts_from_resource_obj(
        &self,
        doc: &RawDocument,
        resources: &RawPdfObject,
    ) -> Option<Vec<BackendFontInfo>> {
        let res_dict = raw_resolve_dict(doc, resources)?;
        let font_obj = raw_dict_get(res_dict, b"Font")?;
        let font_dict = raw_resolve_dict(doc, font_obj)?;

        let mut result = Vec::new();
        for (name, val) in font_dict {
            let font_id = match val.as_reference() {
                Some(r) => r,
                None => continue,
            };
            let fd = match doc.get_dict(font_id) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let base_font = raw_dict_get(fd, b"BaseFont")
                .and_then(|o| o.as_name())
                .map(|n| String::from_utf8_lossy(n).to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            result.push(BackendFontInfo {
                name: name.clone(),
                base_font,
            });
        }

        Some(result)
    }
}

// ---------------------------------------------------------------------------
// RawBackend helper functions
// ---------------------------------------------------------------------------

/// Resolve a PdfObject to a dictionary reference (following references).
fn raw_resolve_dict<'a>(doc: &'a RawDocument, obj: &'a RawPdfObject) -> Option<&'a RawPdfDict> {
    let resolved = doc.resolve(obj);
    match resolved {
        RawPdfObject::Dict(d) => Some(d),
        RawPdfObject::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

/// Nesting bound for color spaces and functions that refer to others.
const MAX_COLOR_DEPTH: usize = 8;

/// Resolve a number array, following references.
fn raw_numbers(doc: &RawDocument, obj: &RawPdfObject) -> Option<Vec<f32>> {
    doc.resolve(obj)
        .as_array()?
        .iter()
        .map(|o| doc.resolve(o).as_f32())
        .collect()
}

fn raw_stream_data(doc: &RawDocument, obj: &RawPdfObject) -> Option<Vec<u8>> {
    match doc.resolve(obj) {
        RawPdfObject::Stream(stream) => raw_stream::decompress(stream, |o| doc.resolve(o)).ok(),
        RawPdfObject::Str(bytes) => Some(bytes.clone()),
        _ => None,
    }
}

/// Resolve an image color space (ISO 32000-1 8.6). `named` is the page's
/// `/ColorSpace` resource dictionary, used for color spaces given by name.
fn resolve_color_space(
    doc: &RawDocument,
    obj: &RawPdfObject,
    named: Option<&RawPdfDict>,
    depth: usize,
) -> Option<ColorSpace> {
    if depth > MAX_COLOR_DEPTH {
        return None;
    }
    let obj = doc.resolve(obj);
    let (family, params): (&[u8], &[RawPdfObject]) = match obj {
        RawPdfObject::Name(name) => (name.as_slice(), &[]),
        RawPdfObject::Array(items) => (items.first()?.as_name()?, &items[1..]),
        _ => return None,
    };
    match family {
        b"DeviceGray" | b"G" | b"CalGray" => Some(ColorSpace::Gray),
        b"DeviceRGB" | b"RGB" | b"CalRGB" => Some(ColorSpace::Rgb),
        b"DeviceCMYK" | b"CMYK" => Some(ColorSpace::Cmyk),
        b"Lab" => {
            let dict = raw_resolve_dict(doc, params.first()?)?;
            let white = raw_dict_get(dict, b"WhitePoint").and_then(|w| raw_numbers(doc, w))?;
            let range = raw_dict_get(dict, b"Range")
                .and_then(|r| raw_numbers(doc, r))
                .unwrap_or_else(|| vec![-100.0, 100.0, -100.0, 100.0]);
            Some(ColorSpace::Lab {
                white_point: white.get(..3)?.try_into().ok()?,
                range: range.get(..4)?.try_into().ok()?,
            })
        }
        b"ICCBased" => {
            let dict = raw_resolve_dict(doc, params.first()?)?;
            let n = raw_dict_get(dict, b"N").and_then(|n| doc.resolve(n).as_i64())?;
            let device = match n {
                1 => ColorSpace::Gray,
                3 => ColorSpace::Rgb,
                4 => ColorSpace::Cmyk,
                _ => return None,
            };
            let alternate = match raw_dict_get(dict, b"Alternate") {
                Some(alternate) => resolve_color_space(doc, alternate, named, depth + 1)?,
                None => device,
            };
            if alternate.components() != n as usize {
                return None;
            }
            let range = match raw_dict_get(dict, b"Range") {
                Some(r) => raw_numbers(doc, r)?,
                None => [0.0, 1.0].repeat(n as usize),
            };
            if range.len() != 2 * n as usize || range.chunks_exact(2).any(|r| r[0] > r[1]) {
                return None;
            }
            Some(ColorSpace::Icc {
                alternate: Box::new(alternate),
                range,
            })
        }
        b"Indexed" | b"I" => {
            let base = resolve_color_space(doc, params.first()?, named, depth + 1)?;
            let hival = doc.resolve(params.get(1)?).as_i64()?;
            let lookup = raw_stream_data(doc, params.get(2)?)?;
            if !(0..=255).contains(&hival)
                || lookup.len() < (hival as usize + 1) * base.components()
            {
                return None;
            }
            Some(ColorSpace::Indexed {
                base: Box::new(base),
                hival: hival as u32,
                lookup,
            })
        }
        b"Separation" | b"DeviceN" => {
            let colorant = if family == b"Separation" {
                Some(String::from_utf8_lossy(doc.resolve(params.first()?).as_name()?).into_owned())
            } else {
                None
            };
            if let Some(name) = colorant.as_deref() {
                if name == "All" || name == "None" {
                    return Some(ColorSpace::Separation {
                        name: name.to_string(),
                        tint: None,
                    });
                }
            }
            let names = if family == b"DeviceN" {
                doc.resolve(params.first()?)
                    .as_array()?
                    .iter()
                    .map(|name| {
                        Some(String::from_utf8_lossy(doc.resolve(name).as_name()?).into_owned())
                    })
                    .collect::<Option<Vec<_>>>()?
            } else {
                Vec::new()
            };
            let components = if colorant.is_some() { 1 } else { names.len() };
            let alternate = resolve_color_space(doc, params.get(1)?, named, depth + 1)?;
            let tint_transform = resolve_function(doc, params.get(2)?, depth + 1)?;
            if components == 0
                || components > 32
                || tint_transform.outputs() != alternate.components()
            {
                return None;
            }
            Some(match colorant {
                Some(name) => ColorSpace::Separation {
                    name,
                    tint: Some((Box::new(alternate), tint_transform)),
                },
                None => ColorSpace::DeviceN {
                    components,
                    names,
                    alternate: Box::new(alternate),
                    tint_transform,
                },
            })
        }
        name if params.is_empty() => {
            let entry = raw_dict_get(named?, name)?;
            resolve_color_space(doc, entry, None, depth + 1)
        }
        _ => None,
    }
}

/// Resolve a PDF function (ISO 32000-1 7.10).
fn resolve_function(doc: &RawDocument, obj: &RawPdfObject, depth: usize) -> Option<Function> {
    if depth > MAX_COLOR_DEPTH {
        return None;
    }
    let obj = doc.resolve(obj);
    if let RawPdfObject::Array(items) = obj {
        if items.is_empty() || items.len() > 32 {
            return None;
        }
        return items
            .iter()
            .map(|item| resolve_function(doc, item, depth + 1))
            .collect::<Option<Vec<_>>>()
            .filter(|functions| functions.iter().all(|f| f.outputs() == 1))
            .map(Function::Array);
    }
    let dict = raw_resolve_dict(doc, obj)?;
    let numbers = |key: &[u8]| raw_dict_get(dict, key).and_then(|v| raw_numbers(doc, v));
    let domain = numbers(b"Domain")?;
    let range = numbers(b"Range");
    if domain.is_empty()
        || domain.len() % 2 != 0
        || domain.len() > 64
        || range
            .as_ref()
            .is_some_and(|r| r.is_empty() || r.len() % 2 != 0 || r.len() > 64)
    {
        return None;
    }
    let pair = |values: &[f32]| -> Option<[f32; 2]> { values.get(..2)?.try_into().ok() };
    match raw_dict_get(dict, b"FunctionType").and_then(|t| doc.resolve(t).as_i64())? {
        0 => {
            let order = raw_dict_get(dict, b"Order")
                .map(|o| doc.resolve(o).as_i64())
                .unwrap_or(Some(1))?;
            if order != 1 {
                return None;
            }
            let range = range?;
            let size: Vec<u32> = numbers(b"Size")?.iter().map(|s| *s as u32).collect();
            let bits_per_sample =
                raw_dict_get(dict, b"BitsPerSample").and_then(|b| doc.resolve(b).as_i64())? as u32;
            if size.is_empty()
                || crate::model::sampled_corner_count(&size).is_none()
                || size.len() * 2 != domain.len()
                || ![1, 2, 4, 8, 12, 16, 24, 32].contains(&bits_per_sample)
            {
                return None;
            }
            let samples = raw_stream_data(doc, obj)?;
            let count = size
                .iter()
                .try_fold(1usize, |n, s| n.checked_mul(*s as usize))?;
            let required_bits = count
                .checked_mul(range.len() / 2)?
                .checked_mul(bits_per_sample as usize)?;
            if samples.len().checked_mul(8)? < required_bits {
                return None;
            }
            Some(Function::Sampled {
                encode: numbers(b"Encode").unwrap_or_else(|| {
                    size.iter().flat_map(|s| [0.0, (*s as f32 - 1.0)]).collect()
                }),
                decode: numbers(b"Decode").unwrap_or_else(|| range.clone()),
                domain,
                range,
                size,
                bits_per_sample,
                samples,
            })
        }
        2 => {
            let c0 = numbers(b"C0").unwrap_or_else(|| vec![0.0]);
            let c1 = numbers(b"C1").unwrap_or_else(|| vec![1.0]);
            if c0.is_empty() || c0.len() > 32 || c0.len() != c1.len() {
                return None;
            }
            Some(Function::Exponential {
                domain: pair(&domain)?,
                c0,
                c1,
                exponent: raw_dict_get(dict, b"N").and_then(|n| doc.resolve(n).as_f32())?,
                range,
            })
        }

        3 => {
            let functions = raw_dict_get(dict, b"Functions")
                .and_then(|f| doc.resolve(f).as_array())?
                .iter()
                .map(|f| resolve_function(doc, f, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            let bounds = numbers(b"Bounds")?;
            let encode = numbers(b"Encode")?;
            if functions.is_empty()
                || functions
                    .iter()
                    .any(|f| f.outputs() != functions[0].outputs())
                || bounds.len() + 1 != functions.len()
                || encode.len() != 2 * functions.len()
            {
                return None;
            }
            Some(Function::Stitching {
                domain: pair(&domain)?,
                bounds,
                encode,
                functions,
                range,
            })
        }
        4 => Some(Function::Calculator {
            program: parse_calculator(&raw_stream_data(doc, obj)?)?,
            domain,
            range: range?,
        }),
        _ => None,
    }
}

/// Extract a string value from a raw PDF dictionary.
fn raw_get_string(doc: &RawDocument, dict: &RawPdfDict, key: &[u8]) -> Option<String> {
    let obj = raw_dict_get(dict, key)?;
    let obj = doc.resolve(obj);
    match obj {
        RawPdfObject::Str(bytes) => {
            if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
                // UTF-16BE with BOM
                let utf16: Vec<u16> = bytes[2..]
                    .chunks(2)
                    .filter_map(|c| {
                        if c.len() == 2 {
                            Some(u16::from_be_bytes([c[0], c[1]]))
                        } else {
                            None
                        }
                    })
                    .collect();
                String::from_utf16(&utf16).ok()
            } else {
                String::from_utf8(bytes.clone())
                    .ok()
                    .or_else(|| Some(bytes.iter().map(|&b| b as char).collect()))
            }
        }
        RawPdfObject::Name(bytes) => String::from_utf8(bytes.clone()).ok(),
        _ => None,
    }
}

/// Extract (width, height) from a MediaBox array.
fn extract_dimensions_from_array(obj: &RawPdfObject) -> Option<(f32, f32)> {
    let arr = obj.as_array()?;
    if arr.len() >= 4 {
        let width = arr[2].as_f32().unwrap_or(612.0);
        let height = arr[3].as_f32().unwrap_or(792.0);
        Some((width, height))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_backend(font: &str, extra_objects: &[String]) -> RawBackend {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> >>".to_string(),
            font.to_string(),
        ];
        objects.extend_from_slice(extra_objects);
        backend_from_objects(&objects)
    }

    fn backend_from_objects(objects: &[String]) -> RawBackend {
        backend_from_bytes(
            &objects
                .iter()
                .map(|s| s.as_bytes().to_vec())
                .collect::<Vec<_>>(),
        )
    }

    fn backend_from_bytes(objects: &[Vec<u8>]) -> RawBackend {
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(object);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        RawBackend::load_bytes(&pdf).unwrap()
    }

    fn stream_object(dict: &str, data: &str) -> String {
        format!(
            "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
            data.len()
        )
    }

    fn xobject_names(backend: &RawBackend, page: PageId) -> Vec<String> {
        let mut names: Vec<String> = backend
            .page_xobjects(page)
            .unwrap()
            .objects
            .into_iter()
            .map(|x| x.name)
            .collect();
        names.sort();
        names
    }

    #[test]
    fn page_xobjects_are_the_ones_its_content_paints() {
        let image = "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8";
        let objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 5 0 R /Contents 9 0 R >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 5 0 R /Contents 10 0 R >>".to_string(),
            "<< /XObject << /Im1 6 0 R /Im2 7 0 R /Fm1 8 0 R /Unused 11 0 R >> >>".to_string(),
            stream_object(image, "a"),
            stream_object(image, "b"),
            stream_object(
                "/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Inner 12 0 R >> >>",
                "q /Inner Do Q",
            ),
            stream_object("", "q 612 0 0 792 0 0 cm /Im1 Do Q"),
            stream_object("", "q /Im2 Do Q /Fm1 Do /Im2 Do"),
            stream_object(image, "c"),
            stream_object(image, "d"),
        ];
        let backend = backend_from_objects(&objects);
        assert_eq!(xobject_names(&backend, (3, 0)), vec!["object_6_0"]);
        assert_eq!(
            xobject_names(&backend, (4, 0)),
            vec!["object_12_0", "object_7_0", "object_8_0"]
        );
    }

    const TEST_IMAGE: &str = "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8";

    fn painted_pdf(resources: &str, content: &str, extra: Vec<String>) -> RawBackend {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << {resources} >> /Contents 4 0 R >>"),
            stream_object("", content),
        ];
        objects.extend(extra);
        backend_from_objects(&objects)
    }

    fn painted_images(backend: &RawBackend) -> Vec<RawXObject> {
        backend
            .page_xobjects((3, 0))
            .unwrap()
            .objects
            .into_iter()
            .filter(|object| object.subtype == "Image")
            .collect()
    }

    #[test]
    fn painted_form_images_have_distinct_resource_identifiers() {
        let backend = painted_pdf(
            "/XObject << /Fm_Im 5 0 R /Fm 6 0 R >>",
            "/Fm_Im Do /Fm Do",
            vec![
                stream_object(TEST_IMAGE, "a"),
                stream_object(
                    "/Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Im 7 0 R >> >>",
                    "/Im Do",
                ),
                stream_object(TEST_IMAGE, "b"),
            ],
        );
        let images = painted_images(&backend);
        assert_eq!(images.len(), 2);
        assert_ne!(images[0].name, images[1].name);
        assert_eq!(images[0].data, b"a");
        assert_eq!(images[1].data, b"b");
    }

    #[test]
    fn painted_tiling_patterns_reach_images_without_page_xobjects() {
        for operator in ["scn", "SCN"] {
            let color_space = if operator == "scn" { "cs" } else { "CS" };
            let backend = painted_pdf("/Pattern << /P 5 0 R /Unused 7 0 R >>",
                &format!("/Pattern {color_space} /P {operator} 0 0 10 10 re B"), vec![
                stream_object("/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 1 1] /XStep 1 /YStep 1 /Resources << /XObject << /Im 6 0 R >> >>", "/Im Do"),
                stream_object(TEST_IMAGE, "a"),
                stream_object("/PatternType 1 /Resources << /XObject << /Im 8 0 R >> >>", "/Im Do"),
                stream_object(TEST_IMAGE, "b"),
            ]);
            let images = painted_images(&backend);
            assert_eq!(images.len(), 1, "{operator}");
            assert_eq!(images[0].data, b"a");
        }
    }

    #[test]
    fn selected_unused_pattern_does_not_contribute_images_or_placements() {
        let image = "/Type /XObject /Subtype /Image /Width 100 /Height 100 /ColorSpace /DeviceGray /BitsPerComponent 8";
        let backend = painted_pdf(
            "/XObject << /A 5 0 R >> /Pattern << /P 6 0 R >>",
            "q /Pattern cs /P scn Q q 100 0 0 100 0 0 cm /A Do Q",
            vec![
                stream_object(image, &"a".repeat(10000)),
                stream_object("/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 100 100] /XStep 100 /YStep 100 /Resources << /XObject << /B 7 0 R >> >>", "/B Do"),
                stream_object(image, &"b".repeat(10000)),
            ],
        );
        let result = backend.page_xobjects((3, 0)).unwrap();
        assert_eq!(result.objects.len(), 1);
        assert_eq!(result.objects[0].name, "object_5_0");
        assert_eq!(result.paint_count, 1);
    }

    #[test]
    fn patterns_paint_only_with_their_fill_or_stroke_operations() {
        for (select, paint, expected) in [
            ("/Pattern cs /P scn", "f", 1),
            ("/Pattern cs /P scn", "F", 1),
            ("/Pattern cs /P scn", "f*", 1),
            ("/Pattern cs /P scn", "S", 0),
            ("/Pattern cs /P scn", "s", 0),
            ("/Pattern CS /P SCN", "S", 1),
            ("/Pattern CS /P SCN", "s", 1),
            ("/Pattern CS /P SCN", "f", 0),
            ("/Pattern CS /P SCN", "f*", 0),
            ("/Pattern cs /P scn", "B", 1),
            ("/Pattern CS /P SCN", "B*", 1),
            ("/Pattern cs /P scn", "b", 1),
            ("/Pattern CS /P SCN", "b*", 1),
            ("/Pattern cs /P scn", "n", 0),
            ("q /Pattern cs /P scn Q", "f", 0),
            ("q /Pattern CS /P SCN Q", "S", 0),
            ("/Pattern cs /P scn q 0 g Q", "f", 1),
            ("/Pattern CS /P SCN q 0 G Q", "S", 1),
            ("/Pattern cs /P scn /DeviceGray cs", "f", 0),
            ("/Pattern CS /P SCN /DeviceGray CS", "S", 0),
            ("/Pattern cs /P scn 0 g", "f", 0),
            ("/Pattern CS /P SCN 0 G", "S", 0),
            ("/Pattern cs /P scn 0 0 0 rg", "f", 0),
            ("/Pattern CS /P SCN 0 0 0 RG", "S", 0),
            ("/Pattern cs /P scn 0 0 0 1 k", "f", 0),
            ("/Pattern CS /P SCN 0 0 0 1 K", "S", 0),
        ] {
            let backend = painted_pdf("/Pattern << /P 5 0 R >>",
                &format!("{select} 0 0 10 10 re {paint}"), vec![
                stream_object("/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 1 1] /XStep 1 /YStep 1 /Resources << /XObject << /Im 6 0 R >> >>", "/Im Do"),
                stream_object(TEST_IMAGE, "a"),
            ]);
            let result = backend.page_xobjects((3, 0)).unwrap();
            assert_eq!(result.objects.len(), expected, "{select}; {paint}");
            assert_eq!(result.paint_count, (expected * 2) as u32, "{select}; {paint}");
        }
    }

    #[test]
    fn forms_inherit_patterns_without_leaking_their_graphics_state() {
        let backend = painted_pdf("/Pattern << /P 5 0 R /Other 7 0 R >> /XObject << /Fm 9 0 R >>",
            "/Pattern cs /P scn /Fm Do /Other scn /Fm Do", vec![
            stream_object("/PatternType 1 /Resources << /XObject << /Im 6 0 R >> >>", "/Im Do"),
            stream_object(TEST_IMAGE, "a"),
            stream_object("/PatternType 1 /Resources << /XObject << /Im 8 0 R >> >>", "/Im Do"),
            stream_object(TEST_IMAGE, "b"),
            stream_object("/Subtype /Form /BBox [0 0 10 10]", "0 0 10 10 re f 0 g"),
        ]);
        let images = painted_images(&backend);
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].data, b"a");
        assert_eq!(images[1].data, b"b");
    }

    #[test]
    fn text_uses_patterns_only_for_painted_rendering_modes() {
        for (mode, fill, stroke) in [(0, true, false), (1, false, true), (2, true, true),
            (3, false, false), (4, true, false), (5, false, true), (6, true, true), (7, false, false)] {
            let backend = painted_pdf("/Pattern << /P 5 0 R /Other 7 0 R >> /Font << /F 9 0 R >>",
                &format!("/Pattern cs /P scn /Pattern CS /Other SCN BT /F 12 Tf {mode} Tr (A) Tj ET"), vec![
                stream_object("/PatternType 1 /Resources << /XObject << /Im 6 0 R >> >>", "/Im Do"),
                stream_object(TEST_IMAGE, "a"),
                stream_object("/PatternType 1 /Resources << /XObject << /Im 8 0 R >> >>", "/Im Do"),
                stream_object(TEST_IMAGE, "b"),
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
            ]);
            let images = painted_images(&backend);
            assert_eq!(images.iter().any(|image| image.data == b"a"), fill, "mode {mode}");
            assert_eq!(images.iter().any(|image| image.data == b"b"), stroke, "mode {mode}");
        }
    }

    #[test]
    fn stencil_placements_use_the_current_fill_pattern() {
        for paint in ["/Mask Do", "BI /W 8 /H 1 /IM true ID \0 EI", "BI /Width 8 /Height 1 /ImageMask true ID \0 EI"] {
            let backend = painted_pdf("/Pattern << /P 5 0 R /Other 7 0 R >> /XObject << /Mask 9 0 R >>",
                &format!("/Pattern cs /P scn {paint} /Other scn {paint}"), vec![
                stream_object("/PatternType 1 /Resources << /XObject << /Im 6 0 R >> >>", "/Im Do"),
                stream_object(TEST_IMAGE, "a"),
                stream_object("/PatternType 1 /Resources << /XObject << /Im 8 0 R >> >>", "/Im Do"),
                stream_object(TEST_IMAGE, "b"),
                stream_object("/Type /XObject /Subtype /Image /Width 8 /Height 1 /ImageMask true", "\0"),
            ]);
            let images = painted_images(&backend);
            assert!(images.iter().any(|image| image.data == b"a"), "{paint:?}");
            assert!(images.iter().any(|image| image.data == b"b"), "{paint:?}");
            assert_eq!(backend.image_decodes.load(std::sync::atomic::Ordering::Relaxed),
                if paint == "/Mask Do" { 3 } else { 2 });
        }
    }

    #[test]
    fn type3_charprocs_paint_independently_of_text_rendering_mode() {
        for mode in [3, 7] {
            let backend = painted_pdf("/Font << /F 5 0 R >>",
                &format!("BT /F 12 Tf {mode} Tr (A) Tj ET"), vec![
                "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [1 0 0 1 0 0] /Encoding << /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [1] /CharProcs << /A 6 0 R >> /Resources << /XObject << /Im 7 0 R >> >> >>".into(),
                stream_object("", "1 0 d0 /Im Do"),
                stream_object(TEST_IMAGE, "a"),
            ]);
            assert_eq!(painted_images(&backend).len(), 1, "mode {mode}");
        }
    }

    #[test]
    fn type3_glyph_colors_determine_whether_the_selected_pattern_paints() {
        let backend = painted_pdf("/Font << /F 5 0 R >> /Pattern << /P 8 0 R >>",
            "/Pattern cs /P scn BT /F 12 Tf (A) Tj ET", vec![
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [1 0 0 1 0 0] /Encoding << /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [1] /CharProcs << /A 6 0 R >> /Resources << /XObject << /Im 7 0 R >> >> >>".into(),
            stream_object("", "1 0 d0 0 g /Im Do"),
            stream_object(TEST_IMAGE, "a"),
            stream_object("/PatternType 1 /Resources << /XObject << /Im 9 0 R >> >>", "/Im Do"),
            stream_object(TEST_IMAGE, "b"),
        ]);
        let images = painted_images(&backend);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].data, b"a");
    }

    #[test]
    fn painted_soft_mask_forms_reach_images() {
        let backend = painted_pdf(
            "/ExtGState << /Mask 5 0 R >>",
            "/Mask gs 0 0 10 10 re f",
            vec![
                "<< /SMask << /S /Luminosity /G 6 0 R >> >>".into(),
                stream_object(
                    "/Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Im 7 0 R >> >>",
                    "/Im Do",
                ),
                stream_object(TEST_IMAGE, "a"),
            ],
        );
        assert_eq!(painted_images(&backend).len(), 1);
    }

    #[test]
    fn painted_type3_glyphs_reach_images() {
        let backend = painted_pdf("/Font << /F 5 0 R >>", "BT /F 12 Tf (A) Tj ET", vec![
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [1 0 0 1 0 0] /Encoding << /Differences [65 /A /B] >> /FirstChar 65 /LastChar 66 /Widths [1 1] /CharProcs << /A 6 0 R /B 8 0 R >> /Resources << /XObject << /Im 7 0 R /Unused 9 0 R >> >> >>".into(),
            stream_object("", "1 0 d0 /Im Do"),
            stream_object(TEST_IMAGE, "a"),
            stream_object("", "1 0 d0 /Unused Do"),
            stream_object(TEST_IMAGE, "b"),
        ]);
        let images = painted_images(&backend);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].data, b"a");
    }

    #[test]
    fn inline_samples_do_not_hide_painted_xobjects() {
        for (dict, samples) in [
            ("/W 8 /H 1 /BPC 8 /CS /G", " EI (abc"),
            (
                "/Width 8 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray",
                " EI (abc",
            ),
            ("/W 21 /H 1 /BPC 1 /CS /RGB", " EI (abc"),
            ("/W 64 /H 1 /IM true", " EI (abc"),
            ("/W 8 /H 1 /BPC 8 /CS /G /F /AHx", "2045492028616263>"),
        ] {
            let backend = painted_pdf(
                "/XObject << /Im 5 0 R >>",
                &format!("BI {dict} ID {samples} EI /Im Do"),
                vec![stream_object(TEST_IMAGE, "z")],
            );
            assert_eq!(painted_images(&backend).len(), 1, "{dict}");
        }
    }

    #[test]
    fn painted_forms_have_no_fixed_nesting_limit_and_stop_cycles() {
        let depth = 96;
        let mut extra = Vec::new();
        for n in 5..5 + depth {
            extra.push(stream_object(&format!("/Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Next {} 0 R /Cycle 5 0 R >> >>", n + 1), "/Next Do /Cycle Do"));
        }
        extra.push(stream_object(TEST_IMAGE, "z"));
        let backend = painted_pdf("/XObject << /First 5 0 R >>", "/First Do", extra);
        assert_eq!(painted_images(&backend).len(), 1);
    }

    #[test]
    fn painted_type3_base_encoding_uses_glyph_names_not_unicode_aliases() {
        let backend = painted_pdf("/Font << /F 5 0 R >>", "BT /F 12 Tf (A) Tj ET", vec![
            "<< /Type /Font /Subtype /Type3 /Encoding /StandardEncoding /CharProcs << /A 6 0 R /uni0041 8 0 R >> /Resources << /XObject << /Im 7 0 R /Alias 9 0 R >> >> >>".into(),
            stream_object("", "1 0 d0 /Im Do"),
            stream_object(TEST_IMAGE, "a"),
            stream_object("", "1 0 d0 /Alias Do"),
            stream_object(TEST_IMAGE, "b"),
        ]);
        // Resource dictionaries use randomized hash order.
        for _ in 0..32 {
            let images = painted_images(&backend);
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].data, b"a");
        }
    }

    #[test]
    fn painted_annotation_appearance_reaches_images() {
        let objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Annots [4 0 R] >>".into(),
            "<< /Type /Annot /Subtype /Stamp /Rect [0 0 10 10] /AP << /N 5 0 R >> >>".into(),
            stream_object(
                "/Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Im 6 0 R >> >>",
                "/Im Do",
            ),
            stream_object(TEST_IMAGE, "z"),
        ];
        assert_eq!(painted_images(&backend_from_objects(&objects)).len(), 1);
    }

    #[test]
    fn inline_named_colors_keep_the_following_image_and_text() {
        let backend = painted_pdf(
            "/ColorSpace << /Custom /DeviceGray >> /XObject << /Im 5 0 R >>",
            "BI /W 8 /H 1 /BPC 8 /CS /Custom ID  EI (abc EI /Im Do BT (visible) Tj ET",
            vec![stream_object(TEST_IMAGE, "z")],
        );
        assert_eq!(painted_images(&backend).len(), 1);
        let ops = backend
            .decode_page_content((3, 0), &backend.page_content((3, 0)).unwrap())
            .unwrap();
        assert!(ops.iter().any(|op| op.operator == "Tj"));
    }

    #[test]
    fn repeated_image_placements_require_composite_rendering_without_redecoding() {
        let backend = painted_pdf(
            "/XObject << /I 5 0 R /J 5 0 R >>",
            "q /I Do Q q 1 0 0 1 4 0 cm /J Do Q",
            vec![stream_object(TEST_IMAGE, "z")],
        );
        let page = super::super::pdf_parser::parse_single_page(
            &backend, 1, &super::super::ParseOptions::new().with_resources(true),
        ).unwrap();
        assert_eq!(page.image_resource_count, 2);
        assert_eq!(page.extracted_resources.len(), 1);
        assert_eq!(backend.image_decodes.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn shared_scan_resources_decode_once_per_painted_image_per_page() {
        use std::sync::atomic::Ordering;
        let pages = 8;
        let images = 32;
        let resources = 3 + pages;
        let first_image = resources + 1;
        let first_content = first_image + images;
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {pages} >>",
                (3..resources)
                    .map(|n| format!("{n} 0 R"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        ];
        for page in 0..pages {
            objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources {resources} 0 R /Contents {} 0 R >>", first_content + page));
        }
        objects.push(format!(
            "<< /XObject << {} >> >>",
            (0..images)
                .map(|n| format!("/Im{n} {} 0 R", first_image + n))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        for _ in 0..images {
            objects.push(stream_object(TEST_IMAGE, "z"));
        }
        for page in 0..pages {
            objects.push(stream_object("", &format!("/Im{page} Do /Im{page} Do")));
        }
        let backend = backend_from_objects(&objects);
        for page in 0..pages {
            let before = backend.image_decodes.load(Ordering::Relaxed);
            let extracted = backend.page_xobjects(((3 + page) as u32, 0)).unwrap();
            assert_eq!(extracted.objects.len(), 1);
            assert_eq!(extracted.paint_count, 2);
            assert_eq!(extracted.objects[0].data, b"z");
            assert_eq!(backend.image_decodes.load(Ordering::Relaxed) - before, 1);
        }
    }

    #[test]
    fn filtered_inline_data_keeps_following_painted_xobjects() {
        use std::io::Write;
        let mut flate = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
        flate.write_all(b" EI (abc").unwrap();
        let flate = flate.finish().unwrap();
        for (filter, samples) in [
            ("/Fl", flate.clone()),
            (
                "[/AHx /Fl]",
                [
                    flate
                        .iter()
                        .flat_map(|byte| format!("{byte:02x}").into_bytes())
                        .collect::<Vec<_>>(),
                    b">".to_vec(),
                ]
                .concat(),
            ),
            ("/A85", b"zz~>".to_vec()),
            ("/RL", [vec![7], b" EI (abc".to_vec(), vec![128]].concat()),
            // clear, one sample, EOD, padded to a byte boundary
            ("/LZW", vec![128, 0, 32, 32]),
            // One all-white Group 4 row, followed by EOFB.
            (
                "/CCF /DP << /K -1 /Columns 8 /Rows 1 >>",
                vec![0x80, 0x08, 0x00, 0x80],
            ),
        ] {
            let mut content = format!("BI /W 8 /H 1 /BPC 8 /CS /G /F {filter} ID ").into_bytes();
            content.extend_from_slice(&samples);
            content.extend_from_slice(b" EI /Im Do");
            let stream = [
                format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
                content,
                b"\nendstream".to_vec(),
            ]
            .concat();
            let objects = vec![
                b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
                stream,
                stream_object(TEST_IMAGE, "z").into_bytes(),
            ];
            assert_eq!(
                painted_images(&backend_from_bytes(&objects)).len(),
                1,
                "{filter}"
            );
        }
    }

    #[test]
    fn devicen_none_colorants_do_not_paint() {
        let backend = text_backend("<< >>", &["/None".into()]);
        for (names, paints) in [
            ("/None /None", false),
            ("/#4Eone 5 0 R", false),
            ("/None /Spot", true),
            ("/Spot /None", true),
            ("/Cyan /Magenta", true),
        ] {
            let source = format!("[/DeviceN [{names}] /DeviceGray << /FunctionType 2 /Domain [0 1 0 1] /C0 [0] /C1 [1] /N 1 >>]");
            let (object, _) =
                super::super::raw::tokenizer::parse_object(source.as_bytes(), 0).unwrap();
            let space = resolve_color_space(&backend.doc, &object, None, 0).unwrap();
            assert_eq!(space.components(), 2);
            assert_eq!(space.paints(), paints, "{names}");
            let mut stored = serde_json::to_value(&space).unwrap();
            stored["DeviceN"].as_object_mut().unwrap().remove("names");
            let without_names: ColorSpace = serde_json::from_value(stored).unwrap();
            assert_eq!(without_names.components(), 2);
            assert!(without_names.paints());
            if paints {
                assert_eq!(space.to_rgb(&[0.5, 0.5]), Some([0.5; 3]), "{names}");
            }
        }
    }

    fn tounicode_backend(cmap: &str) -> RawBackend {
        text_backend(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 5 0 R >>",
            &[format!(
                "<< /Length {} >>\nstream\n{cmap}\nendstream",
                cmap.len()
            )],
        )
    }

    #[test]
    fn test_parse_to_unicode_cmap_drops_nul_mapping() {
        let cmap = "1 begincodespacerange
<00> <FF>
endcodespacerange
2 beginbfchar
<01> <0000>
<02> <00000041>
endbfchar
1 beginbfrange
<20> <7E> <0020>
endbfrange";
        let backend = tounicode_backend(cmap);
        assert_eq!(backend.decode_text((3, 0), b"F1", b"\x01"), "");
        assert_eq!(backend.decode_text((3, 0), b"F1", b"\x02"), "A");
        assert_eq!(backend.decode_text((3, 0), b"F1", b"of\x01ce"), "ofce");
    }

    #[test]
    fn test_decode_text_nul_tounicode_run_boundaries() {
        for mappings in [
            "2 beginbfchar <41> <0000> <42> <0042> endbfchar",
            "1 beginbfrange <41> <42> [<0000> <0042>] endbfrange",
            "1 beginbfrange <41> <41> <0000> endbfrange 1 beginbfchar <42> <0042> endbfchar",
        ] {
            let backend = tounicode_backend(&format!(
                "1 begincodespacerange <00> <FF> endcodespacerange {mappings}"
            ));
            for (bytes, expected) in [(b"A".as_slice(), ""), (b"AB", "B"), (b"BA", "B")] {
                assert_eq!(backend.decode_text((3, 0), b"F1", bytes), expected);
            }
            // An absent mapping still uses the existing encoding fallback.
            assert_eq!(backend.decode_text((3, 0), b"F1", b"C"), "C");
        }
        let backend = tounicode_backend(
            "1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfchar <41> <0000> endbfchar",
        );
        assert_eq!(backend.decode_text((3, 0), b"F1", b"A"), "");
    }

    #[test]
    fn test_decode_text_nul_keeps_two_byte_width() {
        let backend = tounicode_backend("1 begincodespacerange <0000> <FFFF> endcodespacerange 2 beginbfchar <0001> <0041> <0100> <0000> endbfchar");
        assert_eq!(backend.decode_text((3, 0), b"F1", &[1, 0]), "");
        assert_eq!(backend.decode_text((3, 0), b"F1", &[0, 1, 1, 0]), "A");
    }

    #[test]
    fn test_decode_text_nul_embedded_cmap() {
        // Minimal TrueType table with format 12 groups: U+0000 -> GID 1, A -> GID 2.
        let mut font = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        font.extend_from_slice(b"cmap");
        for value in [0u32, 28, 52] {
            font.extend_from_slice(&value.to_be_bytes());
        }
        font.extend_from_slice(&[0, 0, 0, 1, 0, 3, 0, 1]);
        font.extend_from_slice(&12u32.to_be_bytes());
        font.extend_from_slice(&[0, 12, 0, 0]);
        for value in [40u32, 0, 2, 0, 0, 1, 65, 65, 2] {
            font.extend_from_slice(&value.to_be_bytes());
        }
        let hex: String = font.iter().map(|b| format!("{b:02X}")).collect();
        let backend = text_backend(
            "<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [5 0 R] >>",
            &[
                "<< /Type /Font /Subtype /CIDFontType2 /FontDescriptor 6 0 R /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 0 >> >>".into(),
                "<< /Type /FontDescriptor /FontFile2 7 0 R >>".into(),
                format!("<< /Length {} /Filter /ASCIIHexDecode >>\nstream\n{hex}>\nendstream", hex.len() + 1),
            ],
        );
        assert_eq!(backend.decode_text((3, 0), b"F1", &[0, 1]), "");
        assert_eq!(
            backend.decode_text((3, 0), b"F1", &[0, 2, 0, 1, 0, 2]),
            "AA"
        );
    }

    #[test]
    fn test_decode_text_nul_preserves_adjacent_scalars() {
        let backend = tounicode_backend("1 begincodespacerange <00> <FF> endcodespacerange 3 beginbfchar <41> <00000041> <42> <000000660069> <43> <0000D83DDE00> endbfchar");
        assert_eq!(backend.decode_text((3, 0), b"F1", b"ABC"), "Afi😀");
    }

    #[test]
    fn test_decode_text_nul_differences() {
        for glyph in ["uni0000", "u0000"] {
            let backend = text_backend(&format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /{glyph}] >> >>"), &[]);
            assert_eq!(backend.decode_text((3, 0), b"F1", b"A"), "");
            assert_eq!(backend.decode_text((3, 0), b"F1", b"BAB"), "BB");
        }
    }

    #[test]
    fn test_decode_text_nul_direct_strings() {
        let backend = text_backend(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            &[],
        );
        for (bytes, expected) in [
            (b"Hello\0 world\n\t".as_slice(), "Hello world\n\t"),
            (b"Hell\xE9\0 world", "Hellé world"),
        ] {
            assert_eq!(backend.decode_text((3, 0), b"MissingFont", bytes), expected);
        }
        let utf16: Vec<u8> = std::iter::once(0xFEFFu16)
            .chain("Hello\0 world".encode_utf16())
            .flat_map(u16::to_be_bytes)
            .collect();
        assert_eq!(
            backend.decode_text((3, 0), b"MissingFont", &utf16),
            "Hello world"
        );
    }

    #[test]
    fn test_get_number_from_value() {
        assert_eq!(get_number_from_value(&PdfValue::Integer(42)), Some(42.0));
        assert_eq!(
            get_number_from_value(&PdfValue::Real(std::f32::consts::PI)),
            Some(std::f32::consts::PI)
        );
        assert_eq!(get_number_from_value(&PdfValue::Other), None);
    }
}

#[cfg(test)]
mod raw_backend_tests {
    use super::*;
    use std::path::Path;

    /// Skip the test if the PDF fixture (gitignored under `test-files/`)
    /// is unavailable, e.g., on CI. Returns the loaded backend or `None`.
    fn try_load(rel: &str) -> Option<RawBackend> {
        if !Path::new(rel).exists() {
            eprintln!("skipping: fixture not present at {}", rel);
            return None;
        }
        RawBackend::load_file(rel).ok()
    }

    #[test]
    fn test_raw_backend_pages() {
        let Some(raw) = try_load("test-files/basic/trivial.pdf") else {
            return;
        };
        let pages = raw.pages();
        assert!(!pages.is_empty());
    }

    #[test]
    fn test_raw_backend_page_content() {
        let Some(raw) = try_load("test-files/basic/trivial.pdf") else {
            return;
        };
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let content = raw.page_content(first_page).unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn test_raw_backend_decode_content() {
        let Some(raw) = try_load("test-files/basic/trivial.pdf") else {
            return;
        };
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let content = raw.page_content(first_page).unwrap();
        let ops = raw.decode_content(&content).unwrap();
        assert!(!ops.is_empty());
    }

    #[test]
    fn test_raw_backend_metadata() {
        let Some(raw) = try_load("test-files/basic/trivial.pdf") else {
            return;
        };
        let meta = raw.metadata();
        assert!(!meta.version.is_empty());
    }

    #[test]
    fn test_raw_backend_page_dimensions() {
        let Some(raw) = try_load("test-files/basic/trivial.pdf") else {
            return;
        };
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let (w, h) = raw.page_dimensions(first_page);
        assert!(w > 0.0 && h > 0.0);
    }

    #[test]
    fn test_raw_backend_korean_pages() {
        let Some(raw) = try_load("test-files/cjk/korean-test.pdf") else {
            return;
        };
        assert!(!raw.pages().is_empty());
    }

    #[test]
    fn test_iphone_korean_text_decode() {
        let Some(raw) = try_load("test-files/realworld/iphone-info.pdf") else {
            return;
        };
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let decoded = raw.decode_text(first_page, b"T1_1", &[31, 30, 29, 28, 27]);
        assert!(
            decoded.contains('사') && decoded.contains('서'),
            "Korean text should be decoded: got {:?}",
            decoded
        );
    }
}
