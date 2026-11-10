// SPDX-License-Identifier: MIT
use super::*;
use std::io::Write;
use std::time::{Duration, Instant};

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

#[test]
fn hundred_scanned_pages_native_conversion_budget() {
    let pdf = scanned_pdf(100);
    let started = Instant::now();
    let document = convert_pdf(
        &pdf,
        Settings {
            extract_images: true,
            page_markers: true,
        },
    )
    .unwrap();
    let elapsed = started.elapsed();
    eprintln!("100 scanned pages: {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(2),
        "native release conversion exceeds two seconds: {elapsed:?}; run with cargo test --release"
    );
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
            cmyk_to_rgb(&samples)
        } else {
            samples.clone()
        };
        let resource = unpdf::Resource::image(samples, "application/octet-stream")
            .with_dimensions(257, 19)
            .with_color_space(space)
            .with_bits_per_component(8);
        let (bytes, extension) = pdf_image_file(&resource);
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
