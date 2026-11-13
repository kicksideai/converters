//! PDF content stream parser.

use super::tokenizer::{parse_object, PdfDict, PdfObject};
use crate::error::{Error, Result};
use crate::parser::backend::{ContentOp, PdfValue};

/// Parse a content stream into a sequence of operations.
pub fn parse_content_stream(data: &[u8]) -> Result<Vec<ContentOp>> {
    parse_content_stream_with_colors(data, |_| None)
}

pub(crate) fn parse_content_stream_with_colors(
    data: &[u8],
    color_components: impl Fn(&PdfObject) -> Option<usize>,
) -> Result<Vec<ContentOp>> {
    let mut ops = Vec::new();
    let mut operand_stack: Vec<PdfValue> = Vec::new();
    let len = data.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace
        if is_whitespace(data[i]) {
            i += 1;
            continue;
        }

        // Skip PDF comments (% until end of line)
        if data[i] == b'%' {
            while i < len && data[i] != b'\n' && data[i] != b'\r' {
                i += 1;
            }
            continue;
        }

        // Literal string (...)
        if data[i] == b'(' {
            let (val, next) = parse_literal_string(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Hex string <...>
        if data[i] == b'<' && i + 1 < len && data[i + 1] != b'<' {
            let (val, next) = parse_hex_string(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Array [...]
        if data[i] == b'[' {
            let (val, next) = parse_array(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Name /Foo
        if data[i] == b'/' {
            let (val, next) = parse_name(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Number (integer or real), or negative sign
        if data[i] == b'-' || data[i] == b'+' || data[i] == b'.' || data[i].is_ascii_digit() {
            let (val, next) = parse_number(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Alphabetic token → could be operator or keyword (true/false/null)
        if is_operator_start(data[i]) {
            let start = i;
            // Collect alphabetic characters
            while i < len && data[i].is_ascii_alphabetic() {
                i += 1;
            }
            // Check for * suffix (T*, b*, B*)
            if i < len && data[i] == b'*' {
                i += 1;
            }

            let token = &data[start..i];
            let token_str = std::str::from_utf8(token).unwrap_or("");

            match token_str {
                "true" | "false" | "null" => {
                    operand_stack.push(PdfValue::Other);
                }
                "BI" => {
                    // Consume image data using its sample layout or filter boundary.
                    i = skip_inline_image(data, i, &color_components)?;
                    // Emit BI as an operator with no operands (stack should be empty)
                    ops.push(ContentOp {
                        operator: "BI".to_string(),
                        operands: std::mem::take(&mut operand_stack),
                    });
                }
                _ => {
                    // It's an operator
                    ops.push(ContentOp {
                        operator: token_str.to_string(),
                        operands: std::mem::take(&mut operand_stack),
                    });
                }
            }
            continue;
        }

        // Special single-char operators: ' and "
        if data[i] == b'\'' || data[i] == b'"' {
            let op = (data[i] as char).to_string();
            i += 1;
            ops.push(ContentOp {
                operator: op,
                operands: std::mem::take(&mut operand_stack),
            });
            continue;
        }

        // Skip unknown bytes
        i += 1;
    }

    Ok(ops)
}

fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0 | 12)
}

fn is_operator_start(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    ) || is_whitespace(b)
}

/// Parse a literal string starting at `(`, returns (PdfValue, next_index).
fn parse_literal_string(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip opening '('
    let mut result = Vec::new();
    let mut depth = 1;
    let len = data.len();

    while i < len && depth > 0 {
        match data[i] {
            b'(' => {
                depth += 1;
                result.push(b'(');
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth > 0 {
                    result.push(b')');
                }
                i += 1;
            }
            b'\\' if i + 1 < len => {
                i += 1;
                match data[i] {
                    b'n' => {
                        result.push(b'\n');
                        i += 1;
                    }
                    b'r' => {
                        result.push(b'\r');
                        i += 1;
                    }
                    b't' => {
                        result.push(b'\t');
                        i += 1;
                    }
                    b'b' => {
                        result.push(8); // backspace
                        i += 1;
                    }
                    b'f' => {
                        result.push(12); // form feed
                        i += 1;
                    }
                    b'(' => {
                        result.push(b'(');
                        i += 1;
                    }
                    b')' => {
                        result.push(b')');
                        i += 1;
                    }
                    b'\\' => {
                        result.push(b'\\');
                        i += 1;
                    }
                    c if c.is_ascii_digit() => {
                        // Octal escape
                        let mut octal = (c - b'0') as u16;
                        i += 1;
                        for _ in 0..2 {
                            if i < len && data[i].is_ascii_digit() && data[i] <= b'7' {
                                octal = octal * 8 + (data[i] - b'0') as u16;
                                i += 1;
                            } else {
                                break;
                            }
                        }
                        result.push(octal as u8);
                    }
                    b'\r' => {
                        // Backslash + CR (+ optional LF) = line continuation
                        i += 1;
                        if i < len && data[i] == b'\n' {
                            i += 1;
                        }
                    }
                    b'\n' => {
                        // Backslash + LF = line continuation
                        i += 1;
                    }
                    _ => {
                        // Unknown escape, just include the character
                        result.push(data[i]);
                        i += 1;
                    }
                }
            }
            _ => {
                result.push(data[i]);
                i += 1;
            }
        }
    }

    (PdfValue::Str(result), i)
}

/// Parse a hex string starting at `<`, returns (PdfValue, next_index).
fn parse_hex_string(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip '<'
    let len = data.len();
    let mut hex_chars = Vec::new();

    while i < len && data[i] != b'>' {
        if !is_whitespace(data[i]) {
            hex_chars.push(data[i]);
        }
        i += 1;
    }
    if i < len {
        i += 1; // skip '>'
    }

    // Pad odd-length hex with trailing 0
    if hex_chars.len() % 2 != 0 {
        hex_chars.push(b'0');
    }

    let mut result = Vec::with_capacity(hex_chars.len() / 2);
    for pair in hex_chars.chunks(2) {
        let hi = hex_digit(pair[0]);
        let lo = hex_digit(pair[1]);
        result.push((hi << 4) | lo);
    }

    (PdfValue::Str(result), i)
}

fn hex_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

/// Parse a name starting at `/`, returns (PdfValue, next_index).
fn parse_name(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip '/'
    let len = data.len();
    let mut name = Vec::new();

    while i < len && !is_whitespace(data[i]) && !is_delimiter(data[i]) {
        if data[i] == b'#' && i + 2 < len {
            // Hex escape in name
            let hi = hex_digit(data[i + 1]);
            let lo = hex_digit(data[i + 2]);
            name.push((hi << 4) | lo);
            i += 3;
        } else {
            name.push(data[i]);
            i += 1;
        }
    }

    (PdfValue::Name(name), i)
}

/// Parse a number (integer or real), returns (PdfValue, next_index).
fn parse_number(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start;
    let len = data.len();
    let mut has_dot = false;

    // Sign
    if i < len && (data[i] == b'-' || data[i] == b'+') {
        i += 1;
    }

    // Check for leading dot
    if i < len && data[i] == b'.' {
        has_dot = true;
        i += 1;
    }

    while i < len && (data[i].is_ascii_digit() || data[i] == b'.') {
        if data[i] == b'.' {
            has_dot = true;
        }
        i += 1;
    }

    let s = std::str::from_utf8(&data[start..i]).unwrap_or("0");

    if has_dot {
        let val: f32 = s.parse().unwrap_or(0.0);
        (PdfValue::Real(val), i)
    } else {
        let val: i64 = s.parse().unwrap_or(0);
        (PdfValue::Integer(val), i)
    }
}

/// Parse an array starting at `[`, returns (PdfValue, next_index).
fn parse_array(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip '['
    let len = data.len();
    let mut elements = Vec::new();

    while i < len {
        // Skip whitespace
        if is_whitespace(data[i]) {
            i += 1;
            continue;
        }

        if data[i] == b']' {
            i += 1;
            break;
        }

        // Literal string
        if data[i] == b'(' {
            let (val, next) = parse_literal_string(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Hex string
        if data[i] == b'<' && i + 1 < len && data[i + 1] != b'<' {
            let (val, next) = parse_hex_string(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Nested array
        if data[i] == b'[' {
            let (val, next) = parse_array(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Name
        if data[i] == b'/' {
            let (val, next) = parse_name(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Number
        if data[i] == b'-' || data[i] == b'+' || data[i] == b'.' || data[i].is_ascii_digit() {
            let (val, next) = parse_number(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Keywords (true/false/null) inside arrays
        if data[i].is_ascii_alphabetic() {
            let token_start = i;
            while i < len && data[i].is_ascii_alphabetic() {
                i += 1;
            }
            let token = std::str::from_utf8(&data[token_start..i]).unwrap_or("");
            match token {
                "true" | "false" | "null" => {
                    elements.push(PdfValue::Other);
                }
                _ => {
                    // Names without / in arrays are unusual; treat as Other
                    elements.push(PdfValue::Other);
                }
            }
            continue;
        }

        // Skip unknown
        i += 1;
    }

    (PdfValue::Array(elements), i)
}

/// Parse the inline dictionary before consuming image data. Sample bytes can
/// contain any content-stream token, including whitespace-delimited `EI`.
fn skip_inline_image(
    data: &[u8],
    start: usize,
    colors: &impl Fn(&PdfObject) -> Option<usize>,
) -> Result<usize> {
    let mut dict = PdfDict::new();
    let mut i = start;
    loop {
        while i < data.len() && is_whitespace(data[i]) {
            i += 1;
        }
        if data.get(i) == Some(&b'%') {
            while i < data.len() && !matches!(data[i], b'\r' | b'\n') {
                i += 1;
            }
            continue;
        }
        if data.get(i..i + 2) == Some(b"ID") && data.get(i + 2).is_some_and(|b| is_whitespace(*b)) {
            i += 3;
            if data.get(i - 1..i + 1) == Some(b"\r\n") {
                i += 1;
            }
            break;
        }
        let (key, next) = parse_object(data, i)?;
        let (value, next) = parse_object(data, next)?;
        let PdfObject::Name(key) = key else {
            return Err(Error::PdfParse("invalid inline image dictionary".into()));
        };
        dict.insert(key, value);
        i = next;
    }
    let get = |short: &[u8], full: &[u8]| dict.get(short).or_else(|| dict.get(full));
    let filter = get(b"F", b"Filter")
        .and_then(|f| match f {
            PdfObject::Array(filters) => filters.first(),
            value => Some(value),
        })
        .and_then(PdfObject::as_name);
    let samples = &data[i..];
    let parms = get(b"DP", b"DecodeParms")
        .and_then(|p| match p {
            PdfObject::Array(values) => values.first(),
            value => Some(value),
        })
        .and_then(PdfObject::as_dict);
    let size = get(b"Length", b"Length")
        .and_then(PdfObject::as_i64)
        .and_then(|n| usize::try_from(n).ok());
    let size = if let Some(size) = size {
        Some(size)
    } else {
        match filter {
            None => {
                let number = |short, full| {
                    get(short, full)
                        .and_then(PdfObject::as_i64)
                        .and_then(|n| usize::try_from(n).ok())
                };
                let mask = matches!(get(b"IM", b"ImageMask"), Some(PdfObject::Bool(true)));
                let components = if mask {
                    Some(1usize)
                } else {
                    match get(b"CS", b"ColorSpace") {
                        Some(PdfObject::Name(name)) => match name.as_slice() {
                            b"G" | b"DeviceGray" => Some(1),
                            b"RGB" | b"DeviceRGB" => Some(3),
                            b"CMYK" | b"DeviceCMYK" => Some(4),
                            _ => colors(get(b"CS", b"ColorSpace").unwrap()),
                        },
                        Some(PdfObject::Array(array))
                            if array
                                .first()
                                .and_then(PdfObject::as_name)
                                .is_some_and(|n| n == b"I" || n == b"Indexed") =>
                        {
                            Some(1)
                        }
                        value => value.and_then(colors),
                    }
                };
                let bits = if mask {
                    Some(1)
                } else {
                    number(b"BPC", b"BitsPerComponent")
                };
                number(b"W", b"Width")
                    .zip(number(b"H", b"Height"))
                    .zip(bits)
                    .zip(components)
                    .and_then(|(((w, h), bpc), c)| {
                        w.checked_mul(c)?
                            .checked_mul(bpc)?
                            .checked_add(7)?
                            .checked_div(8)?
                            .checked_mul(h)
                    })
            }
            Some(b"AHx" | b"ASCIIHexDecode") => {
                samples.iter().position(|b| *b == b'>').map(|n| n + 1)
            }
            Some(b"A85" | b"ASCII85Decode") => {
                samples.windows(2).position(|b| b == b"~>").map(|n| n + 2)
            }
            Some(b"Fl" | b"FlateDecode") => {
                let mut decoder = flate2::bufread::ZlibDecoder::new(samples);
                std::io::copy(&mut decoder, &mut std::io::sink())
                    .map_err(|e| Error::PdfParse(format!("invalid inline Flate image: {e}")))?;
                Some(decoder.total_in() as usize)
            }
            Some(b"RL" | b"RunLengthDecode") => {
                let mut n = 0;
                loop {
                    let Some(&length) = samples.get(n) else {
                        break None;
                    };
                    n += 1;
                    match length {
                        128 => break Some(n),
                        0..=127 => n += usize::from(length) + 1,
                        _ => n += 1,
                    }
                }
            }
            Some(b"DCT" | b"DCTDecode") => jpeg_length(samples),
            Some(b"LZW" | b"LZWDecode") => {
                let early = parms
                    .and_then(|d| d.get(b"EarlyChange".as_slice()))
                    .and_then(PdfObject::as_i64)
                    .unwrap_or(1);
                lzw_length(samples, early as usize)
            }
            Some(b"CCF" | b"CCITTFaxDecode") => {
                use hayro_ccitt::{DecodeSettings, Decoder, DecoderContext, EncodingMode};
                struct Rows;
                impl Decoder for Rows {
                    fn push_row(&mut self, _: &[u32]) {}
                }
                let param = |key: &[u8]| parms.and_then(|p| p.get(key));
                let number =
                    |key, default| param(key).and_then(PdfObject::as_i64).unwrap_or(default);
                let flag = |key, default| match param(key) {
                    Some(PdfObject::Bool(b)) => *b,
                    _ => default,
                };
                let settings = DecodeSettings {
                    columns: number(b"Columns", 1728) as u32,
                    rows: number(b"Rows", 0) as u32,
                    end_of_block: flag(b"EndOfBlock", true),
                    end_of_line: flag(b"EndOfLine", false),
                    rows_are_byte_aligned: flag(b"EncodedByteAlign", false),
                    encoding: match number(b"K", 0) {
                        n if n < 0 => EncodingMode::Group4,
                        0 => EncodingMode::Group3_1D,
                        _ => EncodingMode::Group3_2D,
                    },
                    strict: true,
                };
                Some(hayro_ccitt::decode(samples, &mut Rows, &mut DecoderContext::new(settings))
                    .map_err(|e| Error::PdfParse(format!("invalid inline CCITT image: {e:?}")))?)
            }
            _ => None,
        }
    }
    .ok_or_else(|| Error::PdfParse("cannot determine inline image data length".into()))?;
    i = i
        .checked_add(size)
        .filter(|end| *end <= data.len())
        .ok_or_else(|| Error::PdfParse("truncated inline image data".into()))?;
    while i < data.len() && is_whitespace(data[i]) {
        i += 1;
    }
    if data.get(i..i + 2) != Some(b"EI") || data.get(i + 2).is_some_and(|b| !is_delimiter(*b)) {
        return Err(Error::PdfParse("missing inline image EI delimiter".into()));
    }
    Ok(i + 2)
}

/// LZW code widths depend on dictionary size, independently of pixel values.
fn lzw_length(data: &[u8], early_change: usize) -> Option<usize> {
    let (mut bit, mut width, mut next, mut previous) = (0, 9, 258, false);
    loop {
        let mut code = 0;
        for _ in 0..width {
            code = (code << 1) | usize::from((data.get(bit / 8)? >> (7 - bit % 8)) & 1);
            bit += 1;
        }
        match code {
            256 => {
                width = 9;
                next = 258;
                previous = false;
            }
            257 => return Some((bit + 7) / 8),
            _ => {
                if previous && next < 4096 {
                    next += 1;
                    if next + early_change == 1 << width && width < 12 {
                        width += 1;
                    }
                }
                previous = true;
            }
        }
    }
}

/// JPEG segments carry their own lengths; entropy data escapes literal FF bytes.
fn jpeg_length(data: &[u8]) -> Option<usize> {
    if !data.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut i = 2;
    while i < data.len() {
        if data[i] != 0xff {
            i += 1;
            continue;
        }
        while data.get(i) == Some(&0xff) {
            i += 1;
        }
        let marker = *data.get(i)?;
        i += 1;
        match marker {
            0xd9 => return Some(i),
            0 | 0xd0..=0xd8 | 0x01 => (),
            _ => {
                let length = u16::from_be_bytes([*data.get(i)?, *data.get(i + 1)?]) as usize;
                if length < 2 {
                    return None;
                }
                i = i.checked_add(length)?;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_text_ops() {
        let data = b"BT /F1 12 Tf 100 700 Td (Hello World) Tj ET";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops.len(), 5);
        assert_eq!(ops[0].operator, "BT");
        assert_eq!(ops[0].operands.len(), 0);
        assert_eq!(ops[1].operator, "Tf");
        assert_eq!(ops[1].operands.len(), 2);
        assert_eq!(ops[2].operator, "Td");
        assert_eq!(ops[3].operator, "Tj");
        assert_eq!(ops[4].operator, "ET");
    }

    #[test]
    fn test_parse_tj_array() {
        let data = b"BT [(Hello) -100 (World)] TJ ET";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops.len(), 3); // BT, TJ, ET
        let tj = &ops[1];
        assert_eq!(tj.operator, "TJ");
        assert_eq!(tj.operands.len(), 1); // one array operand
    }

    #[test]
    fn test_parse_graphics_ops() {
        let data = b"q 1 0 0 1 72 720 cm Q";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops[0].operator, "q");
        assert_eq!(ops[1].operator, "cm");
        assert_eq!(ops[1].operands.len(), 6);
        assert_eq!(ops[2].operator, "Q");
    }

    #[test]
    fn test_parse_real_numbers() {
        let data = b"0.5 0.5 0.5 rg";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops[0].operator, "rg");
        assert_eq!(ops[0].operands.len(), 3);
    }

    #[test]
    fn test_empty_content_stream() {
        let ops = parse_content_stream(b"").unwrap();
        assert!(ops.is_empty());
    }

    #[test]
    fn test_tstar_operator() {
        let data = b"BT T* ET";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops[1].operator, "T*");
    }
}
