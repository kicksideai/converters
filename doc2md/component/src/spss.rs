use crate::text::cp1252;
use crate::types::Converted;
use std::collections::BTreeMap;

const MAGIC: &[u8] = b"$FL2";
const MAGIC_Z: &[u8] = b"$FL3";
const HEADER_LEN: usize = 176;
const PREVIEW_ROWS: usize = 20;
const PREVIEW_COLS: usize = 40;

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    little_endian: bool,
}

impl<'a> Reader<'a> {
    fn i32(&mut self) -> Option<i32> {
        let s = self.bytes.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        let a = [s[0], s[1], s[2], s[3]];
        Some(if self.little_endian {
            i32::from_le_bytes(a)
        } else {
            i32::from_be_bytes(a)
        })
    }

    fn f64(&mut self) -> Option<f64> {
        let s = self.bytes.get(self.pos..self.pos + 8)?;
        self.pos += 8;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Some(if self.little_endian {
            f64::from_le_bytes(a)
        } else {
            f64::from_be_bytes(a)
        })
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.bytes.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(s)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        self.pos = self.pos.checked_add(n).filter(|&p| p <= self.bytes.len())?;
        Some(())
    }
}

// SPSS text is the file's locale encoding (often UTF-8 on modern files, otherwise
// Windows-1252). Prefer a clean UTF-8 decode and fall back to cp1252 byte mapping.
fn decode(raw: &[u8]) -> String {
    let raw = raw
        .iter()
        .rposition(|&b| b != b' ' && b != 0)
        .map(|i| &raw[..=i])
        .unwrap_or(&[]);
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => raw.iter().map(|&b| cp1252(b)).collect(),
    }
}

fn fmt_num(value: f64) -> String {
    if value.is_nan() {
        String::new()
    } else if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

struct Variable {
    name: String,
    label: String,
    width: i32, // 0 = numeric, >0 = string byte width
    elements: usize,
    first_element: usize, // 1-based dictionary index of the first element
}

struct Dictionary {
    variables: Vec<Variable>,
    // first_element (1-based) -> value label map
    value_labels: BTreeMap<usize, Vec<(f64, String)>>,
    total_elements: usize,
}

fn parse_dictionary(reader: &mut Reader) -> Result<Dictionary, String> {
    let mut variables: Vec<Variable> = Vec::new();
    let mut element_index = 0usize; // counts every variable record, including continuations
    let mut pending_labels: Vec<(f64, String)> = Vec::new();
    let mut value_labels: BTreeMap<usize, Vec<(f64, String)>> = BTreeMap::new();

    loop {
        let rec = reader.i32().ok_or("truncated record header")?;
        match rec {
            2 => {
                let width = reader.i32().ok_or("variable: width")?;
                let has_label = reader.i32().ok_or("variable: has_label")?;
                let n_missing = reader.i32().ok_or("variable: n_missing")?;
                let _print = reader.i32().ok_or("variable: print")?;
                let _write = reader.i32().ok_or("variable: write")?;
                let name = decode(reader.take(8).ok_or("variable: name")?);
                element_index += 1;

                let mut label = String::new();
                if has_label == 1 {
                    let len = reader.i32().ok_or("variable: label len")? as usize;
                    let padded = len.div_ceil(4) * 4;
                    let raw = reader.take(padded).ok_or("variable: label")?;
                    label = decode(&raw[..len.min(raw.len())]);
                }
                if n_missing != 0 {
                    reader
                        .skip(n_missing.unsigned_abs() as usize * 8)
                        .ok_or("variable: missing values")?;
                }

                if width == -1 {
                    // String continuation record: it occupies one more data element
                    // of the preceding string variable.
                    if let Some(last) = variables.last_mut() {
                        last.elements += 1;
                    }
                } else {
                    variables.push(Variable {
                        name,
                        label,
                        width,
                        elements: 1,
                        first_element: element_index,
                    });
                }
            }
            3 => {
                let count = reader.i32().ok_or("value labels: count")? as usize;
                pending_labels.clear();
                for _ in 0..count {
                    let value = reader.f64().ok_or("value labels: value")?;
                    let len = *reader.take(1).ok_or("value labels: len")?.first().unwrap() as usize;
                    let padded = (len + 1).div_ceil(8) * 8 - 1;
                    let raw = reader.take(padded).ok_or("value labels: text")?;
                    pending_labels.push((value, decode(&raw[..len.min(raw.len())])));
                }
            }
            4 => {
                let var_count = reader.i32().ok_or("label vars: count")? as usize;
                for _ in 0..var_count {
                    let idx = reader.i32().ok_or("label vars: index")? as usize;
                    if !pending_labels.is_empty() {
                        value_labels.insert(idx, pending_labels.clone());
                    }
                }
                pending_labels.clear();
            }
            6 => {
                let lines = reader.i32().ok_or("document: lines")? as usize;
                reader.skip(lines * 80).ok_or("document: body")?;
            }
            7 => {
                let _subtype = reader.i32().ok_or("extension: subtype")?;
                let size = reader.i32().ok_or("extension: size")? as usize;
                let count = reader.i32().ok_or("extension: count")? as usize;
                reader
                    .skip(size.saturating_mul(count))
                    .ok_or("extension: body")?;
            }
            999 => {
                reader.i32().ok_or("terminator: filler")?;
                break;
            }
            other => return Err(format!("unknown record type {other}")),
        }
    }

    let total_elements = variables.iter().map(|v| v.elements).sum();
    Ok(Dictionary {
        variables,
        value_labels,
        total_elements,
    })
}

enum Element {
    Num(f64),
    Str([u8; 8]),
}

// Decode the case data until `max_cases` are gathered. Handles uncompressed
// (compression 0) and the standard bytecode compression (compression 1).
fn read_cases(
    reader: &mut Reader,
    compression: i32,
    bias: f64,
    elements_per_case: usize,
    max_cases: usize,
) -> Vec<Vec<Element>> {
    let mut cases: Vec<Vec<Element>> = Vec::new();
    let mut current: Vec<Element> = Vec::new();
    let mut commands: std::collections::VecDeque<u8> = std::collections::VecDeque::new();

    'outer: while cases.len() < max_cases {
        let element = if compression == 0 {
            match reader.f64() {
                Some(v) => Element::Num(v),
                None => break,
            }
        } else {
            if commands.is_empty() {
                match reader.take(8) {
                    Some(block) => commands.extend(block.iter().copied()),
                    None => break,
                }
            }
            let code = commands.pop_front().unwrap();
            match code {
                0 => continue,
                252 => break 'outer,
                253 => match reader.take(8) {
                    Some(b) => {
                        let mut a = [0u8; 8];
                        a.copy_from_slice(b);
                        // Raw element: numeric reads as f64, strings as bytes; keep both
                        // interpretations available by storing bytes and the double.
                        Element::Str(a)
                    }
                    None => break,
                },
                254 => Element::Str(*b"        "),
                255 => Element::Num(f64::NAN),
                c => Element::Num(c as f64 - bias),
            }
        };
        current.push(element);
        if current.len() == elements_per_case {
            cases.push(std::mem::take(&mut current));
        }
    }
    cases
}

fn element_num(e: &Element, little_endian: bool) -> f64 {
    match e {
        Element::Num(v) => *v,
        Element::Str(b) => {
            if little_endian {
                f64::from_le_bytes(*b)
            } else {
                f64::from_be_bytes(*b)
            }
        }
    }
}

fn element_bytes<'a>(e: &'a Element, scratch: &'a mut [u8; 8]) -> &'a [u8] {
    match e {
        Element::Str(b) => b,
        Element::Num(v) => {
            *scratch = v.to_le_bytes();
            scratch
        }
    }
}

pub fn convert(blob: &[u8]) -> Result<Converted, String> {
    if !(blob.starts_with(MAGIC) || blob.starts_with(MAGIC_Z)) {
        return Err("not an SPSS system file".into());
    }
    if blob.len() < HEADER_LEN {
        return Err("truncated SPSS header".into());
    }

    let little_endian = {
        let le = i32::from_le_bytes([blob[64], blob[65], blob[66], blob[67]]);
        let be = i32::from_be_bytes([blob[64], blob[65], blob[66], blob[67]]);
        if le == 2 {
            true
        } else if be == 2 {
            false
        } else {
            return Err("unrecognized SPSS layout code".into());
        }
    };

    let rd = |o: usize| {
        let a = [blob[o], blob[o + 1], blob[o + 2], blob[o + 3]];
        if little_endian {
            i32::from_le_bytes(a)
        } else {
            i32::from_be_bytes(a)
        }
    };
    let compression = rd(72);
    let ncases = rd(80);
    let bias = {
        let mut a = [0u8; 8];
        a.copy_from_slice(&blob[84..92]);
        if little_endian {
            f64::from_le_bytes(a)
        } else {
            f64::from_be_bytes(a)
        }
    };

    if compression != 0 && compression != 1 {
        return Err(format!(
            "SPSS compression mode {compression} (zsav) is not supported"
        ));
    }

    let mut reader = Reader {
        bytes: blob,
        pos: HEADER_LEN,
        little_endian,
    };
    let dict = parse_dictionary(&mut reader)?;
    if dict.variables.is_empty() {
        return Err("SPSS file has no variables".into());
    }

    let mut md = String::from("# SPSS Data File\n\n");
    md.push_str(&format!("- Variables: {}\n", dict.variables.len()));
    if ncases >= 0 {
        md.push_str(&format!("- Cases: {ncases}\n"));
    }

    md.push_str("\n## Variables\n\n| Name | Type | Label |\n| --- | --- | --- |\n");
    for v in &dict.variables {
        let ty = if v.width == 0 {
            "numeric".to_string()
        } else {
            format!("string({})", v.width)
        };
        md.push_str(&format!(
            "| {} | {} | {} |\n",
            v.name.replace('|', "\\|"),
            ty,
            v.label.replace('|', "\\|").replace('\n', " ")
        ));
    }

    let labelled: Vec<&Variable> = dict
        .variables
        .iter()
        .filter(|v| dict.value_labels.contains_key(&v.first_element))
        .collect();
    if !labelled.is_empty() {
        md.push_str("\n## Value labels\n\n");
        for v in &labelled {
            md.push_str(&format!("- **{}**: ", v.name));
            let labels = &dict.value_labels[&v.first_element];
            let parts: Vec<String> = labels
                .iter()
                .map(|(val, lab)| format!("{}={}", fmt_num(*val), lab))
                .collect();
            md.push_str(&parts.join(", "));
            md.push('\n');
        }
    }

    let cases = read_cases(
        &mut reader,
        compression,
        bias,
        dict.total_elements,
        PREVIEW_ROWS,
    );
    if !cases.is_empty() {
        let cols = dict.variables.len().min(PREVIEW_COLS);
        md.push_str("\n## Data preview\n\n");
        let total = if ncases >= 0 {
            ncases as usize
        } else {
            cases.len()
        };
        if cases.len() < total || dict.variables.len() > cols {
            md.push_str(&format!(
                "> Showing first {} of {} cases and first {} of {} variables.\n\n",
                cases.len(),
                total,
                cols,
                dict.variables.len()
            ));
        }
        md.push_str("| ");
        md.push_str(
            &dict.variables[..cols]
                .iter()
                .map(|v| v.name.replace('|', "\\|"))
                .collect::<Vec<_>>()
                .join(" | "),
        );
        md.push_str(" |\n|");
        for _ in 0..cols {
            md.push_str(" --- |");
        }
        md.push('\n');
        for case in &cases {
            let mut cells: Vec<String> = Vec::with_capacity(cols);
            for v in &dict.variables[..cols] {
                let start = v.first_element - 1;
                cells.push(render_cell(v, &case[start..], little_endian));
            }
            md.push_str("| ");
            md.push_str(&cells.join(" | "));
            md.push_str(" |\n");
        }
    }

    Ok(Converted {
        markdown: md.trim_end().to_string(),
        images: Vec::new(),
    })
}

fn render_cell(v: &Variable, elems: &[Element], little_endian: bool) -> String {
    if v.width == 0 {
        elems
            .first()
            .map(|e| fmt_num(element_num(e, little_endian)))
            .unwrap_or_default()
    } else {
        let mut bytes = Vec::new();
        for e in elems.iter().take(v.elements) {
            let mut s = [0u8; 8];
            bytes.extend_from_slice(element_bytes(e, &mut s));
        }
        decode(&bytes).replace('|', "\\|").replace('\n', " ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_trims_and_falls_back_to_cp1252() {
        assert_eq!(decode(b"hello   "), "hello");
        assert_eq!(decode(&[0xe9, b' ']), "é"); // cp1252 0xE9
    }

    #[test]
    fn fmt_num_formats_integers_and_floats_and_nan() {
        assert_eq!(fmt_num(3.0), "3");
        assert_eq!(fmt_num(2.5), "2.5");
        assert_eq!(fmt_num(f64::NAN), "");
    }

    // Build a minimal but spec-valid little-endian, bytecode-compressed `.sav`:
    // numeric AGE (with label), numeric SEX (with value labels), string NAME
    // (width 12 -> one continuation element), a document record and an extension
    // record, and two cases. Exercises every dictionary record path.
    fn build_sav() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"$FL2");
        b.extend_from_slice(&[b' '; 60]); // prod_name
        b.extend_from_slice(&2i32.to_le_bytes()); // layout_code
        b.extend_from_slice(&4i32.to_le_bytes()); // nominal_case_size (AGE+SEX+NAME[2])
        b.extend_from_slice(&1i32.to_le_bytes()); // compression
        b.extend_from_slice(&0i32.to_le_bytes()); // weight_index
        b.extend_from_slice(&2i32.to_le_bytes()); // ncases
        b.extend_from_slice(&100.0f64.to_le_bytes()); // bias
        b.extend_from_slice(b"01 Jan 25"); // creation_date (9)
        b.extend_from_slice(b"00:00:00"); // creation_time (8)
        b.extend_from_slice(&[b' '; 64]); // file_label
        b.extend_from_slice(&[0u8; 3]); // padding
        assert_eq!(b.len(), HEADER_LEN);

        let var = |name: &[u8; 8],
                   width: i32,
                   label: Option<&str>,
                   missing: &[f64],
                   out: &mut Vec<u8>| {
            out.extend_from_slice(&2i32.to_le_bytes()); // rec_type
            out.extend_from_slice(&width.to_le_bytes());
            out.extend_from_slice(&(label.is_some() as i32).to_le_bytes()); // has_label
            out.extend_from_slice(&(missing.len() as i32).to_le_bytes()); // n_missing
            out.extend_from_slice(&0i32.to_le_bytes()); // print
            out.extend_from_slice(&0i32.to_le_bytes()); // write
            out.extend_from_slice(name);
            if let Some(l) = label {
                out.extend_from_slice(&(l.len() as i32).to_le_bytes());
                let mut lab = l.as_bytes().to_vec();
                while lab.len() % 4 != 0 {
                    lab.push(b' ');
                }
                out.extend_from_slice(&lab);
            }
            for m in missing {
                out.extend_from_slice(&m.to_le_bytes());
            }
        };
        var(b"AGE     ", 0, Some("Age in years"), &[99.0], &mut b); // one missing value
        var(b"SEX     ", 0, None, &[], &mut b);
        var(b"NAME    ", 12, Some("Full name"), &[], &mut b);
        var(b"        ", -1, None, &[], &mut b); // string continuation element

        // Value labels for SEX: 1=Male, 2=Female.
        b.extend_from_slice(&3i32.to_le_bytes()); // rec_type
        b.extend_from_slice(&2i32.to_le_bytes()); // count
        for (val, text) in [(1.0f64, "Male"), (2.0f64, "Female")] {
            b.extend_from_slice(&val.to_le_bytes());
            b.push(text.len() as u8);
            let mut t = text.as_bytes().to_vec();
            while (t.len() + 1) % 8 != 0 {
                t.push(b' ');
            }
            b.extend_from_slice(&t);
        }
        b.extend_from_slice(&4i32.to_le_bytes()); // rec_type
        b.extend_from_slice(&1i32.to_le_bytes()); // var_count
        b.extend_from_slice(&2i32.to_le_bytes()); // element index of SEX (1-based)

        // Document record: one 80-byte line.
        b.extend_from_slice(&6i32.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&[b' '; 80]);

        // Extension record: subtype 3, 4-byte * 2-count body.
        b.extend_from_slice(&7i32.to_le_bytes());
        b.extend_from_slice(&3i32.to_le_bytes()); // subtype
        b.extend_from_slice(&4i32.to_le_bytes()); // size
        b.extend_from_slice(&2i32.to_le_bytes()); // count
        b.extend_from_slice(&[0u8; 8]);

        b.extend_from_slice(&999i32.to_le_bytes()); // dictionary terminator
        b.extend_from_slice(&0i32.to_le_bytes()); // filler

        // Compressed data, two cases of (AGE, SEX, NAME). Numeric: code = value+bias.
        // String elements: 253 = next 8 raw bytes follow the octet, 254 = 8 spaces.
        b.extend_from_slice(&[130, 101, 253, 254, 125, 102, 253, 254]); // octet
        b.extend_from_slice(b"Alice   "); // raw for case 1 NAME
        b.extend_from_slice(b"Bob     "); // raw for case 2 NAME
        b.extend_from_slice(&[252, 0, 0, 0, 0, 0, 0, 0]); // EOF octet
        b
    }

    #[test]
    fn converts_synthetic_sav_with_metadata_labels_and_data() {
        let c = convert(&build_sav()).unwrap();
        assert!(c.markdown.contains("- Variables: 3"));
        assert!(c.markdown.contains("- Cases: 2"));
        assert!(c.markdown.contains("| AGE | numeric | Age in years |"));
        assert!(c.markdown.contains("| SEX | numeric |  |"));
        assert!(c.markdown.contains("| NAME | string(12) | Full name |"));
        assert!(c.markdown.contains("**SEX**: 1=Male, 2=Female"));
        assert!(c.markdown.contains("| AGE | SEX | NAME |"));
        assert!(c.markdown.contains("| 30 | 1 | Alice |"));
        assert!(c.markdown.contains("| 25 | 2 | Bob |"));
    }

    #[test]
    fn rejects_non_sav_and_unsupported_compression() {
        assert!(convert(b"not a sav file at all").is_err());
        let mut zsav = build_sav();
        zsav[72..76].copy_from_slice(&2i32.to_le_bytes()); // compression = 2 (zsav)
        let err = convert(&zsav).unwrap_err();
        assert!(err.contains("zsav"), "got: {err}");
    }

    #[test]
    fn rejects_truncated_header() {
        // Correct magic but fewer than HEADER_LEN bytes.
        let err = convert(b"$FL2 short").unwrap_err();
        assert!(err.contains("truncated"), "got: {err}");
    }
}
