# kickside/pdf2image

PDF bytes or URLs to selected pages as PNG, JPEG, or WEBP.

This module installs independently into a [KickSide](https://kickside.ai) host.

## Quick start

From a Wippy application directory:

```sh
wippy add kickside/pdf2image
wippy install
```

Declare the SDK import on the calling Lua entry. The
[API reference](src/pdf2image/README.md) documents request tables, return values,
limits, and error handling.

## Key concepts

The package contains a WebAssembly component and a Lua SDK. Calls return
`value, err`; check the error before consuming output.

## Requirements

Wippy 0.3.x; repository validation uses 0.3.47a. Use current Hub dependencies
for the contracts declared by the module.
The converter has no SQL persistence; SQLite and PostgreSQL are host choices.
To rebuild WASM, install Rust with `wasm32-wasip2`. Make and Python 3 support build and staging.

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

Import `kickside.convert.pdf2image:pdf2image` and call `open({bytes = data})`
or `open({url = url})`. The handle exposes `pages`, `to_image(page, options)`,
and `render(options)`. Images are binary strings in the selected output format.

Check the returned `err` before using a result. The linked API reference owns
options, bounds, error classifications, and format limits.

## Reference

- [Module reference](src/pdf2image/README.md)

## Links

- [KickSide](https://kickside.ai)
- [Module authoring](https://kicksideai.github.io/docs/building-modules/)
- [Wippy documentation](https://wippy.ai)
- [Wippy Hub](https://hub.wippy.ai/kickside/pdf2image)

## License

[MIT License](LICENSE). Vendored dependencies retain their own licenses.
