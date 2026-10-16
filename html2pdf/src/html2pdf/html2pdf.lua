type RenderRequest = {
    html: string?,
    url: string?,
}

type RenderOptions = {
    page_size: string?,
    landscape: boolean?,
    margin_mm: number?,
}

type Runtime = {
    funcs: any?,
    http_client: any?,
}

local PREFIX = "kickside.convert.html2pdf: "
local RENDER = "kickside.convert.html2pdf:render"

local DEFAULT_PAGE_SIZE = "A4"
local DEFAULT_MARGIN_MM = 10.0
local MAX_INPUT_BYTES = 128 * 1024 * 1024
local DOWNLOAD_TIMEOUT = "30s"
local ALLOWED_SCHEME = "^https?://"

local VALID_PAGE_SIZES = {
    A4 = true,
    A3 = true,
    A5 = true,
    Letter = true,
    Legal = true,
}

local OPTION_KEYS = {
    page_size = true,
    landscape = true,
    margin_mm = true,
}

local function reject_unknown(options: { [string]: any }, allowed: { [string]: boolean }): string?
    for key in pairs(options) do
        if allowed[key] ~= true then
            return "unknown option: " .. tostring(key)
        end
    end

    return nil
end

local function normalize_options(options: RenderOptions?): (any, string?)
    if options == nil then
        return {
            ["page-size"] = DEFAULT_PAGE_SIZE,
            landscape = false,
            ["margin-mm"] = DEFAULT_MARGIN_MM,
        }, nil
    end

    if type(options) ~= "table" then
        return nil, "options must be a table when provided"
    end

    local unknown = reject_unknown(options, OPTION_KEYS)
    if unknown ~= nil then
        return nil, unknown
    end

    local page_size = DEFAULT_PAGE_SIZE
    if options.page_size ~= nil then
        if type(options.page_size) ~= "string" or not VALID_PAGE_SIZES[options.page_size] then
            return nil, "page_size must be one of A4, A3, A5, Letter, Legal when provided"
        end
        page_size = options.page_size
    end

    local landscape = false
    if options.landscape ~= nil then
        if type(options.landscape) ~= "boolean" then
            return nil, "landscape must be a boolean when provided"
        end
        landscape = options.landscape
    end

    local margin_mm = DEFAULT_MARGIN_MM
    if options.margin_mm ~= nil then
        if type(options.margin_mm) ~= "number" or options.margin_mm < 0 then
            return nil, "margin_mm must be a non-negative number when provided"
        end
        margin_mm = options.margin_mm
    end

    return {
        ["page-size"] = page_size,
        landscape = landscape,
        ["margin-mm"] = margin_mm,
    }, nil
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

    local function resolve_source(request: RenderRequest): (string?, error?)
        if type(request) ~= "table" then
            return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "request must be a table", details = {code = "invalid_input"}}) :: error)
        end

        local has_html = request.html ~= nil
        local has_url = request.url ~= nil
        if has_html == has_url then
            return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "request must set exactly one source: html or url", details = {code = "invalid_input"}}) :: error)
        end

        local html: string
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
            html = body :: string
        else
            if type(request.html) ~= "string" or #request.html == 0 then
                return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "html must be a non-empty string", details = {code = "invalid_input"}}) :: error)
            end
            html = request.html :: string
        end

        if #html > MAX_INPUT_BYTES then
            return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. "input exceeds " .. tostring(MAX_INPUT_BYTES) .. " bytes", details = {code = "invalid_input"}}) :: error)
        end

        return html, nil
    end

    local function render(request: RenderRequest, options: RenderOptions?): (string?, error?)
        local html, src_err = resolve_source(request)
        if src_err ~= nil then
            return nil, src_err
        end

        local wasm_options, opt_err = normalize_options(options)
        if opt_err ~= nil then
            return nil, (errors.new({kind = errors.INVALID, retryable = false,
                message = PREFIX .. opt_err, details = {code = "invalid_input"}}) :: error)
        end

        local result, err = funcs.call(RENDER, html, wasm_options)
        if err ~= nil then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = tostring(err), details = {code = "internal"}}) :: error)
        end

        if type(result) ~= "table" then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = true,
                message = PREFIX .. "converter returned no result", details = {code = "internal"}}) :: error)
        end

        if result.err ~= nil then
            local message = (type(result.err) == "table" and result.err.message) or tostring(result.err)
            return nil, (errors.new({kind = errors.INTERNAL, retryable = false,
                message = message, details = {code = "render_failed"}}) :: error)
        end

        local pdf = result.ok
        if type(pdf) ~= "string" or #pdf == 0 then
            return nil, (errors.new({kind = errors.INTERNAL, retryable = false,
                message = PREFIX .. "render produced no PDF bytes", details = {code = "render_failed"}}) :: error)
        end

        return pdf :: string?, nil
    end

    return { render = render }
end

local default = new()

return {
    render = default.render,
    new = new,
    limits = { max_input_bytes = MAX_INPUT_BYTES },
}
