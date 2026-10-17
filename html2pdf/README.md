# kickside/html2pdf

HTML strings or URLs to PDF bytes.

This module installs independently into a [KickSide](https://kickside.ai) host.

## Quick start

From a Wippy application directory:

```sh
wippy add kickside/html2pdf
wippy install
```

Declare the SDK import on the calling Lua entry. The
[API reference](src/html2pdf/README.md) documents request tables, return values,
limits, and error handling.

## Key concepts

The package contains a WebAssembly component and a Lua SDK. Calls return
`value, err`; check the error before consuming output.
Rendering has no JavaScript or CSS Grid support. Each call produces one page
and clips overflow.

## Requirements

Wippy 0.3.x; repository validation uses 0.3.47a. Use current Hub dependencies
for the contracts declared by the module.
The converter has no SQL persistence; SQLite and PostgreSQL are host choices.
To rebuild WASM, install Docker for the checked-in wasi-sdk toolchain. Make and Python 3 support build and staging.

## Development

From this module directory:

```sh
make lockrefresh
make lint
make test
```

`make build-component` rebuilds and stages the WASM asset and its registry hash.
Run the converter suite against hosts using each database engine when changing
host integration. The converter suite itself has no SQL dialect matrix.

## Lua API

Import `kickside.convert.html2pdf:html2pdf` and call `render({html = text})`
or `render({url = url})`. The result is a binary PDF string. The engine emits
one page and clips content beyond it; it does not execute JavaScript.

Check the returned `err` before using a result. The linked API reference owns
options, bounds, error classifications, and format limits.

## Reference

- [Module reference](src/html2pdf/README.md)

## Links

- [KickSide](https://kickside.ai)
- [Module authoring](https://github.com/kicksideai/docs/blob/main/building-modules/index.md)
- [Wippy documentation](https://wippy.ai)
- [Wippy Hub](https://hub.wippy.ai/kickside/html2pdf)

## License

[MIT License](LICENSE). Vendored dependencies retain their own licenses.
