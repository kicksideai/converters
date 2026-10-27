# Doc2MD

Doc2MD is a Wippy library module that converts documents to Markdown through a
Rust WebAssembly component.

It supports PDF, modern Office files (`.docx`, `.xlsx`, `.pptx`), legacy Office
files (`.doc`, `.xls`, `.ppt`), and RTF. Image extraction is opt-in, so the
default result is text-focused Markdown close to classic document-to-markdown
converters.

## Public API

Import the Lua SDK from the public module entry:

```yaml
imports:
  doc2md: kickside.convert.doc2md:doc2md
```

Convert raw bytes:

```lua
local doc2md = require("doc2md")

local document, err = doc2md.convert({
    bytes = uploaded_bytes,
})
if err ~= nil then
    return nil, err
end

return document.markdown
```

Convert a URL:

```lua
local document, err = doc2md.convert({
    url = "https://example.com/report.pdf",
})
```

Enable image extraction explicitly (a second `options` argument):

```lua
local document, err = doc2md.convert({ bytes = uploaded_bytes }, {
    extract_images = true,
})
```

## Request and options

`doc2md.convert(request [, options])`:

- **`request`** — a table with **exactly one** source:
  - `bytes`: non-empty Lua binary string with the document content, or
  - `url`: HTTP(S) URL to download.
- **`options`** — an optional table (unknown keys are rejected):
  - `extract_images`: optional boolean, default `false`.
  - `page_markers`: optional boolean, default `false`. Inserts a hidden HTML
    comment on its own line where each PDF page (`<!-- page 3 -->`) and PPTX
    slide (`<!-- slide 3: Pricing -->`) starts. Markdown viewers do not show
    comments. Other formats carry no page boundaries and are unchanged.

`doc2md.new(runtime)` builds an instance with injected runtime dependencies
(`{ funcs?, http_client? }`) for tests or a custom HTTP client. See
[RFC 0001](../../docs/rfc/0001-wasm-seam-sdk.md).

## Errors

Every failure returns `nil, err` with a native runtime error. Read
`err:message()`, `err:kind()`, `err:retryable()` and `err:details().code`.
`tostring(err)` and string concatenation retain the diagnostic text.

| `details().code` | `kind()` | `retryable()` |
|---|---|---|
| `invalid_input`, `out_of_range`, `decode_failed` | `errors.INVALID` | `false` |
| `download_failed` | `errors.UNAVAILABLE` | `true` |
| `render_failed` | `errors.INTERNAL` | `false` |
| `internal` | `errors.INTERNAL` | `true` |

## Page and slide markers

`doc2md.markers(markdown)` returns the markers in document order as
`{ kind = "page" | "slide", number, name?, start, finish }`, where `start` and
`finish` are byte offsets of the comment. Text belongs to the last marker that
precedes it. `doc2md.strip_markers(text)` removes marker lines before text is
embedded or displayed.

## Page resources

`doc2md.page_resources(request)` accepts the same `bytes` or `url` request and
returns `[{ page, text_len, image_count }, ...]`, with one-based page
numbers. It helps callers select scanned pages for OCR.

## Response

Successful conversions return:

```lua
{
    markdown = "# Converted markdown",
    images = {
        {
            name = "image1.png",
            data = raw_image_bytes,
        },
    },
}
```

`images` is always a table. It is empty when `extract_images` is false or when
the document has no extractable images. Image data is returned as raw binary Lua
strings, not base64.

## Module Surface

The published `kickside.convert.doc2md` namespace includes:

- `kickside.convert.doc2md:doc2md`: Lua SDK.
- `kickside.convert.doc2md:converter`: WebAssembly converter function.
- `kickside.convert.doc2md:page_resources`: per-page text and image-resource scan.
- `kickside.convert.doc2md:assets`: embedded filesystem for `doc2md.wasm`.

Tests, fixtures, and load harnesses live under `app.env:**`. That namespace is
excluded by `wippy.yaml`, so sample documents are not part of the published
package.

## Development

Run from the module root, with the installed CLI linked as in CI:

```bash
ln -sf "$(command -v wippy)" wippy
make lockrefresh
make build-component
make lint
make test
make load-test
```

`make build-component` requires the Rust toolchain with the `wasm32-wasip2`
target. `make load-test` runs excluded local validation entries and does not
change the publish surface.

## Publish

Validate the exact publish path without uploading:

```bash
make publish-dry-run
```

The dry-run target validates `--module-type library`. Reviewed releases use
the repository’s GitHub Release Please workflow; direct `make publish` is
disabled.

## License

[MIT License](../../LICENSE).
