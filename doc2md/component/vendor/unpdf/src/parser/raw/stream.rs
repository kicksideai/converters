//! PDF stream decompression.

use super::tokenizer::{dict_get, PdfDict, PdfObject, PdfStream};
use crate::error::{Error, Result};
use std::io::Read;

/// Decompress a PDF stream based on its Filter entry.
/// Also applies predictor decoding from DecodeParms if present.
pub fn decompress(stream: &PdfStream) -> Result<Vec<u8>> {
    let filter = dict_get(&stream.dict, b"Filter");

    let decompressed = match filter {
        None => return Ok(stream.raw_data.clone()),
        Some(PdfObject::Name(name)) => decompress_single(name, &stream.raw_data)?,
        Some(PdfObject::Array(filters)) => {
            let mut data = stream.raw_data.clone();
            for f in filters {
                if let Some(name) = f.as_name() {
                    data = decompress_single(name, &data)?;
                }
            }
            data
        }
        _ => return Ok(stream.raw_data.clone()),
    };

    // Apply predictor decoding if DecodeParms is present
    let decode_parms = dict_get(&stream.dict, b"DecodeParms");
    if let Some(parms) = decode_parms {
        if let Some(parms_dict) = parms.as_dict() {
            return apply_predictor(parms_dict, &decompressed);
        }
    }

    Ok(decompressed)
}

fn decompress_single(filter_name: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    match filter_name {
        b"FlateDecode" | b"Fl" => decompress_flate(data),
        b"ASCIIHexDecode" | b"AHx" => decode_ascii_hex(data),
        b"ASCII85Decode" | b"A85" => decode_ascii85(data),
        _ => Err(Error::PdfParse(format!(
            "unsupported filter: {}",
            String::from_utf8_lossy(filter_name)
        ))),
    }
}

fn decompress_flate(data: &[u8]) -> Result<Vec<u8>> {
    // Try zlib first (most common)
    let mut output = Vec::new();
    if flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut output)
        .is_ok()
    {
        return Ok(output);
    }

    // Fallback: raw deflate (some PDF producers omit zlib header)
    output.clear();
    if flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut output)
        .is_ok()
    {
        return Ok(output);
    }

    // Lenient fallback: read as much as we can from partial/corrupt streams.
    // Some PDFs have trailing garbage, wrong /Length values, or minor stream
    // corruption. We try both zlib and raw deflate in chunked reads and return
    // whatever partial data was successfully decompressed.
    output.clear();
    if let Some(partial) = decompress_flate_partial(data) {
        if !partial.is_empty() {
            return Ok(partial);
        }
    }

    Err(Error::PdfParse(
        "decompression failed: corrupt deflate stream".to_string(),
    ))
}

/// Attempt to decompress as much data as possible from a potentially
/// corrupt or truncated flate stream. Returns `Some(data)` with whatever
/// bytes were successfully decompressed, or `None` on total failure.
fn decompress_flate_partial(data: &[u8]) -> Option<Vec<u8>> {
    // Try zlib partial read
    let mut best = try_partial_read(flate2::read::ZlibDecoder::new(data));

    // Try raw deflate partial read
    let raw = try_partial_read(flate2::read::DeflateDecoder::new(data));
    if raw.len() > best.len() {
        best = raw;
    }

    // Also try trimming trailing bytes (some PDFs append extra zeros/garbage).
    // Try stripping up to 8 trailing bytes.
    for trim in 1..=std::cmp::min(8, data.len().saturating_sub(1)) {
        let trimmed = &data[..data.len() - trim];

        let z = try_partial_read(flate2::read::ZlibDecoder::new(trimmed));
        if z.len() > best.len() {
            best = z;
        }

        let d = try_partial_read(flate2::read::DeflateDecoder::new(trimmed));
        if d.len() > best.len() {
            best = d;
        }
    }

    if best.is_empty() {
        None
    } else {
        Some(best)
    }
}

/// Read from a decoder in small chunks, returning whatever was successfully
/// decompressed before an error occurs.
fn try_partial_read<R: Read>(mut reader: R) -> Vec<u8> {
    let mut output = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => output.extend_from_slice(&buf[..n]),
            Err(_) => break, // stop at first error, keep what we have
        }
    }
    output
}

/// Apply predictor decoding as specified by DecodeParms.
///
/// PDF supports two families of predictors:
/// - TIFF Predictor 2: horizontal differencing
/// - PNG Predictors 10-15: PNG filter methods (None, Sub, Up, Average, Paeth, Optimum)
fn apply_predictor(parms: &PdfDict, data: &[u8]) -> Result<Vec<u8>> {
    let predictor = dict_get(parms, b"Predictor")
        .and_then(|o| o.as_i64())
        .unwrap_or(1);

    // Predictor 1 = no prediction
    if predictor == 1 {
        return Ok(data.to_vec());
    }

    let columns = dict_get(parms, b"Columns")
        .and_then(|o| o.as_i64())
        .unwrap_or(1) as usize;

    let colors = dict_get(parms, b"Colors")
        .and_then(|o| o.as_i64())
        .unwrap_or(1) as usize;

    let bits_per_component = dict_get(parms, b"BitsPerComponent")
        .and_then(|o| o.as_i64())
        .unwrap_or(8) as usize;

    if predictor == 2 {
        // TIFF Predictor 2: horizontal differencing
        return apply_tiff_predictor(data, columns, colors, bits_per_component);
    }

    if (10..=15).contains(&predictor) {
        // PNG predictors
        return apply_png_predictor(data, columns, colors, bits_per_component);
    }

    // Unknown predictor, return data as-is
    Ok(data.to_vec())
}

/// Apply TIFF Predictor 2 (horizontal differencing).
fn apply_tiff_predictor(
    data: &[u8],
    columns: usize,
    colors: usize,
    bits_per_component: usize,
) -> Result<Vec<u8>> {
    if bits_per_component != 8 {
        // Only 8-bit components are commonly used; return as-is for others
        return Ok(data.to_vec());
    }
    let row_bytes = columns * colors;
    if row_bytes == 0 {
        return Ok(data.to_vec());
    }

    let mut result = data.to_vec();
    let num_rows = result.len() / row_bytes;

    for row in 0..num_rows {
        let row_start = row * row_bytes;
        for col in colors..row_bytes {
            let idx = row_start + col;
            if idx < result.len() {
                result[idx] = result[idx].wrapping_add(result[idx - colors]);
            }
        }
    }

    Ok(result)
}

/// Apply PNG predictor decoding.
///
/// Each row is prefixed with a 1-byte filter type:
/// 0 = None, 1 = Sub, 2 = Up, 3 = Average, 4 = Paeth
fn apply_png_predictor(
    data: &[u8],
    columns: usize,
    colors: usize,
    bits_per_component: usize,
) -> Result<Vec<u8>> {
    // bytes per pixel (for Sub/Paeth filter lookback)
    let bpp = std::cmp::max(1, (colors * bits_per_component).div_ceil(8));
    // row data bytes (excluding filter byte)
    let row_bytes = columns * colors * bits_per_component / 8;
    // each input row = 1 filter byte + row_bytes data bytes
    let input_row_len = 1 + row_bytes;

    if input_row_len == 0 || data.len() % input_row_len != 0 {
        // If data doesn't divide evenly, try using columns directly as row_bytes
        // (common when Columns already accounts for all bytes per row)
        let alt_row_bytes = columns;
        let alt_input_row_len = 1 + alt_row_bytes;
        if alt_input_row_len > 0 && data.len() % alt_input_row_len == 0 {
            return apply_png_predictor_raw(data, alt_row_bytes, bpp);
        }
        // Fall back: return data as-is rather than fail
        return Ok(data.to_vec());
    }

    apply_png_predictor_raw(data, row_bytes, bpp)
}

/// Core PNG predictor un-filtering.
fn apply_png_predictor_raw(data: &[u8], row_bytes: usize, bpp: usize) -> Result<Vec<u8>> {
    let input_row_len = 1 + row_bytes;
    let num_rows = data.len() / input_row_len;
    let mut result = Vec::with_capacity(num_rows * row_bytes);
    let mut prev_row = vec![0u8; row_bytes];

    for row_idx in 0..num_rows {
        let row_start = row_idx * input_row_len;
        let filter_type = data[row_start];
        let row_data = &data[row_start + 1..row_start + input_row_len];

        let mut current_row = vec![0u8; row_bytes];

        match filter_type {
            0 => {
                // None
                current_row.copy_from_slice(row_data);
            }
            1 => {
                // Sub
                for i in 0..row_bytes {
                    let left = if i >= bpp { current_row[i - bpp] } else { 0 };
                    current_row[i] = row_data[i].wrapping_add(left);
                }
            }
            2 => {
                // Up
                for i in 0..row_bytes {
                    current_row[i] = row_data[i].wrapping_add(prev_row[i]);
                }
            }
            3 => {
                // Average
                for i in 0..row_bytes {
                    let left = if i >= bpp {
                        current_row[i - bpp] as u16
                    } else {
                        0
                    };
                    let up = prev_row[i] as u16;
                    current_row[i] = row_data[i].wrapping_add(((left + up) / 2) as u8);
                }
            }
            4 => {
                // Paeth
                for i in 0..row_bytes {
                    let left = if i >= bpp { current_row[i - bpp] } else { 0 };
                    let up = prev_row[i];
                    let up_left = if i >= bpp { prev_row[i - bpp] } else { 0 };
                    current_row[i] = row_data[i].wrapping_add(paeth_predictor(left, up, up_left));
                }
            }
            _ => {
                // Unknown filter type — treat as None
                current_row.copy_from_slice(row_data);
            }
        }

        result.extend_from_slice(&current_row);
        prev_row = current_row;
    }

    Ok(result)
}

/// Paeth predictor function (used in PNG filter type 4).
fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i16;
    let b = b as i16;
    let c = c as i16;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn decode_ascii_hex(data: &[u8]) -> Result<Vec<u8>> {
    let hex: String = data
        .iter()
        .filter(|b| !b.is_ascii_whitespace())
        .take_while(|&&b| b != b'>')
        .map(|&b| b as char)
        .collect();
    let mut result = Vec::with_capacity(hex.len() / 2);
    let mut chars = hex.chars();
    while let Some(h) = chars.next() {
        let l = chars.next().unwrap_or('0');
        let byte = u8::from_str_radix(&format!("{}{}", h, l), 16)
            .map_err(|_| Error::PdfParse("invalid hex in ASCIIHexDecode".to_string()))?;
        result.push(byte);
    }
    Ok(result)
}

#[inline]
fn is_pdf_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\0' | b'\x0C')
}

/// Report whether the input opens with the Adobe PostScript `<~` wrapper.
///
/// `<` is a base-85 digit, so the digit loop cannot tell the wrapper apart from data;
/// the sequence is identified here, before any digit is consumed. PDF whitespace is
/// ignored anywhere in an ASCII85 stream, so it is skipped between the two bytes too.
fn starts_with_adobe_wrapper(data: &[u8]) -> bool {
    let mut bytes = data.iter().copied().filter(|&b| !is_pdf_whitespace(b));
    bytes.next() == Some(b'<') && bytes.next() == Some(b'~')
}

/// Decode ASCII base-85 encoded data as specified in ISO 32000-1 §7.4.3 (ASCII85Decode Filter).
///
/// Per ISO 32000-1 §7.4.3 and PDF stream filter semantics:
/// - Encoded data consists of characters in the range '!' (33) to 'u' (117) and 'z' (122).
/// - PDF whitespace characters (NUL, TAB, LF, FF, CR, SPACE) are ignored anywhere in the input.
/// - The 2-character sequence `~>` indicates End-Of-Data (EOD) and is strictly required to
///   terminate decoding, including for empty data or whitespace-only inputs. Any stream reaching
///   EOF without encountering `~>` is rejected as truncated/corrupt.
/// - When `~>` is encountered, decoding terminates successfully; any trailing data following `~>`
///   in the stream buffer is ignored per PDF stream filter semantics.
/// - Adobe PostScript opening wrapper `<~` is not part of the PDF filter specification and is
///   strictly rejected as invalid syntax (PDF streams are bounded by `stream`/`endstream` keywords).
///   `<` (0x3C) is itself a base-85 digit, so the wrapper is recognised up front by
///   `starts_with_adobe_wrapper` rather than left to the digit loop.
/// - Zero shorthand `'z'` (representing 4 zero bytes) is only valid at 5-tuple boundaries (`count == 0`).
///   Encountering `'z'` inside a partial tuple is rejected.
/// - A final partial group of 2, 3, or 4 characters preceding `~>` is padded with `'u'` (value 84)
///   and decoded to `count - 1` bytes. A partial group of length 1 is invalid per spec.
/// - Any 5-tuple or partial group decoding to a value > 0xFFFF_FFFF (overflow) is rejected.
/// - Non-alphabet characters, truncated or bare `~` (not followed by `>`), and missing EOD markers
///   are rejected with `Error::PdfParse`.
fn decode_ascii85(data: &[u8]) -> Result<Vec<u8>> {
    if starts_with_adobe_wrapper(data) {
        return Err(Error::PdfParse(
            "ASCII85Decode: Adobe '<~' opening wrapper is not valid in a PDF stream filter"
                .to_string(),
        ));
    }

    let mut output = Vec::with_capacity(data.len());
    let mut tuple = [0u8; 5];
    let mut count = 0usize;
    let mut i = 0;
    let mut eod_found = false;

    while i < data.len() {
        let b = data[i];
        i += 1;

        if is_pdf_whitespace(b) {
            continue;
        }

        if b == b'z' {
            if count != 0 {
                return Err(Error::PdfParse(
                    "ASCII85Decode: 'z' character inside partial tuple".to_string(),
                ));
            }
            output.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }

        if b == b'~' {
            if i < data.len() && data[i] == b'>' {
                // Consume '>' and terminate decoding (EOD marker).
                // Per ISO 32000-1 §7.4.3, trailing data after EOD is ignored.
                eod_found = true;
                break;
            } else {
                return Err(Error::PdfParse(
                    "ASCII85Decode: truncated or invalid '~>' terminator".to_string(),
                ));
            }
        }

        if (b'!'..=b'u').contains(&b) {
            tuple[count] = b - b'!';
            count += 1;
            if count == 5 {
                let mut val: u64 = 0;
                for &digit in &tuple {
                    val = val * 85 + digit as u64;
                }
                if val > 0xFFFF_FFFF {
                    return Err(Error::PdfParse(
                        "ASCII85Decode: 5-tuple value exceeds 32 bits (overflow)".to_string(),
                    ));
                }
                output.extend_from_slice(&(val as u32).to_be_bytes());
                count = 0;
            }
            continue;
        }

        return Err(Error::PdfParse(format!(
            "ASCII85Decode: invalid character 0x{:02x}",
            b
        )));
    }

    if !eod_found {
        return Err(Error::PdfParse(
            "ASCII85Decode: missing '~>' EOD marker".to_string(),
        ));
    }

    if count == 1 {
        return Err(Error::PdfParse(
            "ASCII85Decode: invalid partial group of length 1".to_string(),
        ));
    } else if (2..=4).contains(&count) {
        let mut padded = tuple;
        for digit in padded[count..5].iter_mut() {
            *digit = 84; // 84 corresponds to 'u' - '!'
        }
        let mut val: u64 = 0;
        for &digit in &padded {
            val = val * 85 + digit as u64;
        }
        if val > 0xFFFF_FFFF {
            return Err(Error::PdfParse(
                "ASCII85Decode: partial group value exceeds 32 bits (overflow)".to_string(),
            ));
        }
        let bytes = (val as u32).to_be_bytes();
        output.extend_from_slice(&bytes[..count - 1]);
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_decompress_uncompressed() {
        let stream = PdfStream {
            dict: HashMap::new(),
            raw_data: b"Hello World".to_vec(),
        };
        let result = decompress(&stream).unwrap();
        assert_eq!(result, b"Hello World");
    }

    #[test]
    fn test_decompress_flate() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"Hello Compressed").unwrap();
        let compressed = encoder.finish().unwrap();

        let mut dict = HashMap::new();
        dict.insert(b"Filter".to_vec(), PdfObject::Name(b"FlateDecode".to_vec()));

        let stream = PdfStream {
            dict,
            raw_data: compressed,
        };
        let result = decompress(&stream).unwrap();
        assert_eq!(result, b"Hello Compressed");
    }

    #[test]
    fn test_decode_ascii_hex() {
        let result = decode_ascii_hex(b"48 65 6C 6C 6F>").unwrap();
        assert_eq!(result, b"Hello");
    }

    #[test]
    fn test_unsupported_filter() {
        let mut dict = HashMap::new();
        dict.insert(b"Filter".to_vec(), PdfObject::Name(b"LZWDecode".to_vec()));
        let stream = PdfStream {
            dict,
            raw_data: vec![1, 2, 3],
        };
        assert!(decompress(&stream).is_err());
    }

    #[test]
    fn test_decode_ascii85_valid() {
        let cases: &[(&[u8], &[u8])] = &[
            // Empty stream requires valid EOD marker
            (b"~>", b""),
            (b"  \r\n\t~>", b""),
            // Partial groups with valid EOD marker
            (b"8,~>", b"H"),
            (b"87_~>", b"He"),
            (b"87cT~>", b"Hel"),
            // Full 5-tuple
            (b"87cUR~>", b"Hell"),
            // Multi-tuple with partial group
            (b"87cURDZ~>", b"Hello"),
            // Multi-tuple exact
            (b"87cURD]j7BEbo80~>", b"Hello world!"),
            // PDF whitespace interleaved
            (b"8 7\tc\nU\rR\0D\x0C]j7BEbo80~>", b"Hello world!"),
            // 'z' zero shorthand (4 zero bytes)
            (b"z~>", b"\0\0\0\0"),
            (b"zz~>", b"\0\0\0\0\0\0\0\0"),
            (b"87cURz~>", b"Hell\0\0\0\0"),
            // Max 32-bit value (0xFFFFFFFF)
            (b"s8W-!~>", &[0xFF, 0xFF, 0xFF, 0xFF]),
            // Trailing data after valid EOD marker is ignored per PDF filter semantics
            (
                b"87cURD]j7BEbo80~>trailing garbage ignored",
                b"Hello world!",
            ),
            (b"~>trailing", b""),
            (b"z~>extra bytes", b"\0\0\0\0"),
            // Independent valid vectors (pangram and Hobbes quote)
            (
                b"<+ohcEHPu*CER),Dg-(AAoDo:C3=B4F!,CEATAo8BOr<&@=!2AA8c)~>",
                b"The quick brown fox jumps over the lazy dog",
            ),
            (
                b"9jqo^BlbD-BleB1DJ+*+F(f,q/0JhKF<GL>Cj@.4Gp$d7F!,L7@<6@)/0JDEF<G%<+EV:2F!,O<DJ+*.@<*K0@<6L(Df-\\0Ec5e;DffZ(EZee.Bl.9pF\"AGXBPCsi+DGm>@3BB/F*&OCAfu2/AKYi(DIb:@FD,*)+C]U=@3BN#EcYf8ATDL&B-:_rCh[NqF<G:8+EV:.+Cf>-FD5W8ARlolDIal(DId<j@<?3r@:F%a+D58'ATD4$Bl@l3De:,-DJs`8ARoFb/0JMK@qB4^F!,R<AKZ&-DfTqBG%G>uD.RTpAKYo'+CT/5+Cei#DII?(E,9)oF*2M7/c~>",
                b"Man is distinguished, not only by his reason, but by this singular passion from other animals, which is a lust of the mind, that by a persevering delight in the continued and indefatigable generation of knowledge, exceeds the short vehemence of any carnal pleasure.",
            ),
        ];

        for &(input, expected) in cases {
            let actual = decode_ascii85(input).unwrap_or_else(|e| {
                panic!(
                    "failed to decode {:?}: {}",
                    String::from_utf8_lossy(input),
                    e
                )
            });
            assert_eq!(
                actual,
                expected,
                "mismatch for input: {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn test_decode_ascii85_missing_eod_marker() {
        let cases: &[&[u8]] = &[
            // Empty data without marker
            b"",
            // Whitespace only without marker
            b" ",
            b"  \r\n\t \0\x0C",
            // 'z' zero shorthand without marker
            b"z",
            b"zz",
            b"87cURz",
            // Full tuples without marker
            b"87cUR",
            b"87cURD]j7BEbo80",
            // Partial tuples without marker (partial EOF)
            b"8,",
            b"87_",
            b"87cT",
            b"87cURDZ",
        ];

        for &input in cases {
            assert!(
                decode_ascii85(input).is_err(),
                "expected error for missing EOD marker on: {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn test_decode_ascii85_reject_invalid_inputs() {
        let cases: &[&[u8]] = &[
            // 1-digit partial groups
            b"!~>",
            b"!",
            b"87cUR!~>",
            b"87cUR!",
            // Bare or truncated '~'
            b"~",
            b"87cUR~",
            b"~x",
            b"~ ",
            b"87cUR~x",
            // 'z' inside partial tuple
            b"!z~>",
            b"!z",
            b"87z~>",
            // 32-bit overflow (> 0xFFFFFFFF)
            b"uuuuu~>",
            b"s8W-\"~>",
            b"s8W.~>",
            // Non-alphabet characters
            b"87cURy~>",
            b"87cUR\x80~>",
            b"v~>",
            // Adobe opening wrapper (<~ is not accepted in PDF stream filters)
            b"<~87cUR~>",
            b"<~>",
        ];

        for &input in cases {
            assert!(
                decode_ascii85(input).is_err(),
                "expected error for invalid input: {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn test_decode_ascii85_rejects_adobe_opening_wrapper_by_name() {
        // Adobe-wrapped payloads across every partial-group remainder, including
        // whitespace-separated and whitespace-prefixed wrappers. '<' is 0x3C, inside the
        // '!'..='u' alphabet, so without the wrapper guard each of these is rejected by
        // whatever the shifted digit stream trips next and the reason names the wrong byte.
        let cases: &[&[u8]] = &[
            b"<~>",                 // 1 digit
            b"<~8,~>",              // 3 digits
            b"<~87_~>",             // 4 digits
            b"<~87cT~>",            // 5 digits
            b"<~87cUR~>",           // 6 digits
            b"<~87cURDZ~>",         // 8 digits
            b"<~87cURD]j7BEbo80~>", // 16 digits
            b"<~z~>",               // zero shorthand payload
            b"< ~87cURDZ~>",        // whitespace inside the wrapper
            b"  \r\n<~87cURDZ~>",   // whitespace before the wrapper
        ];

        for &input in cases {
            let err = decode_ascii85(input).expect_err(&format!(
                "expected the Adobe wrapper to be rejected: {:?}",
                String::from_utf8_lossy(input)
            ));
            let message = err.to_string();
            assert!(
                message.contains("Adobe '<~' opening wrapper"),
                "rejection of {:?} must name the Adobe wrapper, got: {}",
                String::from_utf8_lossy(input),
                message
            );
        }
    }

    #[test]
    fn test_decode_ascii85_accepts_leading_angle_bracket_digit() {
        // '<' is a base-85 digit: the wrapper guard must key on the two-byte sequence,
        // not on the byte, or valid payloads opening with '<' stop decoding.
        let decoded = decode_ascii85(b"<+ohcEHPu*CER),Dg-(AAoDo:C3=B4F!,CEATAo8BOr<&@=!2AA8c)~>")
            .expect("payload opening with a '<' digit must still decode");
        assert_eq!(decoded, b"The quick brown fox jumps over the lazy dog");
    }

    #[test]
    fn test_decompress_ascii85() {
        let mut dict = HashMap::new();
        dict.insert(b"Filter".to_vec(), PdfObject::Name(b"ASCII85Decode".to_vec()));
        let stream = PdfStream {
            dict,
            raw_data: b"87cURDZ~>".to_vec(),
        };
        let result = decompress(&stream).unwrap();
        assert_eq!(result, b"Hello");
    }

    #[test]
    fn test_decompress_a85_abbreviation() {
        let mut dict = HashMap::new();
        dict.insert(b"Filter".to_vec(), PdfObject::Name(b"A85".to_vec()));
        let stream = PdfStream {
            dict,
            raw_data: b"87cURDZ~>".to_vec(),
        };
        let result = decompress(&stream).unwrap();
        assert_eq!(result, b"Hello");
    }

    #[test]
    fn test_decompress_ascii85_and_flate_chain() {
        let fixture_stream = b"Gap@F5mkI_'L_[Y`@TP.m4T3nSLm\\?7A]f$5jDE$UdC7L]tI9RL:8!@$mCP&+'T,;N;8c)3VWq2dK3Z@E>!KlX33U`\"'\\HZVm=kg2Q\"oK[6UL1NA<sFp1s/'i6GB]jlKo@kMaE(F?q$8L,YoHT9l^qV,Qjc4Sui8q#He9-0,~>";
        let mut dict = HashMap::new();
        dict.insert(
            b"Filter".to_vec(),
            PdfObject::Array(vec![
                PdfObject::Name(b"ASCII85Decode".to_vec()),
                PdfObject::Name(b"FlateDecode".to_vec()),
            ]),
        );
        let stream = PdfStream {
            dict,
            raw_data: fixture_stream.to_vec(),
        };
        let result = decompress(&stream).unwrap();
        assert!(result.starts_with(b"1 0 0 1 0 0 cm"));
        assert!(result
            .windows(b"PUBLIC ASCII85 REGRESSION".len())
            .any(|w| w == b"PUBLIC ASCII85 REGRESSION"));
    }
}
