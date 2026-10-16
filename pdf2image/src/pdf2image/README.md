# pdf2image

Render the pages of a PDF document to images. The source is supplied as raw bytes
or an `http(s)` URL; output is one image per page, produced on demand, in PNG,
JPEG, or WEBP — with a configurable scale or DPI and background.

## API

```lua
local pdf2image = require("pdf2image")

local doc, err = pdf2image.open({ bytes = pdf_bytes })
if err ~= nil then
    return nil, err            -- err:kind() / err:message() / tostring(err)
end
```

### `pdf2image.open(request) -> handle, err`

Opens a PDF and returns a handle for querying and rendering it.

- **`request`** — a table with **exactly one** source:
  - `{ bytes = <binary string> }` — the raw bytes of a PDF, or
  - `{ url = "https://…" }` — an `http(s)` URL, downloaded automatically.
- **`handle`** — on success, the table described below; `nil` on failure.
- **`err`** — `nil` on success, otherwise a structured [error](#errors).

### `handle.pages -> number`

The number of pages in the document.

### `handle.to_image(page [, opts]) -> image, err`

Renders a single page to one image.

- **`page`** — the 1-based page number, an integer from `1` to `handle.pages`.
- **`opts`** — optional. Either a **number** (treated as `dpi`, for backwards
  compatibility) or a **render options table** (see [Render options](#render-options)).
- **`image`** — the rendered page as bytes (a binary Lua string), or `nil` on failure.

```lua
local png = doc.to_image(1)                         -- PNG at the default 150 DPI
local png = doc.to_image(1, { dpi = 300 })                    -- PNG at 300 DPI
local jpg = doc.to_image(1, { format = "jpeg", quality = 85, scale = 2 })
```

### `handle.render(opts) -> images, err`

Renders several pages in a single call (the document is parsed once). Accepts the
same render options plus a `pages` selector.

- **`opts.pages`** — which pages to render:
  - omitted or `"all"` — every page;
  - `{ from = a, to = b }` — the inclusive 1-based range `a..b`;
  - `{ 1, 3, 5 }` — an explicit list of 1-based page numbers, rendered in order.
- **`images`** — an array of image byte strings, in the requested order; `nil` on failure.

```lua
local images = doc.render({ pages = "all", scale = 2 })
local images = doc.render({ pages = { from = 2, to = 4 }, format = "webp" })
```

### `pdf2image.new(runtime) -> module`

Advanced: build an instance with injected runtime dependencies
(`{ funcs?, http_client? }`); each omitted dependency falls back to the
real runtime module. Used for tests and for supplying a custom HTTP client. See
[RFC 0001](../../docs/rfc/0001-wasm-seam-sdk.md).

## Render options

Options are grouped semantically and validated at the boundary; unknown keys are
rejected. All are optional.

### Scaling

- **`dpi`** — output resolution in dots per inch; defaults to `150`. Must be a
  positive number no greater than `600`.
- **`scale`** — a multiplier relative to the native page size, where `1` is 72 DPI
  (`scale = 2` matches 144 DPI). Must be positive and no greater than `8`.
- `dpi` and `scale` are mutually exclusive; setting both is an error.

### Output

- **`format`** — `"png"` (default), `"jpeg"` (alias `"jpg"`), or `"webp"`. Case-insensitive.
- **`quality`** — `1`–`100`, applies to **JPEG only**; defaults to `90`.
- **`background`** — the color drawn behind the page:
  - omitted or `""` — white;
  - `"transparent"` — fully transparent (PNG and WEBP only; an error for JPEG);
  - `"#RRGGBB"` or `"#RRGGBBAA"` — an explicit hex color (the leading `#` is optional).

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

## Limits and safety

- Inputs (bytes or downloaded body) are capped at 128 MiB.
- URL fetches use an http(s)-only scheme allow-list, a 30s timeout, and a download
  size cap; private-IP targets are blocked by the runtime.

## Input / output formats

- A single **PDF**, supplied as raw bytes or an `http(s)` URL. Classic xref tables
  and xref/object streams (PDF 1.5+), embedded and base fonts, vector graphics,
  and embedded raster images are handled.
- Output is **PNG** (8-bit RGBA) by default, or **JPEG** / **WEBP** when requested,
  returned as raw bytes — nothing is base64-encoded at the API level.

## Build

`make build-component` builds the component and stages it; `make test` runs the suite.
