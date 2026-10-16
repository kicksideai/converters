type SourceRequest = {
    bytes: string?,
    url: string?,
}

type RenderOpts = {
    dpi: number?,
    scale: number?,
    format: string?,
    quality: number?,
    background: string?,
}

type BatchRenderOpts = {
    dpi: number?,
    scale: number?,
    format: string?,
    quality: number?,
    background: string?,
    pages: any?,
}

type Runtime = {
    funcs: any?,
    http_client: any?,
}

local PREFIX = "kickside.convert.pdf2image: "
local PAGES = "kickside.convert.pdf2image:pages"
local RENDER = "kickside.convert.pdf2image:render"

local DEFAULT_DPI = 150
local MAX_DPI = 600
local MAX_SCALE = 8
local MAX_INPUT_BYTES = 128 * 1024 * 1024
local DOWNLOAD_TIMEOUT = "30s"
local ALLOWED_SCHEME = "^https?://"

local VALID_FORMATS = {
    png = true,
    jpeg = true,
    jpg = true,
    webp = true,
}

local TO_IMAGE_OPTION_KEYS = {
    dpi = true,
    scale = true,
    format = true,
    quality = true,
    background = true,
}

local RENDER_OPTION_KEYS = {
    dpi = true,
    scale = true,
    format = true,
    quality = true,
    background = true,
    pages = true,
}

local function reject_unknown(options: { [string]: any }, allowed: { [string]: boolean }): string?
    for key in pairs(options) do
        if allowed[key] ~= true then
            return "unknown option: " .. tostring(key)
        end
    end

    return nil
end

local function valid_background(value: string): boolean
    local lower = value:lower()
    if lower == "" or lower == "transparent" then
        return true
    end

    local hex = value:match("^#?(%x+)$")
    return hex ~= nil and (#hex == 6 or #hex == 8)
end

local function background_alpha(value: string): number
    if value:lower() == "transparent" then
        return 0
    end

    local hex = value:match("^#?(%x+)$")
    if hex ~= nil and #hex == 8 then
        return tonumber(hex:sub(7, 8), 16) or 255
    end

    return 255
end

local function default_options(): any
    return {
        scaling = { dpi = 0, scale = 0 },
        output = { format = "", quality = 0, background = "" },
    }
end

local function validate_dpi(value: any): (number?, string?)
    if type(value) ~= "number" or value <= 0 or value > MAX_DPI then
        return nil, "dpi must be a positive number no greater than " .. tostring(MAX_DPI)
    end

    return value, nil
end

local function validate_scale(value: any): (number?, string?)
    if type(value) ~= "number" or value <= 0 or value > MAX_SCALE then
        return nil, "scale must be a positive number no greater than " .. tostring(MAX_SCALE)
    end

    return value, nil
end

local function normalize_options(opts: any, allowed: { [string]: boolean }): (any, string?)
    if opts == nil then
        return default_options(), nil
    end

    if type(opts) ~= "table" then
        return nil, "options must be a table or nil"
    end

    local unknown = reject_unknown(opts, allowed)
    if unknown ~= nil then
        return nil, unknown
    end

    if opts.dpi ~= nil and opts.scale ~= nil then
        return nil, "set only one of dpi or scale, not both"
    end

    local options = default_options()

    if opts.dpi ~= nil then
        local dpi, dpi_err = validate_dpi(opts.dpi)
        if dpi_err ~= nil then
            return nil, dpi_err
        end
        options.scaling.dpi = dpi
    end

    if opts.scale ~= nil then
        local scale, scale_err = validate_scale(opts.scale)
        if scale_err ~= nil then
            return nil, scale_err
        end
        options.scaling.scale = scale
    end

    if opts.format ~= nil then
        if type(opts.format) ~= "string" or not VALID_FORMATS[opts.format:lower()] then
            return nil, "format must be one of png, jpeg, webp when provided"
        end
        options.output.format = opts.format:lower()
    end

    if opts.quality ~= nil then
        if type(opts.quality) ~= "number" or opts.quality < 1 or opts.quality > 100 then
            return nil, "quality must be a number between 1 and 100 when provided"
        end
        options.output.quality = math.floor(opts.quality)
    end

    if opts.background ~= nil then
        if type(opts.background) ~= "string" or not valid_background(opts.background) then
            return nil, "background must be 'transparent' or a hex color when provided"
        end
        options.output.background = opts.background
    end

    local format = options.output.format
    if (format == "jpeg" or format == "jpg") and background_alpha(options.output.background) < 255 then
        return nil, "a transparent or translucent background is not supported for jpeg"
    end

    return options, nil
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

    local function resolve_source(request: SourceRequest): (string?, error?)
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

    local function open(request: SourceRequest): (any, error?)
        local blob, src_err = resolve_source(request)
        if src_err ~= nil then
            return nil, src_err
        end

        local result, err = funcs.call(PAGES, blob)
        if err ~= nil then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = tostring(err), details = {code = "internal"}}) :: error)
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

        local count = tonumber(result.ok)
        if count == nil then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "unexpected page count result", details = {code = "internal"}}) :: error)
        end

        local function render_pages(zero_based_pages: { number }, options: any): (any, error?)
            if #zero_based_pages == 0 then
                return {}, nil
            end

            local res, call_err = funcs.call(RENDER, blob, zero_based_pages, options)
            if call_err ~= nil then
                return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = tostring(call_err), details = {code = "internal"}}) :: error)
            end

            if type(res) ~= "table" then
                return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "converter returned no result", details = {code = "internal"}}) :: error)
            end

            if res.err ~= nil then
                local message = (type(res.err) == "table" and tostring(res.err.message)) or tostring(res.err)
                return nil, (errors.new({kind = errors.INTERNAL, retryable = false,
                message = message, details = {code = "render_failed"}}) :: error)
            end

            local rendered = res.ok
            if type(rendered) ~= "table" then
                return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "unexpected render result", details = {code = "internal"}}) :: error)
            end

            local images = {}
            for i = 1, #rendered do
                local item = rendered[i]
                if type(item) ~= "string" then
                    return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "unexpected render result", details = {code = "internal"}}) :: error)
                end

                images[i] = item
            end

            return images, nil
        end

        local function all_pages(): any
            local zero_based = {}
            for page = 1, count do
                zero_based[page] = page - 1
            end

            return zero_based
        end

        local function resolve_pages(selector: any): (any, error?)
            if selector == nil or selector == "all" then
                return all_pages(), nil
            end

            if type(selector) ~= "table" then
                return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "pages must be 'all', a {from, to} table, or an array of page numbers", details = {code = "invalid_input"}}) :: error)
            end

            local zero_based = {}
            local length = #selector

            for i = 1, length do
                local page = tonumber(selector[i]) or 0
                if page < 1 or page > count or page % 1 ~= 0 then
                    return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "page out of range (1.." .. tostring(count) .. ")", details = {code = "out_of_range"}}) :: error)
                end
                zero_based[i] = page - 1
            end

            if length > 0 then
                return zero_based, nil
            end

            if selector.from ~= nil or selector.to ~= nil then
                local from = tonumber(selector.from)
                local to = tonumber(selector.to)

                if selector.from ~= nil and from == nil then
                    return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "pages.from must be a number", details = {code = "invalid_input"}}) :: error)
                end
                if selector.to ~= nil and to == nil then
                    return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "pages.to must be a number", details = {code = "invalid_input"}}) :: error)
                end

                from = from or 1
                to = to or count

                if from % 1 ~= 0 or to % 1 ~= 0 or from < 1 or to > count or from > to then
                    return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "page range out of bounds (1.." .. tostring(count) .. ")", details = {code = "out_of_range"}}) :: error)
                end

                for page = from, to do
                    zero_based[#zero_based + 1] = page - 1
                end
            end

            return zero_based, nil
        end

        local function to_image(page: number, opts: RenderOpts | nil): (string?, error?)
            if type(page) ~= "number" or page % 1 ~= 0 or page < 1 or page > count then
                return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "page out of range (1.." .. tostring(count) .. ")", details = {code = "out_of_range"}}) :: error)
            end

            local options, opt_err = normalize_options(opts, TO_IMAGE_OPTION_KEYS)
            if opt_err ~= nil then
                return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. opt_err, details = {code = "invalid_input"}}) :: error)
            end

            local images, render_err = render_pages({ page - 1 }, options)
            if render_err ~= nil then
                return nil, render_err
            end

            return images[1] :: string?, nil
        end

        local function render(opts: BatchRenderOpts?): (any, error?)
            local options_in = opts or {}
            if type(options_in) ~= "table" then
                return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "render options must be a table", details = {code = "invalid_input"}}) :: error)
            end

            local zero_based, page_err = resolve_pages(options_in.pages)
            if page_err ~= nil then
                return nil, page_err
            end

            local options, opt_err = normalize_options(options_in, RENDER_OPTION_KEYS)
            if opt_err ~= nil then
                return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. opt_err, details = {code = "invalid_input"}}) :: error)
            end

            return render_pages(zero_based :: { number }, options)
        end

        return { pages = count, to_image = to_image, render = render }, nil
    end

    return { open = open }
end

local default = new()

return {
    open = default.open,
    new = new,
    limits = { max_input_bytes = MAX_INPUT_BYTES },
}
