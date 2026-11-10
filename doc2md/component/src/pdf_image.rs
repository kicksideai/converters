// SPDX-License-Identifier: MIT
//! Viewable files from PDF image resources: encoded streams pass through, and
//! decoded sample buffers are rendered through their color space into PNG.

use unpdf::{ColorSpace, Resource, SampleLayout};

/// Returns the image file (bytes, extension) for a PDF image resource, or `None`
/// when the resource holds no decodable image.
pub(crate) fn image_file(resource: &Resource) -> Option<(Vec<u8>, &'static str)> {
    let ext = match resource.mime_type.as_str() {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/tiff" => Some("tiff"),
        "image/jp2" => Some("jp2"),
        _ => None,
    };
    if let Some(ext) = ext {
        return Some((resource.data.clone(), ext));
    }
    let layout = resource.samples.as_ref()?;
    let (width, height, bits) = (
        resource.width?,
        resource.height?,
        resource.bits_per_component?,
    );
    let (pixels, gray) = render_samples(&resource.data, width, height, bits, layout)?;
    encode_png(&pixels, width, height, gray).map(|png| (png, "png"))
}

// Renders samples to 8-bit gray or RGB pixels; returns (pixels, is_gray).
fn render_samples(
    data: &[u8],
    width: u32,
    height: u32,
    bits: u8,
    layout: &SampleLayout,
) -> Option<(Vec<u8>, bool)> {
    if ![1, 2, 4, 8, 16].contains(&bits) || width == 0 || height == 0 {
        return None;
    }
    let space = &layout.color_space;
    let n = space.components();
    let (w, h) = (width as usize, height as usize);
    let row_bytes = (w * n * bits as usize).div_ceil(8);
    if data.len() < row_bytes.checked_mul(h)? {
        return None;
    }
    let default_decode = space.default_decode(bits);
    let decode = layout.decode.as_deref().unwrap_or(&default_decode);
    if decode.len() < 2 * n {
        return None;
    }
    let gray = space.is_gray();
    let channels = if gray { 1 } else { 3 };

    if bits == 8
        && decode == default_decode.as_slice()
        && matches!(space, ColorSpace::Gray | ColorSpace::Rgb)
    {
        return Some((data[..w * h * n].to_vec(), gray));
    }

    let max = ((1u32 << bits) - 1) as f32;
    let mut components = vec![0.0; n];
    let mut pixel = |samples: &[u32]| -> Option<[u8; 3]> {
        for (i, s) in samples.iter().enumerate() {
            components[i] = decode[2 * i] + *s as f32 * (decode[2 * i + 1] - decode[2 * i]) / max;
        }
        Some(
            space
                .to_rgb(&components)?
                .map(|v| (v * 255.0).round() as u8),
        )
    };
    // One-component samples of at most 8 bits take a lookup table per sample value.
    let table: Option<Vec<[u8; 3]>> = if n == 1 && bits <= 8 {
        Some(
            (0..=max as u32)
                .map(|s| pixel(&[s]))
                .collect::<Option<Vec<_>>>()?,
        )
    } else {
        None
    };

    let mut out = Vec::with_capacity(w * h * channels);
    let mut samples = vec![0u32; n];
    for y in 0..h {
        let row = y * row_bytes * 8;
        for x in 0..w {
            for (c, sample) in samples.iter_mut().enumerate() {
                *sample =
                    unpdf::model::read_sample(data, row + (x * n + c) * bits as usize, bits as u32);
            }
            let rgb = match &table {
                Some(table) => table[samples[0] as usize],
                None => pixel(&samples)?,
            };
            out.extend_from_slice(&rgb[..channels]);
        }
    }
    Some((out, gray))
}

fn encode_png(pixels: &[u8], width: u32, height: u32, gray: bool) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(if gray {
            png::ColorType::Grayscale
        } else {
            png::ColorType::Rgb
        });
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(pixels).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::image_file;
    use crate::{convert_pdf, Settings};
    use std::io::Write;
    use unpdf::{ColorSpace, Resource, SampleLayout};

    #[test]
    fn encoded_streams_pass_through_and_device_samples_encode_as_png() {
        for (mime, ext) in [
            ("image/jpeg", "jpg"),
            ("image/png", "png"),
            ("image/gif", "gif"),
            ("image/tiff", "tiff"),
            ("image/jp2", "jp2"),
        ] {
            assert_eq!(
                image_file(&Resource::image(b"x".to_vec(), mime)),
                Some((b"x".to_vec(), ext))
            );
        }
        for (space, samples) in [
            (ColorSpace::Rgb, vec![10, 20, 30]),
            (ColorSpace::Gray, vec![128]),
            (ColorSpace::Cmyk, vec![0, 0, 0, 0]),
        ] {
            let resource = Resource::image(samples, "application/octet-stream")
                .with_dimensions(1, 1)
                .with_bits_per_component(8)
                .with_samples(SampleLayout {
                    color_space: space,
                    decode: None,
                });
            let (bytes, ext) = image_file(&resource).unwrap();
            assert_eq!((&bytes[..8], ext), (&b"\x89PNG\r\n\x1a\n"[..], "png"));
        }
    }

    #[test]
    fn cmyk_samples_render_subtractively() {
        let layout = SampleLayout {
            color_space: ColorSpace::Cmyk,
            decode: None,
        };
        let samples = [0, 0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 255, 100, 50, 10, 20];
        assert_eq!(
            super::render_samples(&samples, 4, 1, 8, &layout),
            Some((
                vec![255, 255, 255, 0, 255, 255, 0, 0, 0, 143, 189, 226],
                false
            ))
        );
    }

    #[test]
    fn sample_buffers_without_a_layout_or_with_too_few_bytes_are_not_images() {
        assert_eq!(
            image_file(
                &Resource::image(vec![1, 2, 3], "application/octet-stream")
                    .with_dimensions(1, 1)
                    .with_bits_per_component(8)
            ),
            None
        );
        let short = Resource::image(vec![1, 2], "application/octet-stream")
            .with_dimensions(5, 5)
            .with_bits_per_component(8)
            .with_samples(SampleLayout {
                color_space: ColorSpace::Rgb,
                decode: None,
            });
        assert_eq!(image_file(&short), None);
    }

    struct Object<'a> {
        dict: &'a str,
        stream: Option<&'a [u8]>,
    }

    // A one-page PDF drawing image object 5; `objects` become objects 6, 7, ...
    fn pdf_with_image(
        image_dict: &str,
        samples: &[u8],
        page_resources: &str,
        objects: &[Object],
    ) -> Vec<u8> {
        let content = b"q 100 0 0 100 0 0 cm /Im0 Do Q";
        let mut bodies: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im0 5 0 R >> {page_resources} >> /Contents 4 0 R >>").into_bytes(),
            stream_body("", content),
            stream_body(&format!("/Type /XObject /Subtype /Image {image_dict}"), samples),
        ];
        for object in objects {
            bodies.push(match object.stream {
                Some(data) => stream_body(object.dict, data),
                None => object.dict.as_bytes().to_vec(),
            });
        }
        assemble_pdf(bodies)
    }

    fn assemble_pdf(bodies: Vec<Vec<u8>>) -> Vec<u8> {
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in bodies.iter().enumerate() {
            offsets.push(pdf.len());
            write!(pdf, "{} 0 obj\n", index + 1).unwrap();
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        write!(pdf, "xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).unwrap();
        for offset in offsets {
            writeln!(pdf, "{offset:010} 00000 n ").unwrap();
        }
        write!(
            pdf,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            bodies.len() + 1
        )
        .unwrap();
        pdf
    }

    fn stream_body(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut body = format!("<< /Length {} {dict} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        body
    }

    fn flate(data: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    // Converts the PDF and returns the single extracted image decoded from PNG.
    fn extracted_png(pdf: &[u8]) -> (png::ColorType, Vec<u8>) {
        let converted = convert_pdf(
            pdf,
            Settings {
                extract_images: true,
                page_markers: false,
            },
        )
        .unwrap();
        assert_eq!(converted.images.len(), 1, "expected one extracted image");
        let image = &converted.images[0];
        assert!(
            image.name.ends_with(".png"),
            "image is named {}",
            image.name
        );
        let mut reader = png::Decoder::new(std::io::Cursor::new(image.data.clone()))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        pixels.truncate(frame.buffer_size());
        (frame.color_type, pixels)
    }

    #[test]
    fn separation_samples_use_a_sampled_tint_transform() {
        let pdf = pdf_with_image(
            "/Width 3 /Height 1 /BitsPerComponent 8 /ColorSpace [/Separation /Black /DeviceRGB 6 0 R] /Filter /FlateDecode",
            &flate(&[0, 128, 255]),
            "",
            &[Object {
                dict: "/FunctionType 0 /Domain [0 1] /Range [0 1 0 1 0 1] /Size [2] /BitsPerSample 8",
                stream: Some(&[255, 255, 255, 0, 0, 0]),
            }],
        );
        let (color, pixels) = extracted_png(&pdf);
        assert_eq!(color, png::ColorType::Rgb);
        assert_eq!(pixels, vec![255, 255, 255, 127, 127, 127, 0, 0, 0]);
    }

    #[test]
    fn separation_samples_use_an_exponential_tint_transform_into_gray() {
        let pdf = pdf_with_image(
            "/Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace [/Separation /Black /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>]",
            &[0, 255],
            "",
            &[],
        );
        assert_eq!(
            extracted_png(&pdf),
            (png::ColorType::Grayscale, vec![255, 0])
        );
    }

    #[test]
    fn devicen_samples_use_a_calculator_tint_transform() {
        let pdf = pdf_with_image(
            "/Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace [/DeviceN [/Cyan /Magenta] /DeviceCMYK 6 0 R]",
            &[255, 0, 0, 255],
            "",
            &[Object {
                dict: "/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1 0 1]",
                stream: Some(b"{ 0 0 }"),
            }],
        );
        assert_eq!(
            extracted_png(&pdf),
            (png::ColorType::Rgb, vec![0, 255, 255, 255, 0, 255])
        );
    }

    #[test]
    fn indexed_samples_look_up_their_palette() {
        let pdf = pdf_with_image(
            "/Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace [/Indexed /DeviceRGB 1 <FF00000000FF>]",
            &[0, 1],
            "",
            &[],
        );
        assert_eq!(
            extracted_png(&pdf),
            (png::ColorType::Rgb, vec![255, 0, 0, 0, 0, 255])
        );
    }

    #[test]
    fn image_masks_and_sub_byte_samples_unpack_per_row() {
        let mask = pdf_with_image(
            "/Width 3 /Height 2 /ImageMask true",
            &[0b1010_0000, 0b0100_0000],
            "",
            &[],
        );
        assert_eq!(
            extracted_png(&mask),
            (png::ColorType::Grayscale, vec![255, 0, 255, 0, 255, 0])
        );

        let gray = pdf_with_image(
            "/Width 2 /Height 1 /BitsPerComponent 4 /ColorSpace /DeviceGray /Decode [1 0]",
            &[0xF0],
            "",
            &[],
        );
        assert_eq!(
            extracted_png(&gray),
            (png::ColorType::Grayscale, vec![0, 255])
        );
    }

    #[test]
    fn icc_and_named_color_spaces_resolve_to_their_components() {
        let icc = pdf_with_image(
            "/Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace [/ICCBased 6 0 R]",
            &[10, 20, 30],
            "",
            &[Object {
                dict: "/N 3",
                stream: Some(b"profile"),
            }],
        );
        assert_eq!(extracted_png(&icc), (png::ColorType::Rgb, vec![10, 20, 30]));

        let named = pdf_with_image(
            "/Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /CS0",
            &[0],
            "/ColorSpace << /CS0 [/Indexed /DeviceGray 0 <80>] >>",
            &[],
        );
        assert_eq!(
            extracted_png(&named),
            (png::ColorType::Grayscale, vec![128])
        );
    }

    #[test]
    fn lab_samples_convert_to_rgb() {
        let pdf = pdf_with_image(
            "/Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace [/Lab << /WhitePoint [0.9505 1 1.089] /Range [-128 127 -128 127] >>]",
            &[255, 128, 128, 0, 128, 128],
            "",
            &[],
        );
        assert_eq!(
            extracted_png(&pdf),
            (png::ColorType::Rgb, vec![255, 255, 255, 0, 0, 0])
        );
    }

    #[test]
    fn samples_left_encoded_by_an_unsupported_filter_are_not_reported_as_images() {
        let pdf = pdf_with_image(
            "/Width 8 /Height 8 /BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /CCITTFaxDecode",
            &[0x26, 0xA0, 0x00, 0x10],
            "",
            &[],
        );
        let converted = convert_pdf(
            &pdf,
            Settings {
                extract_images: true,
                page_markers: false,
            },
        )
        .unwrap();
        assert!(
            converted.images.is_empty(),
            "got {:?}",
            converted.images.iter().map(|i| &i.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn special_separation_colorants_ignore_the_alternate_and_transform() {
        for (name, expected) in [("All", vec![255, 127, 0]), ("None", vec![255, 255, 255])] {
            let pdf = pdf_with_image(
                &format!("/Width 3 /Height 1 /BitsPerComponent 8 /ColorSpace [/Separation /{name} /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>]"),
                &[0, 128, 255], "", &[]);
            assert_eq!(
                extracted_png(&pdf),
                (png::ColorType::Grayscale, expected),
                "{name}"
            );
        }
    }

    #[test]
    fn calculator_comments_and_logical_shifts_produce_exact_pixels() {
        for (program, expected) in [
            (
                &b"{ % tint to gray\r\n 1 exch sub % } ignored\n }"[..],
                vec![255, 0],
            ),
            (
                &b"{ pop -1 -1 bitshift 0 gt { 0 } { 1 } ifelse }"[..],
                vec![0, 0],
            ),
        ] {
            let pdf = pdf_with_image(
                "/Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace [/Separation /Black /DeviceGray 6 0 R]",
                &[0, 255], "", &[Object { dict: "/FunctionType 4 /Domain [0 1] /Range [0 1]", stream: Some(program) }]);
            assert_eq!(extracted_png(&pdf), (png::ColorType::Grayscale, expected));
        }
    }

    #[test]
    fn cubic_sampled_functions_are_explicitly_unsupported() {
        let pdf = pdf_with_image(
            "/Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace [/Separation /Black /DeviceGray 6 0 R]",
            &[128], "", &[Object { dict: "/FunctionType 0 /Domain [0 1] /Range [0 1] /Size [4] /BitsPerSample 8 /Order 3", stream: Some(&[0, 255, 0, 255]) }]);
        let doc =
            unpdf::parse_bytes_with_options(&pdf, unpdf::ParseOptions::new().with_resources(true))
                .unwrap();
        assert!(doc.resources.values().all(|r| r.samples.is_none()));
        assert_eq!(
            crate::collect_page_resources(&pdf).unwrap()[0].image_count,
            1
        );
    }

    #[test]
    fn icc_ranges_and_alternates_preserve_sample_mapping() {
        for (profile, data, expected) in [
            (
                "/N 1 /Alternate /DeviceGray /Range [0 0.5]",
                vec![0, 255],
                vec![0, 128],
            ),
            ("/N 1 /Range [0.25 0.75]", vec![0, 255], vec![64, 191]),
            ("/N 1 /Alternate /DeviceGray", vec![0, 255], vec![0, 255]),
        ] {
            let pdf = pdf_with_image(
                "/Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace [/ICCBased 6 0 R]",
                &data,
                "",
                &[Object {
                    dict: profile,
                    stream: Some(b"profile"),
                }],
            );
            assert_eq!(
                extracted_png(&pdf),
                (png::ColorType::Grayscale, expected),
                "{profile}"
            );
        }
        let pdf = pdf_with_image(
            "/Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace [/ICCBased 6 0 R]",
            &[255],
            "",
            &[Object {
                dict: "/N 1 /Alternate /DeviceRGB",
                stream: Some(b"profile"),
            }],
        );
        let doc =
            unpdf::parse_bytes_with_options(&pdf, unpdf::ParseOptions::new().with_resources(true))
                .unwrap();
        assert!(doc.resources.values().all(|r| r.samples.is_none()));
    }

    #[test]
    fn stitching_two_and_sixteen_bit_samples_inverse_masks_and_stream_palettes() {
        let stitched = pdf_with_image(
            "/Width 3 /Height 1 /BitsPerComponent 8 /ColorSpace [/Separation /Black /DeviceGray << /FunctionType 3 /Domain [0 1] /Functions [<< /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>] /Bounds [0.5] /Encode [0 1 0 1] >>]", &[0, 128, 255], "", &[]);
        assert_eq!(
            extracted_png(&stitched),
            (png::ColorType::Grayscale, vec![255, 1, 255])
        );
        for (attributes, data, expected) in [
            (
                "/Width 3 /Height 2 /BitsPerComponent 2 /ColorSpace /DeviceGray",
                vec![0x18, 0xe4],
                vec![0, 85, 170, 255, 170, 85],
            ),
            (
                "/Width 3 /Height 1 /BitsPerComponent 16 /ColorSpace /DeviceGray",
                vec![0, 0, 128, 0, 255, 255],
                vec![0, 128, 255],
            ),
            (
                "/Width 3 /Height 2 /ImageMask true /Decode [1 0]",
                vec![0xa0, 0x40],
                vec![0, 255, 0, 255, 0, 255],
            ),
        ] {
            assert_eq!(
                extracted_png(&pdf_with_image(attributes, &data, "", &[])),
                (png::ColorType::Grayscale, expected),
                "{attributes}"
            );
        }
        let palette = flate(&[255, 0, 0, 0, 0, 255]);
        let pdf = pdf_with_image(
            "/Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace [/Indexed /DeviceRGB 1 6 0 R]",
            &[0, 1],
            "",
            &[Object {
                dict: "/Filter /FlateDecode",
                stream: Some(&palette),
            }],
        );
        assert_eq!(
            extracted_png(&pdf),
            (png::ColorType::Rgb, vec![255, 0, 0, 0, 0, 255])
        );
    }

    #[test]
    fn single_element_encoded_filter_arrays_preserve_the_encoded_file() {
        for (filter, ext) in [("JPXDecode", "jp2"), ("DCTDecode", "jpg")] {
            let pdf = pdf_with_image(&format!("/Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter [/{filter}]"), b"encoded", "", &[]);
            let converted = convert_pdf(
                &pdf,
                Settings {
                    extract_images: true,
                    page_markers: false,
                },
            )
            .unwrap();
            assert_eq!(converted.images.len(), 1, "{filter}");
            assert!(converted.images[0].name.ends_with(ext));
            assert_eq!(converted.images[0].data, b"encoded");
        }
    }

    #[test]
    fn unsupported_filters_remain_in_resource_counts_without_fake_pixels() {
        for filter in ["CCITTFaxDecode", "JBIG2Decode", "UnknownDecode"] {
            let pdf = pdf_with_image(&format!("/Width 100 /Height 100 /BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /{filter}"), &[0x26, 0xa0, 0, 0x10], "", &[]);
            let converted = convert_pdf(
                &pdf,
                Settings {
                    extract_images: true,
                    page_markers: true,
                },
            )
            .unwrap();
            assert!(converted.images.is_empty(), "{filter}");
            let pages = crate::collect_page_resources(&pdf).unwrap();
            assert_eq!(
                (pages[0].image_count, pages[0].text_len),
                (1, 0),
                "{filter}"
            );
        }
    }

    #[test]
    fn dropping_the_first_image_keeps_links_on_their_own_pages() {
        let pdf = assemble_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Count 2 /Kids [3 0 R 4 0 R] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Bad 7 0 R >> >> /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Good 8 0 R >> >> /Contents 6 0 R >>".to_vec(),
            stream_body("", b"q 100 0 0 100 0 0 cm /Bad Do Q"),
            stream_body("", b"q 100 0 0 100 0 0 cm /Good Do Q"),
            stream_body("/Type /XObject /Subtype /Image /Width 64 /Height 64 /BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /CCITTFaxDecode", &[0x26, 0xa0, 0, 0x10]),
            stream_body("/Type /XObject /Subtype /Image /Width 64 /Height 64 /BitsPerComponent 8 /ColorSpace /DeviceRGB", &[10, 20, 30].repeat(64*64)),
        ]);
        let converted = convert_pdf(
            &pdf,
            Settings {
                extract_images: true,
                page_markers: true,
            },
        )
        .unwrap();
        assert_eq!(converted.images.len(), 1);
        let image = &converted.images[0];
        assert_eq!(image.page, Some(2));
        let link = converted
            .markdown
            .find(&format!("]({})", image.name))
            .unwrap_or_else(|| panic!("{}", converted.markdown));
        let marker = converted.markdown.find("<!-- page 2 -->").unwrap();
        assert!(link > marker, "{}", converted.markdown);
        let mut reader = png::Decoder::new(std::io::Cursor::new(&image.data))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(pixels, [10, 20, 30].repeat(64 * 64));
    }

    #[test]
    fn cmyk_and_devicen_allocation_counts_do_not_grow_with_pixel_count() {
        use unpdf::model::{parse_calculator, Function};
        let exponential = Function::Exponential {
            domain: [0., 1.],
            c0: vec![0.; 4],
            c1: vec![1.; 4],
            exponent: 1.,
            range: None,
        };
        let functions = vec![
            Function::Calculator {
                domain: vec![0., 1., 0., 1.],
                range: vec![0., 1., 0., 1., 0., 1., 0., 1.],
                program: parse_calculator(b"{ 2 copy pop pop 0 0 }").unwrap(),
            },
            Function::Sampled {
                domain: vec![0., 1., 0., 1.],
                range: vec![0., 1., 0., 1., 0., 1., 0., 1.],
                size: vec![2, 2],
                bits_per_sample: 8,
                encode: vec![0., 1., 0., 1.],
                decode: vec![0., 1., 0., 1., 0., 1., 0., 1.],
                samples: vec![0; 16],
            },
            exponential.clone(),
            Function::Stitching {
                domain: [0., 1.],
                functions: vec![exponential.clone(), exponential],
                bounds: vec![0.5],
                encode: vec![0., 1., 0., 1.],
                range: None,
            },
            Function::Array(
                (0..4)
                    .map(|_| Function::Exponential {
                        domain: [0., 1.],
                        c0: vec![0.],
                        c1: vec![1.],
                        exponent: 1.,
                        range: None,
                    })
                    .collect(),
            ),
        ];
        for space in
            std::iter::once(ColorSpace::Cmyk).chain(functions.into_iter().map(|tint_transform| {
                ColorSpace::DeviceN {
                    components: 2,
                    alternate: Box::new(ColorSpace::Cmyk),
                    tint_transform,
                }
            }))
        {
            let n = space.components();
            let layout = SampleLayout {
                color_space: space,
                decode: None,
            };
            let small = vec![0; n];
            let large = vec![0; 4096 * n];
            let (one, _) = crate::test_allocations::count(|| {
                super::render_samples(&small, 1, 1, 8, &layout).unwrap()
            });
            let (many, (pixels, _)) = crate::test_allocations::count(|| {
                super::render_samples(&large, 4096, 1, 8, &layout).unwrap()
            });
            assert_eq!(pixels, vec![255; 4096 * 3]);
            assert_eq!(
                many, one,
                "{:?}: one={one}, many={many}",
                layout.color_space
            );
            eprintln!(
                "{one} allocations for 1 pixel; {many} for 4096 pixels: {:?}",
                layout.color_space
            );
        }
    }

    #[test]
    fn calculator_stack_overflow_rejects_samples_without_losing_the_page() {
        let program = format!("{{ pop {} }}", "1 ".repeat(101));
        let pdf = pdf_with_image(
            "/Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace [/Separation /Black /DeviceGray 6 0 R]",
            &[128], "", &[Object {dict: "/FunctionType 4 /Domain [0 1] /Range [0 1]", stream: Some(program.as_bytes())}]);
        let converted = convert_pdf(
            &pdf,
            Settings {
                extract_images: true,
                page_markers: true,
            },
        )
        .unwrap();
        assert!(converted.images.is_empty());
        assert_eq!(
            crate::collect_page_resources(&pdf).unwrap()[0].image_count,
            1
        );
    }

    #[test]
    fn invalid_calculator_arithmetic_does_not_fabricate_pixels() {
        for operation in ["0 0 div", "1 0 idiv", "1 0 mod", "-1 sqrt", "0 ln"] {
            let program = format!("{{ pop {operation} }}");
            let pdf = pdf_with_image("/Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace [/Separation /Black /DeviceGray 6 0 R]", &[128], "", &[Object {dict: "/FunctionType 4 /Domain [0 1] /Range [0 1]", stream: Some(program.as_bytes())}]);
            let converted = convert_pdf(
                &pdf,
                Settings {
                    extract_images: true,
                    page_markers: true,
                },
            )
            .unwrap();
            assert!(converted.images.is_empty(), "{operation}");
            assert_eq!(
                crate::collect_page_resources(&pdf).unwrap()[0].image_count,
                1
            );
        }
    }
}
