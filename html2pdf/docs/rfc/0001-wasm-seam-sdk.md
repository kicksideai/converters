# RFC 0001 — WASM-Seam SDK Convention for Document Converters

- Status: Proposed
- Applies to: `pdf2image`, `html2pdf`, `doc2md` (and future WASM-backed Lua SDK modules)
- Reference implementation: `pdf2image`

## Summary

Every document-converter module is a thin Lua SDK over a WebAssembly component
reached through `funcs.call`. This RFC defines one shared convention for that
SDK layer: a single dependency-injection **seam** (`new(runtime?)`) that wraps the
host primitives (`funcs`, `base64`, `http_client`), plus the uniform request,
options, error, and hardening contract that sits on top of it. The seam makes the
WASM boundary explicit, swappable, and testable without weakening the public API.

## Motivation

The converters share the same anatomy and, before this work, the same gaps:

1. **An untestable failure surface.** Each module captures its host modules as
   upvalues (`local funcs = require("funcs")`). The test framework's `mock()`
   swaps `_G.<module>`, which an already-`require`d upvalue never sees. So the
   defensive branches at the WASM boundary — `funcs.call` returning `(nil, nil)`
   when a component yields a nil value, a component emitting invalid base64, a
   component returning a structured `{ err = ... }` — could not be exercised
   deterministically. They were therefore untested, and silent regressions there
   would crash callers (`attempt to index a nil value`).

2. **Three divergent shapes.** `pdf2image` returned a bare `open` function;
   `html2pdf` a bare `render`; `doc2md` a `{ convert = ... }` table. Two guessed
   URL-vs-blob with a `^https?://` heuristic.
   Only one carried type annotations. Errors were ad-hoc strings.

3. **An unhardened host boundary.** No fetch timeout, no max download/input size,
   no scheme allow-list, unchecked `base64.decode`, unvalidated numeric options.

The seam addresses (1) directly and gives (2) and (3) a single, consistent home.

## The seam

Each module is built by a factory that takes its host dependencies and returns the
public table. The module's top-level value is the default instance, wired to the
real runtime modules, with the factory exposed as `new` for tests and advanced use:

```lua
local function new(runtime: Runtime?): any
    local deps = runtime or {}
    local funcs = deps.funcs or require("funcs")
    local base64 = deps.base64 or require("base64")
    local http_client = deps.http_client or require("http_client")

    -- download / resolve_source / open|render|convert close over deps
    return { open = open }   -- or { render = ... } / { convert = ... }
end

local default = new()

return {
    open = default.open,     -- the production entry point(s)
    new = new,               -- the seam
}
```

`Runtime` is `{ funcs: any?, base64: any?, http_client: any? }`; any omitted
dependency falls back to `require`. (A module that needs no base64 — e.g.
`html2pdf`, `doc2md` — simply does not reference it.)

### Why a factory rather than `mock()`

`get_target_info` in the test framework resolves a mock path to `_G[name]` and
replaces that global with a new table. A converter that did
`local funcs = require("funcs")` holds the *original* reference, so the swap is
invisible to it. Injecting the dependency at construction is the only seam that
reliably reaches the code under test, and — unlike a test-only back door — it is
also a legitimate public feature (a caller may supply a custom `http_client`).

### What the seam enables in tests

```lua
local function fake(call_impl)
    return pdf2image.new({
        funcs = { call = call_impl },
        base64 = base64,
        http_client = { get = function() return nil, "unused" end },
    })
end

-- (nil, nil) from the component  -> "converter returned no result"
fake(function() return nil, nil end).open({ bytes = pdf })
-- a structured component error    -> passed through, kind = decode_failed
fake(function() return { err = { message = "corrupt pdf" } }, nil end).open(...)
-- invalid base64 in ok            -> "failed to decode render result"
```

## The WASM boundary contract

All components answer through one envelope, unwrapped identically everywhere:

1. `result, transport_err = funcs.call(TARGET, ...)`.
2. `transport_err ~= nil` → `internal` (retryable). Pass the message through.
3. `type(result) ~= "table"` → `internal`, `"converter returned no result"`.
   This is the live `(nil, nil)` case and **must** be guarded before indexing.
4. `result.err ~= nil` → a component-side failure. Read
   `(type(result.err) == "table" and result.err.message) or tostring(result.err)`
   and classify by the call site: a parse/sniff call → `decode_failed`; a
   render/produce call → `render_failed`.
5. otherwise `result.ok` is the success value; type-check it, and for binary
   payloads carried as base64, `base64.decode` and check both return values.

Binary inputs are marshaled to the component as base64 (`pdf2image`);
`doc2md` passes raw bytes; `html2pdf` passes the HTML string. The component
`funcs.call` target ids and argument shapes are unchanged by this RFC — only the
Lua surface around them is standardized.

> Components today emit only `err.message`. The Lua layer infers `kind` from which
> target it called. A future component change could add an explicit
> `err:details().code` to allow precise classification (e.g. distinguishing a corrupt
> input from an internal panic). This is optional and out of scope here.

## Public surface contract

- **Export shape.** A table of named verbs plus `new`. `pdf2image.open`,
  `html2pdf.render`, `doc2md.convert`.
- **Source request.** Exactly one of two keys. Binary modules: `{ bytes }` or
  `{ url }`. `html2pdf` (UTF-8 source): `{ html }` or `{ url }`. The `^https?://`
  test survives only as a *validation* assertion on `url`, never to route.
- **Options.** A trailing optional table, validated, with **unknown keys
  rejected** so typos fail loudly. Options are domain-specific and are NOT unified
  across modules (e.g. `pdf2image`'s `format`/`scale`/`background`/`pages`,
  `html2pdf`'s `page_size`/`landscape`/`margin_mm`, `doc2md`'s `extract_images`).
- **Returns.** `(value, nil)` on success, `(nil, Error)` on failure.

## Error contract

Every failure returns `nil, err` with a native runtime error. Read
`err:message()`, `err:kind()`, `err:retryable()` and `err:details().code`.
`tostring(err)` and string concatenation retain the diagnostic text.

| `details().code` | `kind()` | `retryable()` |
|---|---|---|
| `invalid_input`, `out_of_range`, `decode_failed` | `errors.INVALID` | `false` |
| `download_failed` | `errors.UNAVAILABLE` | `true` |
| `render_failed` | `errors.INTERNAL` | `false` |
| `internal` | `errors.INTERNAL` | `true` |

## Security and robustness hardening (at the seam)

- **Fetch timeout** via `http_client.get(url, { timeout = "30s" })`.
- **Max download / input size** of 32 MiB, enforced by `max_response_body` and a
  post-check, and again on raw `bytes`/`html` before marshaling (base64 inflates
  memory ~33% and is held alongside the blob).
- **Scheme allow-list** (`http`/`https` only) as a pre-flight string check, plus a
  re-check of the final `resp.url` to catch a redirect to a disallowed scheme.
  `http_client` exposes no scheme/redirect option, so this is the available lever.
- **SSRF to private IPs** is already enforced by the runtime (`checkPrivateIP`);
  the wrapper surfaces that denial as `download_failed` and does not reimplement it.
- **Numeric validation.** Page indices must be integers in range (`out_of_range`);
  `dpi`/`scale` are bounded (`dpi <= 600`, `scale <= 8`) to prevent a memory-DoS
  render; `quality` is `1..100`.
- **Decode/encode checks.** Every `base64.encode`/`decode` result is checked.

## Module template / checklist

A conforming module:

1. Declares the source request and options/result types; failures use native `error`.
2. Constructs native `errors.new` values with kind, retryability and a stable
   `details.code` at each failure boundary.
3. Validates the request (exactly-one-source, non-empty, URL scheme) and size.
4. Hardens `download` (timeout, max bytes, scheme, redirect re-check).
5. Rejects unknown option keys and validates each known key.
6. Unwraps every `funcs.call` through the 5-step boundary contract above.
7. Exposes `{ <verbs>, new }`, with `new(runtime?)` as the only injection seam.
8. Ships tests that drive the boundary branches through `new` with a fake `funcs`.

## Adoption

| Module | Entry | Source keys | Notable options |
|---|---|---|---|
| `pdf2image` | `open` (+ handle `to_image`/`render`) | `bytes` \| `url` | `dpi`/`scale`/`format`/`quality`/`background`/`pages` |
| `html2pdf` | `render` | `html` \| `url` | `page_size`/`landscape`/`margin_mm` |
| `doc2md` | `convert` | `bytes` \| `url` | `extract_images` |

All four implement the seam and the contract above. The `new`-based tests cover
the `(nil, nil)` result, transport failure, component error passthrough, invalid
base64, and degenerate output for each module.

## Drawbacks and alternatives

- **Breaking change.** Bare-function exports and string errors are gone; callers
  must adopt `.open(...)` / `.render(...)` / `.convert(...)` and `err:details().code`.
  Mitigated by `__tostring`/`__concat` and module version bumps.
- **Replication over a shared library.** The helpers are duplicated per module to
  keep each self-contained and independently publishable; a shared module was
  rejected to avoid a cross-module dependency. A consistency check (diffing the
  helper blocks) compensates.
- **Kind inference by call site.** Without a component-side `kind`, classification
  is coarse. Acceptable today; revisitable via the optional component change noted
  above.
