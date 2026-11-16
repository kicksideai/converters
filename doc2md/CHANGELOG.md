# Changelog

## [0.8.28](https://github.com/kicksideai/converters/compare/doc2md-v0.8.27...doc2md-v0.8.28) (2026-11-16)


### Bug Fixes

* declare WASM limits at options.limits ([#46](https://github.com/kicksideai/converters/issues/46)) ([c4dc65b](https://github.com/kicksideai/converters/commit/c4dc65b1c1d1e8821716efbfd47a21f3144a1d70))

## [0.8.27](https://github.com/kicksideai/converters/compare/doc2md-v0.8.26...doc2md-v0.8.27) (2026-11-16)


### Performance Improvements

* build the converters with WebAssembly SIMD ([38313e0](https://github.com/kicksideai/converters/commit/38313e009d1734ef6ab4ecbdfcf7dfead57d0130))

## [0.8.26](https://github.com/kicksideai/converters/compare/doc2md-v0.8.25...doc2md-v0.8.26) (2026-11-16)


### Bug Fixes

* **doc2md:** count page images without decoding them in page_resources ([#37](https://github.com/kicksideai/converters/issues/37)) ([9a41697](https://github.com/kicksideai/converters/commit/9a41697356bee7a640b43c09184775306e47e9c3))

## [0.8.25](https://github.com/kicksideai/converters/compare/doc2md-v0.8.24...doc2md-v0.8.25) (2026-11-15)


### Bug Fixes

* **doc2md:** share cached font decode maps instead of copying them per text run ([#36](https://github.com/kicksideai/converters/issues/36)) ([b07d0cf](https://github.com/kicksideai/converters/commit/b07d0cf9a10e592404d081ed89b32c420cb62316))


### Performance Improvements

* accelerate document conversion on released runtimes ([6ba1dcb](https://github.com/kicksideai/converters/commit/6ba1dcbf72ecc70865941ae7200c41ddbd530dff))

## [0.8.24](https://github.com/kicksideai/converters/compare/doc2md-v0.8.23...doc2md-v0.8.24) (2026-11-14)


### Bug Fixes

* **doc2md:** build the page index once per document ([#34](https://github.com/kicksideai/converters/issues/34)) ([b5c9966](https://github.com/kicksideai/converters/commit/b5c9966c4e18cccb705cf704b36dc3fe2455a134))

## [0.8.23](https://github.com/kicksideai/converters/compare/doc2md-v0.8.22...doc2md-v0.8.23) (2026-11-14)


### Bug Fixes

* **doc2md:** decode only the XObjects a page paints ([#31](https://github.com/kicksideai/converters/issues/31)) ([5d6d787](https://github.com/kicksideai/converters/commit/5d6d787a7ad61228b73faeeebcc7a5e0a82e3a16))
* **doc2md:** preserve painted image identity and composition ([#33](https://github.com/kicksideai/converters/issues/33)) ([1a8f724](https://github.com/kicksideai/converters/commit/1a8f724827f70659f925f09310bcf03eaf4c324a))

## [0.8.22](https://github.com/kicksideai/converters/compare/doc2md-v0.8.21...doc2md-v0.8.22) (2026-11-12)


### Bug Fixes

* **doc2md:** strip markers without lazy pattern quantifiers ([#29](https://github.com/kicksideai/converters/issues/29)) ([944a271](https://github.com/kicksideai/converters/commit/944a271bfad30f9508d53dcee10f1f42925c4c6f))

## [0.8.21](https://github.com/kicksideai/converters/compare/doc2md-v0.8.20...doc2md-v0.8.21) (2026-11-12)


### Bug Fixes

* **doc2md:** omit images with only None DeviceN colorants ([#26](https://github.com/kicksideai/converters/issues/26)) ([1497ff2](https://github.com/kicksideai/converters/commit/1497ff265d148b25bcd6d25b20ca3413f959a33e))

## [0.8.20](https://github.com/kicksideai/converters/compare/doc2md-v0.8.19...doc2md-v0.8.20) (2026-11-11)


### Bug Fixes

* **doc2md:** omit NUL from PDF text ([#21](https://github.com/kicksideai/converters/issues/21)) ([94c3b84](https://github.com/kicksideai/converters/commit/94c3b84746ff0c8a950d353955505ded7af8b16c))

## [0.8.19](https://github.com/kicksideai/converters/compare/doc2md-v0.8.18...doc2md-v0.8.19) (2026-11-10)


### Bug Fixes

* **doc2md:** encode PDF image samples in every color space as PNG ([#16](https://github.com/kicksideai/converters/issues/16)) ([f97678b](https://github.com/kicksideai/converters/commit/f97678be3ff19ae5eec748846ef056bce0c93b3d))
* **doc2md:** keep page markers outside page cleanup ([#19](https://github.com/kicksideai/converters/issues/19)) ([e5a20ca](https://github.com/kicksideai/converters/commit/e5a20ca34e13bedcbda935b8ed8a7a586d095ed5))
* **doc2md:** speed up scanned PDFs and respect caller budgets ([#14](https://github.com/kicksideai/converters/issues/14)) ([0956091](https://github.com/kicksideai/converters/commit/0956091f0cc39795f70e8555526aef42652d88da))

## [0.8.18](https://github.com/kicksideai/converters/compare/doc2md-v0.8.17...doc2md-v0.8.18) (2026-11-10)


### Bug Fixes

* **doc2md:** preserve PDF image page identity ([#13](https://github.com/kicksideai/converters/issues/13)) ([1f3a0ff](https://github.com/kicksideai/converters/commit/1f3a0ff7f5a401d5f251823bfb6c3302fbf24796))

## [0.8.17](https://github.com/kicksideai/converters/compare/doc2md-v0.8.16...doc2md-v0.8.17) (2026-11-04)


### Bug Fixes

* **doc2md:** build the shipped component independently of installed rust-src ([57d2f84](https://github.com/kicksideai/converters/commit/57d2f8448bbf7778ebb5de54b29d6f8e024df601))

## [0.8.16](https://github.com/kicksideai/converters/compare/doc2md-v0.8.15...doc2md-v0.8.16) (2026-11-01)


### Bug Fixes

* **doc2md:** remove compiler sysroot paths from shipped component ([08cd224](https://github.com/kicksideai/converters/commit/08cd224120b3c54e523f7767f85ccc345786f818))

## [0.8.15](https://github.com/kicksideai/converters/compare/doc2md-v0.8.14...doc2md-v0.8.15) (2026-10-27)


### Features

* **doc2md:** optional hidden page and slide markers ([d0d1d18](https://github.com/kicksideai/converters/commit/d0d1d189377c10be5fea461b2dee907588e12015))
* **doc2md:** optional hidden page and slide markers ([2c2beea](https://github.com/kicksideai/converters/commit/2c2beea33425de7bc22c5df808fbe0887aac5ae8))

## 0.8.14 (2026-10-17)

- Build the component with portable compiler source paths and refresh packaged documentation.

## 0.8.11 (2026-10-08)

- Add selectable module access policies and native scope tests (fcef4be)
- test: refresh converter harness dependencies (7aab14b)
- fix(convert): preserve string error messages across component calls (f1aeea2)

## 0.8.10

Current published Hub version.
