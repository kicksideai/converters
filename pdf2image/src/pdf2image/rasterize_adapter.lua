local fs = require("fs")
local logger = require("logger")
local log = logger:named("pdf2image.rasterize")
local pdf2image = require("pdf2image")
local types = require("types")

-- PDF rasterizer adapter behind the convert.rasterizer contract. Input is a content
-- reference resolved to a pointer (storage volume + path, or url, or raw bytes) -- the
-- bytes are pulled here, not pushed through the call boundary. Output is per-page PNG
-- (or requested format) blobs. The pdf2image WASM pool bounds render concurrency.
--
-- run(req) where req = {
--   ref = { storage_id, storage_path } | { url } | { bytes },
--   dpi?, format?, pages? (1-based list or "all"), count_only?,
-- }
-- returns { ok = { total, pages = { { page, bytes, format } } } } or { err = { message, kind, retryable, code } }

local RETRYABLE_KINDS: {[string]: boolean} = {[errors.UNAVAILABLE] = true, [errors.TIMEOUT] = true}

local function fail(cause: error | string): types.RasterizeResult
    if type(cause) == "userdata" then
        local err = cause :: error
        local kind = err:kind()
        local retryable = err:retryable()
        if retryable == nil then retryable = RETRYABLE_KINDS[kind] == true end
        local code = (err:details() or {}).code
        if type(code) ~= "string" then code = kind end
        return {err = {message = err:message(), kind = kind, retryable = retryable, code = code}}
    end
    return {err = {message = cause :: string, kind = errors.INVALID, retryable = false, code = "invalid_input"}}
end

local function storage_failure(cause: error): error
    log:warn("page source could not be read", {error = tostring(cause)})
    return errors.new({message = "page source could not be read", kind = cause:kind(),
        retryable = cause:retryable(), details = cause:details()})
end

local function open_doc(ref: types.RasterizeRef): (types.PdfDocument?, error | string?)
    if type(ref) ~= "table" then return nil, "rasterize: ref must be a table" end

    if type(ref.url) == "string" and (ref.url :: string) ~= "" then
        local doc, err = pdf2image.open({ url = ref.url })
        if err ~= nil then return nil, err end
        return doc, nil
    end

    local bytes = ref.bytes
    if type(bytes) ~= "string" or bytes == "" then
        local storage_id = ref.storage_id
        local storage_path = ref.storage_path
        if type(storage_id) ~= "string" or storage_id == "" or type(storage_path) ~= "string" or storage_path == "" then
            return nil, "rasterize: ref needs storage_id/storage_path, url, or bytes"
        end
        local storage, storage_err = fs.get(storage_id :: string)
        if storage_err ~= nil then return nil, storage_failure(storage_err) end
        if not storage then
            return nil, errors.new({message = "rasterize: storage unavailable", kind = errors.INTERNAL,
                retryable = false, details = {code = "storage_unavailable"}})
        end
        local content, read_err = storage:readfile(storage_path :: string)
        if read_err ~= nil then return nil, storage_failure(read_err) end
        if type(content) ~= "string" or content == "" then return nil, "rasterize: failed to read source bytes" end
        bytes = content
    end

    local doc, err = pdf2image.open({ bytes = bytes })
    if err ~= nil then return nil, err end
    return doc, nil
end

local function run(req: types.RasterizeRequest): types.RasterizeResult
    req = type(req) == "table" and req or {}

    local doc, open_err = open_doc(req.ref)
    if open_err ~= nil or doc == nil then return fail(open_err or "rasterize: open failed") end

    local total = tonumber(doc.pages) or 0
    if req.count_only == true then
        return { ok = { total = total } }
    end
    if total < 1 then return { ok = { total = 0, pages = {} } } end

    local format = type(req.format) == "string" and (req.format :: string) ~= "" and req.format or "png"
    local render_opts: types.RasterizeOptions = { dpi = tonumber(req.dpi) or 150, format = format }
    if tonumber(req.quality) ~= nil then render_opts.quality = tonumber(req.quality) end

    local page_selector = req.pages
    if type(page_selector) == "table" then render_opts.pages = page_selector else render_opts.pages = "all" end

    local images, render_err = doc.render(render_opts)
    if render_err ~= nil then return fail(render_err) end
    if type(images) ~= "table" then return fail("rasterize: unexpected render result") end

    local pages: {types.RasterizePage} = {}
    for i = 1, #images do
        -- render returns images in selector order; map back to the page number.
        local page_no = (type(page_selector) == "table") and tonumber(page_selector[i]) or i
        pages[i] = { page = page_no, bytes = images[i], format = format }
    end

    return { ok = { total = total, pages = pages } }
end

return { run = run }
