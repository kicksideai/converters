# Contributing to KickSide Converters

Follow the [organization contribution guide](https://github.com/kicksideai/.github/blob/main/CONTRIBUTING.md),
[Code of Conduct](https://github.com/kicksideai/.github/blob/main/CODE_OF_CONDUCT.md), and
[CLA](https://github.com/kicksideai/.github/blob/main/CLA.md). Send the signed CLA privately to
[licensing@wippy.ai](mailto:licensing@wippy.ai) before merge.

## Development and testing

Install Make, Python 3, `shasum`, and the Wippy CLI version used by
[CI](.github/workflows/ci.yml). Rust modules use their checked-in toolchains and
`wasm32-wasip2`; `html2pdf` uses Docker and `wasi-sdk`. There is no UI build.
From the repository root, prepare the module you are changing:

```sh
cd doc2md
ln -sf "$(command -v wippy)" wippy
make install
make check
```

Use the corresponding directory for `html2pdf` or `pdf2image`.
`make check` rebuilds the component, lints, and runs the module's suite. Commit
regenerated tracked WASM assets with their registry hashes. Preserve
[third-party notices](THIRD_PARTY_NOTICES.md) and the vendored license files.

## Pull requests

Create a focused branch and submit a GitHub pull request to this repository.
Use a commit subject such as
`docs: clarify setup` or `fix: handle an empty result`. Explain the behavior and
validation in the PR; include relevant commands and outcomes. Maintainers review
and merge GitHub pull requests after the required checks and CLA confirmation.
