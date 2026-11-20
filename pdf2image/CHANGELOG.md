# Changelog

## [0.7.20](https://github.com/kicksideai/converters/compare/pdf2image-v0.7.19...pdf2image-v0.7.20) (2026-11-20)


### Bug Fixes

* **pdf2image:** keep rendered JPEGs within the browser dimension limit ([#51](https://github.com/kicksideai/converters/issues/51)) ([0e2a184](https://github.com/kicksideai/converters/commit/0e2a18400f93b2dfc3376357c93a8d89bdc710ea))

## [0.7.19](https://github.com/kicksideai/converters/compare/pdf2image-v0.7.18...pdf2image-v0.7.19) (2026-11-16)


### Bug Fixes

* declare WASM limits at options.limits ([#46](https://github.com/kicksideai/converters/issues/46)) ([c4dc65b](https://github.com/kicksideai/converters/commit/c4dc65b1c1d1e8821716efbfd47a21f3144a1d70))

## [0.7.18](https://github.com/kicksideai/converters/compare/pdf2image-v0.7.17...pdf2image-v0.7.18) (2026-11-16)


### Performance Improvements

* build the converters with WebAssembly SIMD ([38313e0](https://github.com/kicksideai/converters/commit/38313e009d1734ef6ab4ecbdfcf7dfead57d0130))

## [0.7.17](https://github.com/kicksideai/converters/compare/pdf2image-v0.7.16...pdf2image-v0.7.17) (2026-11-15)


### Bug Fixes

* **pdf2image:** render oversized pages within renderer limits ([#39](https://github.com/kicksideai/converters/issues/39)) ([3316d00](https://github.com/kicksideai/converters/commit/3316d00d8ada5e3cb34953954f43d6791e79bca5))


### Performance Improvements

* accelerate document conversion on released runtimes ([6ba1dcb](https://github.com/kicksideai/converters/commit/6ba1dcbf72ecc70865941ae7200c41ddbd530dff))

## [0.7.16](https://github.com/kicksideai/converters/compare/pdf2image-v0.7.15...pdf2image-v0.7.16) (2026-11-12)


### Bug Fixes

* **pdf2image:** preserve pixels under nonpainting image colorants ([#25](https://github.com/kicksideai/converters/issues/25)) ([b7098c6](https://github.com/kicksideai/converters/commit/b7098c688be8c7a4c7f86d49a6e48b414c3493f2))

## [0.7.15](https://github.com/kicksideai/converters/compare/pdf2image-v0.7.14...pdf2image-v0.7.15) (2026-11-11)


### Bug Fixes

* **pdf2image:** preserve rasterizer error classification ([#23](https://github.com/kicksideai/converters/issues/23)) ([7ebc634](https://github.com/kicksideai/converters/commit/7ebc63498d34dbec572c992a3b6ef3d87a01cea6))

## [0.7.14](https://github.com/kicksideai/converters/compare/pdf2image-v0.7.13...pdf2image-v0.7.14) (2026-11-04)


### Bug Fixes

* **doc2md:** build the shipped component independently of installed rust-src ([57d2f84](https://github.com/kicksideai/converters/commit/57d2f8448bbf7778ebb5de54b29d6f8e024df601))

## 0.7.13 (2026-10-17)

- Build the component with portable compiler source paths and refresh packaged documentation.

## 0.7.10 (2026-10-08)

- Add selectable module access policies and native scope tests (fcef4be)
- fix(release): retain the published PDF2Image snapshot baseline (ede0d66)
- fix(convert): preserve string error messages across component calls (f1aeea2)

## 0.7.9

Current published Hub version.

## 0.7.8

Current published Hub version.
