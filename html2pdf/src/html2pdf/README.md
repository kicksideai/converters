# html2pdf

Render an HTML document into a PDF. The source is an HTML string or an `http(s)`
URL (downloaded automatically); output is the PDF as raw bytes.

## API

```lua
local html2pdf = require("html2pdf")

local pdf, err = html2pdf.render({ html = "<h1>Hello</h1>" })
if err ~= nil then
    return nil, err            -- err:kind() / err:message() / tostring(err)
end
```

### `html2pdf.render(request [, options]) -> pdf, err`

- **`request`** — a table with **exactly one** source:
  - `{ html = "<…>" }` — a UTF-8 HTML document string, or
  - `{ url = "https://…" }` — an `http(s)` URL, downloaded automatically.
- **`options`** — an optional table (unknown keys are rejected):
  - **`page_size`** — `"A4"` (default), `"Letter"`, `"Legal"`, `"A3"`, `"A5"`.
  - **`landscape`** — boolean, defaults to `false`.
  - **`margin_mm`** — page margin in millimetres, a non-negative number, defaults to `10`.
- **`pdf`** — the rendered document as PDF bytes (a binary Lua string), or `nil` on failure.
- **`err`** — `nil` on success, otherwise a structured [error](#errors).

### `html2pdf.new(runtime) -> module`

Advanced: build an instance with injected runtime dependencies
(`{ funcs?, http_client? }`). See [RFC 0001](../../docs/rfc/0001-wasm-seam-sdk.md).

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

- Inputs (the HTML string or downloaded body) are capped at 128 MiB.
- URL fetches use an http(s)-only scheme allow-list, a 30s timeout, and a download
  size cap; private-IP targets are blocked by the runtime.

## Rendering engine

HTML and CSS are rendered by an embedded
[litehtml](https://github.com/litehtml/litehtml) engine, with PDF output produced by
[libharu](https://github.com/libharu/libharu), both compiled into a single
`wasm32-wasip2` component. The engine supports real HTML and CSS including flexbox,
tables, backgrounds, borders, and list markers.

- **Fonts** — DejaVu LGC (Sans, Serif, Mono; regular/bold/italic) are embedded in the
  component, so `sans-serif`, `serif` and `monospace` render out of the box. Text is
  UTF-8, so Latin accents and common symbols (e.g. `á ç ã é € —`) render correctly.
- **Images** — `data:` URIs in **JPEG** are drawn; other formats (including PNG) and remote
  `http(s)` image references are not resolved.
- **Not supported** — JavaScript, CSS Grid, and full browser-grade CSS fidelity. This module
  targets document-grade HTML/CSS (reports, invoices, certificates), not arbitrary web pages.
- **Pagination** — a single page is produced per call; content taller than the page is clipped.
