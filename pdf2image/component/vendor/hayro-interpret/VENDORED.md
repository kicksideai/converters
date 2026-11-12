# Vendored hayro-interpret 0.7.0

## Provenance

- Crate: `hayro-interpret` 0.7.0 from crates.io
- Source archive: https://static.crates.io/crates/hayro-interpret/hayro-interpret-0.7.0.crate
- SHA-256: `f2613d0406898995042d0794c4245b2f2fba1246490c8ac593769bba6551129d`
- Matches the original registry checksum in `pdf2image/component/Cargo.lock` before vendoring
- Upstream commit: `0c98904d2ced8e37b63ed2bd1e60f95bc0c7c774` (`hayro-interpret`, recorded in `.cargo_vcs_info.json`)
- License: Apache-2.0 OR MIT (`LICENSE-APACHE` and `LICENSE-MIT` in this directory)

The tree is the unpacked crates.io source. Assets, examples, and upstream tests
are kept. `target/` and other build debris are not included.

Only `hayro-interpret` needs a patch: it owns image XObject painting and the
existing colorant predicate. `hayro` 0.7.1 and every other dependency remain on
their existing registry versions. The component's `[patch.crates-io]` selects
this local source for the transitive interpreter dependency.

## Patches

### 1. Nonpainting Image Colorants (`src/x_object.rs`)

`draw_image_xobject` previously submitted every non-stencil image as an opaque
raster, including Separation `/None` and DeviceN spaces with only `/None`
colorants. Converting their samples to RGB discards the no-paint semantics and
can obscure content underneath.

Before painting a non-stencil image, the interpreter now uses the same
`ToRgb::is_none` predicate as scalar paints. Separation `/None` and DeviceN
spaces whose colorants are all `/None` leave the page untouched, as required
by ISO 32000-1 sections 8.6.6.4 and 8.6.6.5. Stencil images retain their existing
paint behavior. Real Separation colorants and mixed DeviceN colorants retain
the existing conversion and painting path.
The same rule covers inline images, including named color spaces resolved through page resources.

The component's generated `tests/assets/colorant-images.pdf` overlays a
synthetic scanned pattern. Native regressions compare nonpainting overlays to
the original scan and real or mixed colorants to an opaque DeviceGray control.

### 2. README Whitespace (`README.md`)

Remove the trailing space on the final license sentence so the imported tree
passes Git whitespace checks. The text is unchanged.

No other hayro-interpret sources are modified.

## Render comparison

From the repository root, compare committed revisions with:

```sh
python3 pdf2image/component/tests/compare_renders.py \
  --baseline origin/main --candidate HEAD --work-dir /path/to/new/comparison-directory
```

The script extracts each revision into the supplied directory, retains its Cargo
lock, and runs the same native snapshot test against both components. It compares
encoded bytes directly and records each output's SHA-256 in `comparison.json`.
Inputs are the existing `sample.pdf` component asset and the `pdf` and `text_pdf`
Lua fixtures (four pages total). Each page renders at scales 1 and 2.0833333
(72 and 150 DPI), with white and transparent backgrounds, in PNG, JPEG (quality
90), and lossless WebP. Other rendering settings and embedded font resolution
use the component defaults.

The pre-vendoring `origin/main` snapshot
`78ff12add8fdaa87c6b2937d4bc77551d5cd7a20` and merged patch revision
`1497ff265d148b25bcd6d25b20ca3413f959a33e` produce 48 byte-identical outputs
with rustc 1.96.0. Reproduce that comparison by passing those commit hashes as
`--baseline` and `--candidate`. The sorted output manifest (filename, byte length,
SHA-256; JSON with sorted keys) has SHA-256
`9a9bf0c8c719909129d1c1b199b2c549e0290e37043f9c31cd204f94293192e9`.
