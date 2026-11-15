use hayro::hayro_interpret::font::FontQuery;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::{AlphaColor, Rgba8, Srgb};
use hayro::vello_cpu::Pixmap;
use hayro::{render, RenderCache, RenderSettings};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageEncoder, RgbImage};
use std::sync::Arc;

#[allow(clippy::too_many_arguments)]
mod bindings {
    wit_bindgen::generate!({
        world: "pdf2image",
    });
}

use bindings::{Guest, RenderError, RenderOptions, ScalingOptions};

struct Component;

const ERR_PARSE: u8 = 2;
const ERR_RANGE: u8 = 3;
const ERR_RENDER: u8 = 4;
const ERR_OPTIONS: u8 = 5;

const DEFAULT_DPI: f32 = 150.0;
const PDF_POINTS_PER_INCH: f32 = 72.0;
const DEFAULT_JPEG_QUALITY: u8 = 90;
// Per-page pixel ceiling: 64 MP (e.g. 8192x8192) ~ 256 MB rgba. page-size * scale
// stays under this.
const MAX_OUTPUT_PIXELS: u64 = 64 * 1024 * 1024;
// vello_common 0.0.8, tile.rs: Tile::WIDTH = 4. rect.rs includes the
// right-edge tile, then tests col + WIDTH once more. Both sums use u16:
// floor(d / WIDTH) * WIDTH + 2 * WIDTH <= u16::MAX. Thus d <= 65_527.
// Use the same bound for either orientation (tiles are square).
const RENDER_TILE_WIDTH: u16 = 4;
const MAX_OUTPUT_DIMENSION: f32 =
    ((u16::MAX / RENDER_TILE_WIDTH - 1) * RENDER_TILE_WIDTH - 1) as f32;
// Maximum pages rendered per request.
const MAX_RENDER_PAGES: usize = 1024;
const WHITE: AlphaColor<Srgb> = AlphaColor::from_rgba8(255, 255, 255, 255);

enum Format {
    Png,
    Jpeg,
    Webp,
}

struct Settings {
    scale: f32,
    format: Format,
    quality: u8,
    bg_color: AlphaColor<Srgb>,
}

fn fail(code: u8, message: impl Into<String>) -> RenderError {
    RenderError {
        code,
        message: message.into(),
    }
}

fn resolve_scale(scaling: &ScalingOptions) -> f32 {
    if scaling.scale.is_finite() && scaling.scale > 0.0 {
        scaling.scale
    } else if scaling.dpi.is_finite() && scaling.dpi > 0.0 {
        scaling.dpi / PDF_POINTS_PER_INCH
    } else {
        DEFAULT_DPI / PDF_POINTS_PER_INCH
    }
}

// Rendered pixel count for a `page_w` x `page_h` (points) page at `scale`; returns
// an error for non-finite, degenerate, or over-budget dimensions. saturating_mul
// bounds the multiply.
fn output_pixels(page_w: f32, page_h: f32, scale: f32) -> Result<u64, RenderError> {
    checked_pixels(page_w, page_h, scale).map_err(|message| fail(ERR_RANGE, message))
}

fn checked_pixels(page_w: f32, page_h: f32, scale: f32) -> Result<u64, &'static str> {
    let out_w = (page_w * scale).floor();
    let out_h = (page_h * scale).floor();
    if !out_w.is_finite() || !out_h.is_finite() || out_w < 1.0 || out_h < 1.0 {
        return Err("invalid render dimensions");
    }
    if out_w > MAX_OUTPUT_DIMENSION || out_h > MAX_OUTPUT_DIMENSION {
        return Err("render dimensions exceed the renderer limit");
    }
    let pixels = (out_w as u64).saturating_mul(out_h as u64);
    if pixels > MAX_OUTPUT_PIXELS {
        return Err("render dimensions exceed the pixel limit");
    }
    Ok(pixels)
}

// Scale for a `page_w` x `page_h` (points) page: the requested scale, lowered to
// a uniform fit within the pixel and renderer dimension limits. Integer output
// dimensions determine whether the requested scale already fits.
fn page_scale(page_w: f32, page_h: f32, requested: f32) -> Result<f32, RenderError> {
    if !page_w.is_finite() || !page_h.is_finite() || page_w <= 0.0 || page_h <= 0.0 {
        return Err(fail(ERR_RANGE, "invalid page dimensions"));
    }
    if checked_pixels(page_w, page_h, requested).is_ok() {
        return Ok(requested);
    }
    let width = f64::from(page_w);
    let height = f64::from(page_h);
    let fit = (MAX_OUTPUT_PIXELS as f64 / (width * height)).sqrt();
    let mut fit_scale = fit as f32;
    if f64::from(fit_scale) > fit {
        fit_scale = fit_scale.next_down();
    }
    // Dimension ratios round to the nearest f32: the renderer floors their
    // products, so rounding down here can unnecessarily lose a whole pixel.
    let scale = requested
        .min(fit_scale)
        .min(MAX_OUTPUT_DIMENSION / page_w)
        .min(MAX_OUTPUT_DIMENSION / page_h);
    output_pixels(page_w, page_h, scale).map(|_| scale)
}

fn parse_hex_pair(bytes: &[u8], index: usize) -> Result<u8, RenderError> {
    // saturating_add keeps the slice end within usize.
    let pair = bytes
        .get(index..index.saturating_add(2))
        .ok_or_else(|| fail(ERR_OPTIONS, "background color is too short"))?;
    let text = core::str::from_utf8(pair)
        .map_err(|_| fail(ERR_OPTIONS, "background color contains invalid characters"))?;
    u8::from_str_radix(text, 16).map_err(|_| {
        fail(
            ERR_OPTIONS,
            format!("invalid hex in background color '{text}'"),
        )
    })
}

fn parse_background(value: &str) -> Result<AlphaColor<Srgb>, RenderError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(WHITE);
    }

    if trimmed.eq_ignore_ascii_case("transparent") {
        return Ok(AlphaColor::TRANSPARENT);
    }

    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let bytes = hex.as_bytes();
    match bytes.len() {
        6 => Ok(AlphaColor::from_rgba8(
            parse_hex_pair(bytes, 0)?,
            parse_hex_pair(bytes, 2)?,
            parse_hex_pair(bytes, 4)?,
            255,
        )),
        8 => Ok(AlphaColor::from_rgba8(
            parse_hex_pair(bytes, 0)?,
            parse_hex_pair(bytes, 2)?,
            parse_hex_pair(bytes, 4)?,
            parse_hex_pair(bytes, 6)?,
        )),
        _ => Err(fail(
            ERR_OPTIONS,
            format!("background must be 'transparent' or a hex color, got '{value}'"),
        )),
    }
}

fn parse_format(value: &str) -> Result<Format, RenderError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "png" => Ok(Format::Png),
        "jpeg" | "jpg" => Ok(Format::Jpeg),
        "webp" => Ok(Format::Webp),
        other => Err(fail(
            ERR_OPTIONS,
            format!("unsupported format '{other}' (expected png, jpeg, or webp)"),
        )),
    }
}

impl TryFrom<RenderOptions> for Settings {
    type Error = RenderError;

    fn try_from(options: RenderOptions) -> Result<Self, Self::Error> {
        let format = parse_format(&options.output.format)?;
        let bg_color = parse_background(&options.output.background)?;
        let quality = match options.output.quality {
            0 => DEFAULT_JPEG_QUALITY,
            q => q.clamp(1, 100) as u8,
        };

        Ok(Self {
            scale: resolve_scale(&options.scaling),
            format,
            quality,
            bg_color,
        })
    }
}

fn load(blob: Vec<u8>) -> Result<Pdf, RenderError> {
    Pdf::new(blob).map_err(|e| fail(ERR_PARSE, format!("failed to parse PDF: {e:?}")))
}

fn font_settings() -> InterpreterSettings {
    InterpreterSettings {
        font_resolver: Arc::new(|query| match query {
            FontQuery::Standard(standard) => Some(standard.get_font_data()),
            FontQuery::Fallback(fallback) => Some(fallback.pick_standard_font().get_font_data()),
        }),
        ..Default::default()
    }
}

fn encode(pixmap: Pixmap, settings: &Settings) -> Result<Vec<u8>, RenderError> {
    match settings.format {
        Format::Png => pixmap
            .into_png()
            .map_err(|e| fail(ERR_RENDER, format!("failed to encode PNG: {e:?}"))),
        Format::Jpeg => {
            let width = pixmap.width() as u32;
            let height = pixmap.height() as u32;
            // Build the RGB buffer directly from the pixels in one pass. JPEG has no
            // alpha channel: with an opaque background every pixel is opaque, so RGB
            // is taken as-is; with a transparent or semi-transparent background each
            // pixel is composited over white.
            let pixels = pixmap.take_unpremultiplied();
            let mut rgb = Vec::with_capacity(pixels.len() * 3);
            if settings.bg_color.components[3] >= 1.0 {
                for p in &pixels {
                    rgb.push(p.r);
                    rgb.push(p.g);
                    rgb.push(p.b);
                }
            } else {
                for p in &pixels {
                    let a = p.a as u16;
                    let over = |c: u8| (((c as u16) * a + 255 * (255 - a)) / 255) as u8;
                    rgb.push(over(p.r));
                    rgb.push(over(p.g));
                    rgb.push(over(p.b));
                }
            }
            let img = RgbImage::from_raw(width, height, rgb)
                .ok_or_else(|| fail(ERR_RENDER, "failed to build image buffer"))?;
            let mut out = Vec::new();
            JpegEncoder::new_with_quality(&mut out, settings.quality)
                .encode_image(&img)
                .map_err(|e| fail(ERR_RENDER, format!("failed to encode JPEG: {e:?}")))?;
            Ok(out)
        }
        Format::Webp => {
            let width = pixmap.width() as u32;
            let height = pixmap.height() as u32;
            let rgba = bytemuck::cast_slice::<Rgba8, u8>(&pixmap.take_unpremultiplied()).to_vec();
            let mut out = Vec::new();
            WebPEncoder::new_lossless(&mut out)
                .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
                .map_err(|e| fail(ERR_RENDER, format!("failed to encode WEBP: {e:?}")))?;
            Ok(out)
        }
    }
}

impl Guest for Component {
    fn pages(blob: Vec<u8>) -> Result<u32, RenderError> {
        let pdf = load(blob)?;
        Ok(pdf.pages().len() as u32)
    }

    fn render(
        blob: Vec<u8>,
        pages: Vec<u32>,
        options: RenderOptions,
    ) -> Result<Vec<Vec<u8>>, RenderError> {
        let settings = Settings::try_from(options)?;
        let pdf = load(blob)?;

        let doc_pages = pdf.pages();
        let count = doc_pages.len() as u32;

        let selection: Vec<u32> = if pages.is_empty() {
            (0..count).collect()
        } else {
            pages
        };

        if selection.len() > MAX_RENDER_PAGES {
            return Err(fail(
                ERR_RANGE,
                format!(
                    "requested {} pages exceeds the limit of {MAX_RENDER_PAGES}",
                    selection.len()
                ),
            ));
        }

        let cache = RenderCache::new();
        let fonts = font_settings();
        let mut rendered = Vec::with_capacity(selection.len());

        for page in selection {
            if page >= count {
                return Err(fail(
                    ERR_RANGE,
                    format!("page index {page} out of range (document has {count} pages)"),
                ));
            }

            let target = doc_pages
                .get(page as usize)
                .ok_or_else(|| fail(ERR_RANGE, format!("page index {page} not found")))?;

            // hayro sizes the pixmap as page.render_dimensions() * scale.
            let (page_w, page_h) = target.render_dimensions();
            let scale = page_scale(page_w, page_h, settings.scale)
                .map_err(|e| fail(e.code, format!("page {page}: {}", e.message)))?;

            let render_settings = RenderSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: settings.bg_color,
                ..Default::default()
            };

            let pixmap = render(target, &cache, &fonts, &render_settings);
            let bytes = encode(pixmap, &settings)?;
            rendered.push(bytes);
        }

        Ok(rendered)
    }
}

bindings::export!(Component with_types_in bindings);

#[cfg(test)]
mod tests {
    use super::*;

    fn colorant_page(page: usize) -> Vec<u8> {
        let pdf = load(include_bytes!("../tests/assets/colorant-images.pdf").to_vec()).unwrap();
        render(
            &pdf.pages()[page],
            &RenderCache::new(),
            &font_settings(),
            &RenderSettings::default(),
        )
        .into_png()
        .unwrap()
    }

    #[test]
    fn separation_none_image_preserves_scanned_pixels() {
        let scan = colorant_page(0);
        assert_ne!(scan, colorant_page(5), "scan differs from opaque gray");
        assert_eq!(colorant_page(1), scan);
    }

    #[test]
    fn devicen_all_none_image_preserves_scanned_pixels() {
        assert_eq!(colorant_page(3), colorant_page(0));
    }

    #[test]
    fn separation_real_colorant_image_paints() {
        assert_eq!(colorant_page(2), colorant_page(5));
        assert_ne!(colorant_page(2), colorant_page(0));
    }

    #[test]
    fn devicen_mixed_colorants_image_paints() {
        assert_eq!(colorant_page(4), colorant_page(5));
        assert_ne!(colorant_page(4), colorant_page(0));
    }

    #[test]
    fn separation_none_inline_image_preserves_scanned_pixels() {
        assert_eq!(colorant_page(6), colorant_page(0));
    }

    #[test]
    fn devicen_all_none_inline_image_preserves_scanned_pixels() {
        assert_eq!(colorant_page(8), colorant_page(0));
    }

    #[test]
    fn separation_real_colorant_inline_image_paints() {
        assert_eq!(colorant_page(7), colorant_page(5));
        assert_ne!(colorant_page(7), colorant_page(0));
    }

    #[test]
    fn devicen_mixed_colorants_inline_image_paints() {
        assert_eq!(colorant_page(9), colorant_page(5));
        assert_ne!(colorant_page(9), colorant_page(0));
    }

    fn scaling(scale: f32, dpi: f32) -> ScalingOptions {
        ScalingOptions { dpi, scale }
    }

    // Hot-path microbench: render dominates, encode is the part we own. Run:
    //   cargo test --release bench_render_and_encode -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_render_and_encode() {
        use std::time::Instant;
        let blob = include_bytes!("../tests/assets/sample.pdf").to_vec();
        let pdf = load(blob).expect("load sample.pdf");
        let doc_pages = pdf.pages();
        let page = doc_pages.get(0).expect("a page");
        let cache = RenderCache::new();
        let fonts = font_settings();
        let scale = DEFAULT_DPI / PDF_POINTS_PER_INCH;
        let rs = RenderSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
            ..Default::default()
        };
        let iters = 100u32;

        let t = Instant::now();
        for _ in 0..iters {
            std::hint::black_box(render(page, &cache, &fonts, &rs));
        }
        let (w, h) = page.render_dimensions();
        println!(
            "\n  render only            {:9.4} ms/op  (~{}x{} px)",
            t.elapsed().as_secs_f64() * 1e3 / iters as f64,
            (w * scale) as u32,
            (h * scale) as u32
        );

        for (name, fmt) in [
            ("png", Format::Png),
            ("jpeg q90", Format::Jpeg),
            ("webp lossless", Format::Webp),
        ] {
            let settings = Settings {
                scale,
                format: fmt,
                quality: 90,
                bg_color: WHITE,
            };
            let mut out_len = 0usize;
            let t = Instant::now();
            for _ in 0..iters {
                let pm = render(page, &cache, &fonts, &rs);
                out_len = std::hint::black_box(encode(pm, &settings).expect("encode")).len();
            }
            println!(
                "  render+encode {name:14} {:9.4} ms/op  ({out_len} B out)",
                t.elapsed().as_secs_f64() * 1e3 / iters as f64
            );
        }
    }

    #[test]
    fn resolve_scale_prefers_scale_then_dpi_then_default() {
        assert_eq!(resolve_scale(&scaling(2.0, 300.0)), 2.0);
        assert_eq!(
            resolve_scale(&scaling(0.0, 144.0)),
            144.0 / PDF_POINTS_PER_INCH
        );
        assert_eq!(
            resolve_scale(&scaling(0.0, 0.0)),
            DEFAULT_DPI / PDF_POINTS_PER_INCH
        );
    }

    #[test]
    fn resolve_scale_falls_back_on_non_finite_or_nonpositive() {
        let default = DEFAULT_DPI / PDF_POINTS_PER_INCH;
        assert_eq!(resolve_scale(&scaling(f32::INFINITY, 0.0)), default);
        assert_eq!(resolve_scale(&scaling(f32::NAN, f32::NAN)), default);
        assert_eq!(resolve_scale(&scaling(-5.0, -10.0)), default);
    }

    #[test]
    fn output_pixels_accepts_a_normal_page() {
        // US Letter (612x792 pt) at the default 150-DPI scale.
        let px = output_pixels(612.0, 792.0, DEFAULT_DPI / PDF_POINTS_PER_INCH).unwrap();
        assert!(px > 1_000_000 && px < MAX_OUTPUT_PIXELS);
    }

    #[test]
    fn output_pixels_rejects_huge_scale() {
        // A large scale exceeds the pixel budget.
        assert_eq!(
            output_pixels(612.0, 792.0, 1.0e6).unwrap_err().code,
            ERR_RANGE
        );
        // page * scale is non-finite at f32::MAX.
        assert_eq!(
            output_pixels(612.0, 792.0, f32::MAX).unwrap_err().code,
            ERR_RANGE
        );
    }

    #[test]
    fn jpeg_flattens_transparent_background_to_white() {
        // JPEG has no alpha; a transparent background composites to white. Decode the
        // JPEG and check a background corner pixel.
        let blob = include_bytes!("../tests/assets/sample.pdf").to_vec();
        let pdf = load(blob).expect("load sample.pdf");
        let pages = pdf.pages();
        let page = pages.get(0).expect("a page");
        let cache = RenderCache::new();
        let fonts = font_settings();
        let scale = DEFAULT_DPI / PDF_POINTS_PER_INCH;
        let bg = AlphaColor::TRANSPARENT;
        let rs = RenderSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: bg,
            ..Default::default()
        };
        let settings = Settings {
            scale,
            format: Format::Jpeg,
            quality: 90,
            bg_color: bg,
        };
        let jpeg = encode(render(page, &cache, &fonts, &rs), &settings).expect("encode");
        let img = image::load_from_memory(&jpeg)
            .expect("decode jpeg")
            .to_rgb8();
        let p = img.get_pixel(0, 0);
        assert!(
            p[0] > 200 && p[1] > 200 && p[2] > 200,
            "transparent-bg corner should flatten to white, got {p:?}"
        );
    }

    #[test]
    fn output_pixels_rejects_oversized_page_and_degenerate_dims() {
        assert_eq!(
            output_pixels(200_000.0, 200_000.0, 1.0).unwrap_err().code,
            ERR_RANGE
        );
        assert!(output_pixels(0.0, 100.0, 1.0).is_err());
        assert!(output_pixels(100.0, 0.0, 1.0).is_err());
        assert!(output_pixels(f32::NAN, 100.0, 1.0).is_err());
        assert!(output_pixels(-100.0, 100.0, 1.0).is_err());
    }

    // Single-page PDF of `width` x `height` points painting one 2x2 RGB image over
    // the whole page.
    fn image_page_pdf(width: u32, height: u32) -> Vec<u8> {
        let content = format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] \
                 /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
            ),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let samples = [0u8, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 0];
        offsets.push(pdf.len());
        pdf.extend_from_slice(
            format!(
                "5 0 obj\n<< /Type /XObject /Subtype /Image /Width 2 /Height 2 \
                 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
                samples.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&samples);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
        );
        for offset in &offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn large_format_page_renders_within_the_pixel_limit() {
        // 150x100.17 in: 22500x15024 px at 150 DPI, over MAX_OUTPUT_PIXELS.
        let (page_w, page_h) = (10800u32, 7212u32);
        let options = RenderOptions {
            scaling: scaling(0.0, DEFAULT_DPI),
            output: bindings::OutputOptions {
                format: "jpeg".into(),
                quality: 0,
                background: String::new(),
            },
        };
        let images = <Component as Guest>::render(image_page_pdf(page_w, page_h), vec![0], options)
            .map_err(|e| e.message)
            .expect("large-format page renders");
        let img = image::load_from_memory(&images[0]).expect("decode jpeg");
        let pixels = u64::from(img.width()) * u64::from(img.height());
        assert!(pixels <= MAX_OUTPUT_PIXELS, "{pixels} px over the limit");
        assert!(
            pixels as f64 >= MAX_OUTPUT_PIXELS as f64 * 0.999,
            "{}x{} renders well below the limit",
            img.width(),
            img.height()
        );
        let aspect = f64::from(img.width()) / f64::from(img.height());
        let target = f64::from(page_w) / f64::from(page_h);
        assert!(
            (aspect / target - 1.0).abs() < 1e-3,
            "aspect {aspect} vs {target}"
        );
    }

    #[test]
    fn panoramic_pages_preserve_full_content_and_aspect() {
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (width, height) in [(14_400, 144), (144, 14_400), (65_527, 144), (65_531, 144)] {
                let options = RenderOptions {
                    scaling: scaling(if width > 14_400 { 1.0 } else { 8.0 }, 0.0),
                    output: bindings::OutputOptions {
                        format: "jpeg".into(),
                        quality: 90,
                        background: String::new(),
                    },
                };
                let images =
                    <Component as Guest>::render(image_page_pdf(width, height), vec![0], options)
                        .map_err(|e| e.message)
                        .expect("panoramic page renders");
                let img = image::load_from_memory(&images[0])
                    .expect("decode jpeg")
                    .to_rgb8();
                let aspect = f64::from(img.width()) / f64::from(img.height());
                let target = f64::from(width) / f64::from(height);
                assert!(
                    (aspect / target - 1.0).abs() < 1.0 / f64::from(img.width().min(img.height())),
                    "{}x{}: aspect {aspect} vs {target}",
                    img.width(),
                    img.height()
                );
                assert_eq!(img.width().max(img.height()), 65_527);
                assert!(u64::from(img.width()) * u64::from(img.height()) <= MAX_OUTPUT_PIXELS);
                for (x, y, light) in [(1, 1, false), (99, 1, true), (1, 99, true), (99, 99, false)]
                {
                    let pixel = img.get_pixel(img.width() * x / 100, img.height() * y / 100);
                    assert!(
                        pixel
                            .0
                            .iter()
                            .all(|c| if light { *c > 240 } else { *c < 15 }),
                        "full-page image corner ({x}, {y}): {pixel:?}"
                    );
                }
            }
            send.send(()).unwrap();
        });
        receive
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("panoramic rendering completes without a tile-counter loop");
    }

    #[test]
    fn page_scale_fits_renderer_tile_boundary() {
        for (width, height) in [(65_527.0, 9.0), (9.0, 65_527.0)] {
            assert_eq!(page_scale(width, height, 1.0).unwrap(), 1.0);
        }
        for (width, height) in [(65_531.0, 9.0), (9.0, 65_531.0)] {
            let scale = page_scale(width, height, 1.0).unwrap();
            assert!(scale < 1.0);
            assert_eq!((width.max(height) * scale).floor() as u32, 65_527);
        }
    }

    #[test]
    fn page_scale_keeps_in_budget_integer_dimensions() {
        for (width, height) in [(8192.9, 8192.1), (8192.1, 8192.9)] {
            let scale = page_scale(width, height, 1.0).unwrap();
            assert_eq!(scale, 1.0);
            assert_eq!((width * scale).floor() as u32, 8192);
            assert_eq!((height * scale).floor() as u32, 8192);
            assert_eq!(
                output_pixels(width, height, scale).unwrap(),
                MAX_OUTPUT_PIXELS
            );
        }
    }

    #[test]
    fn output_pixels_rejects_renderer_dimension_overflow() {
        assert!(output_pixels(65_536.0, 1.0, 1.0).is_err());
        assert!(output_pixels(1.0, 65_536.0, 1.0).is_err());
        assert!(output_pixels(65_528.0, 1.0, 1.0).is_err());
        assert!(output_pixels(1.0, 65_528.0, 1.0).is_err());
        assert_eq!(output_pixels(65_527.0, 1.0, 1.0).unwrap(), 65_527);
    }

    #[test]
    fn page_scale_keeps_requested_scale_within_the_limit() {
        let scale = DEFAULT_DPI / PDF_POINTS_PER_INCH;
        assert_eq!(page_scale(612.0, 792.0, scale).unwrap(), scale);
    }

    #[test]
    fn page_scale_bounds_oversized_output_to_the_limit() {
        for (w, h, requested) in [
            (10800.0, 7212.0, DEFAULT_DPI / PDF_POINTS_PER_INCH),
            (612.0, 792.0, 1.0e6),
            (612.0, 792.0, f32::MAX),
            (200_000.0, 200_000.0, 1.0),
            (8192.0, 8192.0, 1.0),
        ] {
            let scale = page_scale(w, h, requested).unwrap();
            assert!(scale <= requested);
            let pixels = output_pixels(w, h, scale).unwrap();
            assert!(
                pixels as f64 >= MAX_OUTPUT_PIXELS as f64 * 0.99,
                "{w}x{h}: {pixels} px"
            );
        }
    }

    #[test]
    fn page_scale_rejects_invalid_dimensions() {
        for (w, h) in [
            (0.0, 100.0),
            (100.0, 0.0),
            (f32::NAN, 100.0),
            (-100.0, 100.0),
            (f32::INFINITY, 100.0),
        ] {
            assert_eq!(page_scale(w, h, 1.0).unwrap_err().code, ERR_RANGE);
        }
    }

    fn rng(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn mutate(seed: &[u8], r: &mut u64) -> Vec<u8> {
        let mut v = seed.to_vec();
        for _ in 0..1 + (rng(r) % 24) {
            if v.is_empty() {
                v.push((rng(r) & 0xff) as u8);
            }
            let x = rng(r);
            let len = v.len();
            match x % 6 {
                0 => v[(x >> 8) as usize % len] ^= 1 << ((x >> 4) & 7),
                1 => v[(x >> 8) as usize % len] = (x >> 16) as u8,
                2 => v.insert((x >> 8) as usize % len, (x >> 16) as u8),
                3 => {
                    let i = (x >> 8) as usize % len;
                    v.remove(i);
                }
                4 => v.truncate((x >> 8) as usize % len),
                _ => v.extend(std::iter::repeat_n((x >> 16) as u8, (x >> 8) as usize % 64)),
            }
        }
        v
    }

    // Run:
    //   cargo test --release fuzz_render_never_panics -- --ignored --nocapture
    #[test]
    #[ignore]
    fn fuzz_render_never_panics() {
        const ITERS: u64 = 15_000;
        let seed = include_bytes!("../tests/assets/sample.pdf");
        let scale = DEFAULT_DPI / PDF_POINTS_PER_INCH;
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let mut crashes = 0u32;
        for iter in 0..ITERS {
            let mut r = 0x9A1F_55C0_2468_ACE0u64 ^ iter.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let input = mutate(seed, &mut r);
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let Ok(pdf) = load(input.clone()) else { return };
                let pages = pdf.pages();
                let Some(page) = pages.get(0) else { return };
                let (w, h) = page.render_dimensions();
                if output_pixels(w, h, scale).is_err() {
                    return;
                }
                let cache = RenderCache::new();
                let fonts = font_settings();
                let rs = RenderSettings {
                    x_scale: scale,
                    y_scale: scale,
                    bg_color: WHITE,
                    ..Default::default()
                };
                let pm = render(page, &cache, &fonts, &rs);
                let _ = encode(
                    pm,
                    &Settings {
                        scale,
                        format: Format::Jpeg,
                        quality: 90,
                        bg_color: WHITE,
                    },
                );
            }));
            if res.is_err() {
                crashes += 1;
                if crashes >= 5 {
                    break;
                }
            }
        }
        std::panic::set_hook(prev);
        assert_eq!(crashes, 0, "{crashes} pdf fuzz crash(es)");
        eprintln!("pdf fuzz: {ITERS} iters, 0 panics");
    }
}
