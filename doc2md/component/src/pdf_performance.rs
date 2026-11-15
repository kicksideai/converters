// SPDX-License-Identifier: MIT
use super::*;
use std::io::Write;

fn scanned_pdf(pages: usize) -> Vec<u8> {
    let mut objects = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), Vec::new()];
    let mut kids = Vec::new();
    for page in 1..=pages {
        let mut pixels = vec![255; 1500 * 1800 * 3];
        for y in 120..170 {
            for x in 90..1200 {
                if (x / 12 + y / 12 + page) % 3 == 0 {
                    let offset = (y * 1500 + x) * 3;
                    pixels[offset..offset + 3].fill(0);
                }
            }
        }
        pixels[..3].copy_from_slice(&[page as u8, 0, 0]);
        let mut compressed =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        compressed.write_all(&pixels).unwrap();
        let compressed = compressed.finish().unwrap();
        let image_id = objects.len() + 1;
        let mut image = format!("<< /Type /XObject /Subtype /Image /Width 1500 /Height 1800 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n", compressed.len()).into_bytes();
        image.extend_from_slice(&compressed);
        image.extend_from_slice(b"\nendstream");
        objects.push(image);
        let content = b"q 612 0 0 792 0 0 cm /Scan Do Q";
        let content_id = objects.len() + 1;
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                std::str::from_utf8(content).unwrap()
            )
            .into_bytes(),
        );
        let page_id = objects.len() + 1;
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Scan {image_id} 0 R >> >> /Contents {content_id} 0 R >>").into_bytes());
        kids.push(format!("{page_id} 0 R"));
    }
    objects[1] = format!(
        "<< /Type /Pages /Count {pages} /Kids [{}] >>",
        kids.join(" ")
    )
    .into_bytes();
    pdf_from_objects(&objects)
}

fn pdf_from_objects(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        write!(pdf, "{} 0 obj\n", index + 1).unwrap();
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    write!(pdf, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).unwrap();
    for offset in offsets {
        writeln!(pdf, "{offset:010} 00000 n ").unwrap();
    }
    write!(
        pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )
    .unwrap();
    pdf
}

const CMAP_ENTRIES: u32 = 60_000;
const RUN_CODES: u32 = 8;

fn cmap_glyph(code: u32) -> char {
    char::from(b'a' + (code % 26) as u8)
}

fn run_codes(page: u32, run: u32, runs: u32) -> impl Iterator<Item = u32> {
    let first = ((page * runs + run) * RUN_CODES) % (CMAP_ENTRIES - RUN_CODES) + 1;
    first..first + RUN_CODES
}

fn run_word(page: u32, run: u32, runs: u32) -> String {
    run_codes(page, run, runs).map(cmap_glyph).collect()
}

/// Text pages sharing one Type0 font whose ToUnicode CMap maps `CMAP_ENTRIES`
/// two-byte codes; every page shows `runs` separate text runs through it.
fn large_cmap_pdf(pages: u32, runs: u32) -> Vec<u8> {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let codes: Vec<u32> = (1..=CMAP_ENTRIES).collect();
    for chunk in codes.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for code in chunk {
            cmap.push_str(&format!(
                "<{code:04X}> <{:04X}>\n",
                cmap_glyph(*code) as u32
            ));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nend\nend\n");

    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        Vec::new(),
        b"<< /Type /Font /Subtype /Type0 /BaseFont /Synthetic /Encoding /Identity-H /DescendantFonts [4 0 R] /ToUnicode 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Synthetic /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 500 >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{cmap}endstream", cmap.len()).into_bytes(),
    ];
    let mut kids = Vec::new();
    for page in 0..pages {
        let mut content = String::new();
        for run in 0..runs {
            let hex: String = run_codes(page, run, runs)
                .map(|code| format!("{code:04X}"))
                .collect();
            let y = 780 - (run % 96) * 8;
            let x = 36 + (run / 96) * 280;
            content.push_str(&format!("BT /F1 6 Tf {x} {y} Td <{hex}> Tj ET\n"));
        }
        let content_id = objects.len() + 1;
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{content}endstream",
                content.len()
            )
            .into_bytes(),
        );
        let page_id = objects.len() + 1;
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {content_id} 0 R >>").into_bytes());
        kids.push(format!("{page_id} 0 R"));
    }
    objects[1] = format!(
        "<< /Type /Pages /Count {pages} /Kids [{}] >>",
        kids.join(" ")
    )
    .into_bytes();
    pdf_from_objects(&objects)
}

const LARGE_CMAP_PAGES: u32 = 50;
const LARGE_CMAP_RUNS: u32 = 200;

#[test]
fn large_tounicode_cmap_converts_every_text_run() {
    let (pages, runs) = (LARGE_CMAP_PAGES, LARGE_CMAP_RUNS);
    let pdf = large_cmap_pdf(pages, runs);
    let document = convert_pdf(
        &pdf,
        Settings {
            extract_images: true,
            page_markers: true,
        },
    )
    .unwrap();
    for page in 0..pages {
        assert!(document
            .markdown
            .contains(&format!("<!-- page {} -->", page + 1)));
        for run in [0, runs - 1] {
            let word = run_word(page, run, runs);
            assert!(
                document.markdown.contains(&word),
                "page {page} run {run}: {word}"
            );
        }
    }
}

#[test]
fn large_tounicode_cmap_page_resources_count_every_text_run() {
    let (pages, runs) = (LARGE_CMAP_PAGES, LARGE_CMAP_RUNS);
    let info = collect_page_resources(&large_cmap_pdf(pages, runs)).unwrap();
    assert_eq!(info.len(), pages as usize);
    for (index, page) in info.iter().enumerate() {
        assert_eq!(page.page, index as u32 + 1);
        assert_eq!(page.image_count, 0);
        assert!(
            page.text_len >= runs * RUN_CODES,
            "page {}: {}",
            page.page,
            page.text_len
        );
    }
}

#[test]
fn hundred_scanned_pages_preserve_images_and_page_identity() {
    let pdf = scanned_pdf(100);
    let document = convert_pdf(
        &pdf,
        Settings {
            extract_images: true,
            page_markers: true,
        },
    )
    .unwrap();
    assert_eq!(document.images.len(), 100);
    for (index, image) in document.images.iter().enumerate() {
        assert_eq!(image.page, Some(index as u32 + 1));
        assert!(image.name.ends_with(".png"));
        assert!(document
            .markdown
            .contains(&format!("<!-- page {} -->", index + 1)));
    }
    for index in [0, 99] {
        let mut reader = png::Decoder::new(std::io::Cursor::new(&document.images[index].data))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (1500, 1800));
        assert_eq!(&pixels[..3], &[index as u8 + 1, 0, 0]);
    }
}

#[test]
fn raw_pdf_png_preserves_every_sample() {
    for (space, channels) in [("DeviceGray", 1), ("DeviceRGB", 3), ("DeviceCMYK", 4)] {
        let samples: Vec<u8> = (0..257 * 19 * channels)
            .map(|i| (i * 73 + i / 7) as u8)
            .collect();
        let expected = if channels == 4 {
            samples
                .chunks_exact(4)
                .flat_map(|pixel| {
                    pixel[..3].iter().map(|channel| {
                        (((255 - *channel as u32) * (255 - pixel[3] as u32) + 127) / 255) as u8
                    })
                })
                .collect()
        } else {
            samples.clone()
        };
        let resource = unpdf::Resource::image(samples, "application/octet-stream")
            .with_dimensions(257, 19)
            .with_color_space(space)
            .with_bits_per_component(8)
            .with_samples(unpdf::SampleLayout {
                color_space: match channels {
                    1 => unpdf::ColorSpace::Gray,
                    3 => unpdf::ColorSpace::Rgb,
                    4 => unpdf::ColorSpace::Cmyk,
                    _ => unreachable!(),
                },
                decode: None,
            });
        let (bytes, extension) = pdf_image::image_file(resource).unwrap();
        assert_eq!(extension, "png");
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!((info.width, info.height), (257, 19));
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        assert_eq!(&decoded[..info.buffer_size()], expected);
    }
}

#[test]
fn counting_page_images_matches_extracting_them() {
    for pdf in [
        scanned_pdf(3),
        include_bytes!("../tests/assets/sample.pdf").to_vec(),
    ] {
        let parse = |options: unpdf::ParseOptions| {
            unpdf::parse_bytes_with_options(trim_pdf_trailer(&pdf), options).unwrap()
        };
        let extracted = parse(unpdf::ParseOptions::new().with_resources(true));
        let counted = parse(unpdf::ParseOptions::new().with_resource_counts(true));
        assert!(counted.resources.is_empty(), "counting decodes no resource");
        let counts = |doc: &unpdf::Document| -> Vec<u32> {
            doc.pages
                .iter()
                .map(|page| page.image_resource_count)
                .collect()
        };
        assert_eq!(counts(&counted), counts(&extracted));
    }
}

fn shared_image_pdf(pages: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; 300 * 200 * 3];
    for (index, value) in pixels.iter_mut().enumerate() {
        *value = (index % 251) as u8;
    }
    let mut compressed =
        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    compressed.write_all(&pixels).unwrap();
    let compressed = compressed.finish().unwrap();
    let mut image = format!("<< /Type /XObject /Subtype /Image /Width 300 /Height 200 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n", compressed.len()).into_bytes();
    image.extend_from_slice(&compressed);
    image.extend_from_slice(b"\nendstream");
    let content = b"q 300 0 0 200 0 0 cm /Shared Do Q";
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        Vec::new(),
        image,
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            std::str::from_utf8(content).unwrap()
        )
        .into_bytes(),
    ];
    let mut kids = Vec::new();
    for _ in 0..pages {
        objects.push(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Shared 3 0 R >> >> /Contents 4 0 R >>".to_vec());
        kids.push(format!("{} 0 R", objects.len()));
    }
    objects[1] = format!(
        "<< /Type /Pages /Count {pages} /Kids [{}] >>",
        kids.join(" ")
    )
    .into_bytes();
    pdf_from_objects(&objects)
}

#[test]
fn an_image_shared_by_pages_is_one_file_per_page() {
    let document = convert_pdf(
        &shared_image_pdf(3),
        Settings {
            extract_images: true,
            page_markers: false,
        },
    )
    .unwrap();
    assert_eq!(document.images.len(), 3);
    for (index, image) in document.images.iter().enumerate() {
        assert_eq!(image.page, Some(index as u32 + 1));
        assert_eq!(image.name, format!("page{}_object_3_0.png", index + 1));
        assert_eq!(image.data, document.images[0].data);
        assert!(document.markdown.contains(&image.name));
    }
}

#[test]
fn shared_image_respects_page_color_spaces() {
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 2 /Kids [3 0 R 4 0 R] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << /XObject << /Im0 5 0 R >> /ColorSpace << /CS [/Indexed /DeviceRGB 0 <FF0000>] >> >> /Contents 6 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << /XObject << /Im0 5 0 R >> /ColorSpace << /CS [/Indexed /DeviceRGB 0 <0000FF>] >> >> /Contents 6 0 R >>".to_vec(),
        b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /CS /BitsPerComponent 8 /Length 1 >>\nstream\n\x00\nendstream".to_vec(),
        b"<< /Length 27 >>\nstream\nq 72 0 0 72 0 0 cm /Im0 Do Q\nendstream".to_vec(),
    ];
    let converted = convert_pdf(&pdf_from_objects(&objects), Settings {
        extract_images: true, page_markers: false,
    }).unwrap();
    assert_eq!(converted.images.len(), 2);
    for (image, expected) in converted.images.iter().zip([[255, 0, 0], [0, 0, 255]]) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(&image.data)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(&pixels[..info.buffer_size()], &expected);
    }
}
