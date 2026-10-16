type ConvertRequest = {
    bytes: string?,
    url: string?,
}

type ConvertOptions = {
    extract_images: boolean?,
}

type Image = {
    name: string,
    data: string,
}

type Document = {
    markdown: string,
    images: { Image },
}

type Runtime = {
    funcs: any?,
    http_client: any?,
}

local PREFIX = "kickside.convert.doc2md: "
local CONVERTER = "kickside.convert.doc2md:converter"
local PAGE_RESOURCES = "kickside.convert.doc2md:page_resources"

local MAX_INPUT_BYTES = 128 * 1024 * 1024
local DOWNLOAD_TIMEOUT = "30s"
local ALLOWED_SCHEME = "^https?://"

local OPTION_KEYS = {
    extract_images = true,
}

local function reject_unknown(options: { [string]: any }, allowed: { [string]: boolean }): string?
    for key in pairs(options) do
        if allowed[key] ~= true then
            return "unknown option: " .. tostring(key)
        end
    end

    return nil
end

local function normalize_options(options: ConvertOptions?): (boolean?, string?)
    if options == nil then
        return false, nil
    end

    if type(options) ~= "table" then
        return nil, "options must be a table when provided"
    end

    local unknown = reject_unknown(options, OPTION_KEYS)
    if unknown ~= nil then
        return nil, unknown
    end

    if options.extract_images ~= nil and type(options.extract_images) ~= "boolean" then
        return nil, "extract_images must be a boolean when provided"
    end

    return options.extract_images == true, nil
end

local function normalize_images(images: any): (any, error?)
    if images == nil then
        return {}, nil
    end

    if type(images) ~= "table" then
        return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "converter returned invalid images: expected table, got " .. type(images), details = {code = "internal"}}) :: error)
    end

    local out = {}
    for index, image in ipairs(images) do
        if type(image) ~= "table" then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "converter returned invalid image: expected table, got " .. type(image), details = {code = "internal"}}) :: error)
        end
        if type(image.name) ~= "string" then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "converter returned invalid image name: expected string, got " .. type(image.name), details = {code = "internal"}}) :: error)
        end
        if type(image.data) ~= "string" then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "converter returned invalid image data: expected binary string, got " .. type(image.data), details = {code = "internal"}}) :: error)
        end
        out[index] = {
            name = image.name,
            data = image.data,
        }
    end

    return out, nil
end

local function new(runtime: Runtime?): any
    local deps = runtime or {}
    local funcs = deps.funcs or require("funcs")
    local http_client = deps.http_client or require("http_client")

    local function download(url: string): (string?, string?)
        local resp, err = http_client.get(url, {
            timeout = DOWNLOAD_TIMEOUT,
            max_response_body = MAX_INPUT_BYTES,
        })
        if err ~= nil then
            return nil, "download failed: " .. tostring(err)
        end

        if resp == nil then
            return nil, "download failed: empty response"
        end

        local status = resp.status_code or 0
        if status < 200 or status >= 300 then
            return nil, "download failed: HTTP " .. tostring(status)
        end

        local final_url = resp.url
        if type(final_url) == "string" and not final_url:match(ALLOWED_SCHEME) then
            return nil, "download failed: redirected to a disallowed scheme"
        end

        local body = resp.body
        if type(body) ~= "string" or #body == 0 then
            return nil, "download failed: empty body"
        end

        if #body > MAX_INPUT_BYTES then
            return nil, "download failed: response exceeds " .. tostring(MAX_INPUT_BYTES) .. " bytes"
        end

        return body, nil
    end

    local function resolve_source(request: ConvertRequest): (string?, error?)
        if type(request) ~= "table" then
            return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "request must be a table", details = {code = "invalid_input"}}) :: error)
        end

        local has_bytes = request.bytes ~= nil
        local has_url = request.url ~= nil
        if has_bytes == has_url then
            return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "request must set exactly one source: bytes or url", details = {code = "invalid_input"}}) :: error)
        end

        local blob: string
        if has_url then
            if type(request.url) ~= "string" or #request.url == 0 then
                return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "url must be a non-empty string", details = {code = "invalid_input"}}) :: error)
            end
            if not request.url:match(ALLOWED_SCHEME) then
                return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "url must be an http(s) URL", details = {code = "invalid_input"}}) :: error)
            end

            local body, derr = download(request.url)
            if derr ~= nil then
                return nil, (errors.new({kind = errors.UNAVAILABLE, retryable = true,
                message = PREFIX .. derr, details = {code = "download_failed"}}) :: error)
            end
            blob = body :: string
        else
            if type(request.bytes) ~= "string" or #request.bytes == 0 then
                return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "bytes must be a non-empty binary string", details = {code = "invalid_input"}}) :: error)
            end
            blob = request.bytes :: string
        end

        if #blob > MAX_INPUT_BYTES then
            return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "input exceeds " .. tostring(MAX_INPUT_BYTES) .. " bytes", details = {code = "invalid_input"}}) :: error)
        end

        return blob, nil
    end

    local function convert(request: ConvertRequest, options: ConvertOptions?): (any, error?)
        local blob, src_err = resolve_source(request)
        if src_err ~= nil then
            return nil, src_err
        end

        local extract_images, opt_err = normalize_options(options)
        if opt_err ~= nil then
            return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. opt_err, details = {code = "invalid_input"}}) :: error)
        end

        local wasm_request = {
            bytes = blob,
            options = {
                ["extract-images"] = extract_images == true,
            },
        }

        local result, call_err = funcs.call(CONVERTER, wasm_request)
        if call_err ~= nil then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = tostring(call_err), details = {code = "internal"}}) :: error)
        end

        if type(result) ~= "table" then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "converter returned no result", details = {code = "internal"}}) :: error)
        end

        if result.err ~= nil then
            local message = (type(result.err) == "table" and tostring(result.err.message)) or tostring(result.err)
            return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = message, details = {code = "decode_failed"}}) :: error)
        end

        local document = result.ok
        if type(document) ~= "table" then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "converter returned invalid document: expected table, got " .. type(document), details = {code = "internal"}}) :: error)
        end
        if type(document.markdown) ~= "string" then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "converter returned invalid markdown: expected string, got " .. type(document.markdown), details = {code = "internal"}}) :: error)
        end

        local images, images_err = normalize_images(document.images)
        if images_err ~= nil then
            return nil, images_err
        end

        return {
            markdown = document.markdown,
            images = images,
        }, nil
    end

    -- page_resources returns one entry per page: { page, text_len, image_count }.
    -- A page with little text and one or more images is a scanned page, so callers
    -- can render and OCR only those pages instead of the whole document.
    local function page_resources(request: ConvertRequest): (any, error?)
        local blob, src_err = resolve_source(request)
        if src_err ~= nil then
            return nil, src_err
        end

        local result, call_err = funcs.call(PAGE_RESOURCES, blob)
        if call_err ~= nil then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = tostring(call_err), details = {code = "internal"}}) :: error)
        end
        if type(result) ~= "table" then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "page_resources returned no result", details = {code = "internal"}}) :: error)
        end
        if result.err ~= nil then
            local message = (type(result.err) == "table" and tostring(result.err.message)) or tostring(result.err)
            return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = message, details = {code = "decode_failed"}}) :: error)
        end

        local raw = result.ok
        if type(raw) ~= "table" then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "page_resources returned invalid pages: expected table, got " .. type(raw), details = {code = "internal"}}) :: error)
        end

        local pages = {}
        for _, p in ipairs(raw :: { any }) do
            pages[#pages + 1] = {
                page = p.page,
                text_len = p["text-len"],
                image_count = p["image-count"],
            }
        end
        return pages, nil
    end

    return { convert = convert, page_resources = page_resources }
end

local default = new()

return {
    convert = default.convert,
    page_resources = default.page_resources,
    new = new,
    limits = { max_input_bytes = MAX_INPUT_BYTES },
}
