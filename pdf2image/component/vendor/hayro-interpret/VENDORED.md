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

The component's generated `tests/assets/colorant-images.pdf` overlays a
synthetic scanned pattern. Native regressions compare nonpainting overlays to
the original scan and real or mixed colorants to an opaque DeviceGray control.

### 2. README Whitespace (`README.md`)

Remove the trailing space on the final license sentence so the imported tree
passes Git whitespace checks. The text is unchanged.

No other hayro-interpret sources are modified.
