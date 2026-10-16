// Windows-1252 high range (0x80-0x9F) to Unicode; 0xA0-0xFF map 1:1 to Latin-1.
pub fn cp1252(byte: u8) -> char {
    const HIGH: [u16; 32] = [
        0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022,
        0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
    ];
    match byte {
        0x80..=0x9F => char::from_u32(HIGH[(byte - 0x80) as usize] as u32).unwrap_or('\u{FFFD}'),
        other => other as char,
    }
}

pub fn push_char(out: &mut String, c: char) {
    match c {
        '\r' | '\u{000B}' | '\u{000C}' => out.push('\n'),
        '\u{0007}' => out.push('\t'),
        '\u{001E}' => out.push('-'),
        '\u{00A0}' => out.push(' '),
        '\u{0002}' | '\u{001F}' => {}
        c if (c as u32) < 0x20 && c != '\n' => {}
        c => out.push(c),
    }
}

pub fn tidy(text: &str) -> String {
    let mut lines: Vec<String> = text.split('\n').map(|l| l.trim_end().to_string()).collect();
    lines.dedup_by(|a, b| a.is_empty() && b.is_empty());
    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp1252_maps_ascii_and_high_range() {
        assert_eq!(cp1252(b'A'), 'A');
        assert_eq!(cp1252(0x80), '\u{20AC}');
        assert_eq!(cp1252(0x93), '\u{201C}');
        assert_eq!(cp1252(0xE9), 'é');
    }

    #[test]
    fn push_char_normalizes_control_chars() {
        let mut s = String::new();
        push_char(&mut s, '\r');
        push_char(&mut s, '\u{0007}');
        push_char(&mut s, '\u{00A0}');
        push_char(&mut s, '\u{0001}');
        push_char(&mut s, 'x');
        assert_eq!(s, "\n\t x");
    }

    #[test]
    fn tidy_collapses_blank_lines_and_trims() {
        assert_eq!(tidy("\n\na\n\n\n b \n\n"), "a\n\n b");
    }
}
