# Vendored unpdf 0.7.0

## Provenance

- Crate: `unpdf` 0.7.0 from crates.io
- Source archive: https://static.crates.io/crates/unpdf/unpdf-0.7.0.crate
- SHA-256: `1601aff0f5e2910a32e195ad8093b581b01b2c1d76daa6caaa51eb2265c35990`
- Matches the original registry checksum in `doc2md/component/Cargo.lock` before vendoring
- License: MIT (`LICENSE` in this directory)

The tree is the unpacked crates.io source. Bindings, benches, cmap tables, and
upstream tests are kept. `target/` and other build debris are not included.

## Patches

### 1. Heading Determinism Tie-Break (`src/parser/layout.rs`)

`src/parser/layout.rs` `FontStatistics::analyze` used
`HashMap::iter().max_by_key(|(_, count)| *count)` to pick body font size.
Rust's default hasher randomizes iteration order, so equal-frequency font sizes
could choose different winners across conversions, including within one process. Heading markers in Markdown then flipped
across runs of the same PDF.

On a count tie the comparison now also uses `Reverse(font_size_key)`, so the
smaller font is body text and the larger font can still qualify as a heading.
The choice is independent of HashMap iteration order.

### 2. ASCII85Decode Stream Filter (`src/parser/raw/stream.rs`)

Upstream `unpdf` 0.7.0 `src/parser/raw/stream.rs` only supported `FlateDecode` (`Fl`)
and `ASCIIHexDecode` (`AHx`). PDF streams with `ASCII85Decode` or `A85` (such as
multi-stage compressed filter chains like `[ /ASCII85Decode /FlateDecode ]`) failed to
decompress, returning empty content streams.

Implemented spec-compliant, bounded-by-input private decoder `decode_ascii85` per ISO 32000-1 §7.4.3:
- Decoder is kept private (`fn decode_ascii85`), consistent with `decode_ascii_hex` and `decompress_flate`.
- Supports filter names `ASCII85Decode` and standard abbreviation `A85`.
- Ignores PDF whitespace characters: NUL (0x00), TAB (0x09), LF (0x0A), FF (0x0C), CR (0x0D), SP (0x20).
- Strict EOD marker validation: requires the `~>` EOD marker on all streams, including empty data, whitespace-only, full tuples, and partial groups. Rejects missing EOD marker, bare/truncated `~`, and partial groups at EOF (partialEOF) rather than accepting silently.
- Trailing data policy: once valid `~>` EOD is encountered, decoding terminates successfully; any trailing data following `~>` in the stream buffer is ignored per PDF filter stream semantics.
- Adobe `<~` opening wrapper is strictly rejected per PDF filter semantics (PDF stream boundaries are delimited by `stream`/`endstream` keywords). See patch 3 for the check that carries this.
- Supports zero shorthand `'z'` (representing 4 zero bytes) only at tuple boundaries (`count == 0`); rejects `'z'` inside partial tuples.
- Supports partial groups of lengths 2..=4 (padded with `'u'`, decoded to 32-bit big-endian, producing length - 1 bytes).
- Strict validation: rejects 1-digit partial groups, 32-bit overflow (> 0xFFFFFFFF) on 5-tuples and partial groups, non-alphabet characters, and truncated/malformed terminators.
- Tested via table-driven unit tests in `stream.rs` (covering valid vectors, explicit missing-marker cases for empty/z/full/partial/whitespace, and invalid inputs) and an integration test in `doc2md/component/src/lib.rs`.

### 3. Adobe `<~` Wrapper Guard (`src/parser/raw/stream.rs`)

Patch 2 documented the Adobe PostScript opening wrapper `<~` as strictly rejected, but
carried no check for it. `<` is 0x3C, inside the `'!'..='u'` base-85 alphabet, so the digit
loop consumed it as data and rejection fell to whichever later rule the shifted digit
stream happened to break: the `~` of the wrapper failed the `~>` terminator test, and
`<~>` failed the 1-digit partial-group test. Both guard cases in
`test_decode_ascii85_reject_invalid_inputs` passed on those unrelated rules, and every
rejection named the wrong byte.

`decode_ascii85` now calls `starts_with_adobe_wrapper` before consuming any digit and
rejects the input as `ASCII85Decode: Adobe '<~' opening wrapper is not valid in a PDF
stream filter`. The helper reads the first two non-whitespace bytes, since PDF whitespace
is ignored anywhere in an ASCII85 stream. Keying on the two-byte sequence keeps `<` usable
as the base-85 digit it is, which `test_decode_ascii85_accepts_leading_angle_bracket_digit`
holds; `test_decode_ascii85_rejects_adobe_opening_wrapper_by_name` asserts the rejection
names the wrapper across payload lengths, `z` payloads, and whitespace-separated and
whitespace-prefixed wrappers.

No other unpdf sources are modified.
