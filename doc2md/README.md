# kickside/doc2md

PDF, Office, legacy Office, and RTF documents to Markdown with optional image extraction.

This module installs independently into a [KickSide](https://kickside.ai) host.

## Quick start

From a Wippy application directory:

```sh
wippy add kickside/doc2md
wippy install
```

Declare the SDK import on the calling Lua entry. The
[API reference](src/doc2md/README.md) documents request tables, return values,
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

Import `kickside.convert.doc2md:doc2md` and call `convert({bytes = data})` or
`convert({url = url})`. The result contains `markdown` and an `images` array;
image extraction is opt-in through the second options argument.

Check the returned `err` before using a result. The linked API reference owns
options, bounds, error classifications, and format limits.

## Reference

- [Module reference](src/doc2md/README.md)

## Links

- [KickSide](https://kickside.ai)
- [Module authoring](https://kicksideai.github.io/docs/building-modules/)
- [Wippy documentation](https://wippy.ai)
- [Wippy Hub](https://hub.wippy.ai/kickside/doc2md)

## License

[MIT License](LICENSE). Vendored dependencies retain their own licenses.
