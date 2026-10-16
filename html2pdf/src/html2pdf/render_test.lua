local test = require("test")
local html2pdf = require("html2pdf")

local function is_pdf(blob)
    return type(blob) == "string"
        and #blob > 8
        and blob:sub(1, 5) == "%PDF-"
end

local function expect_error(value: any, err: any?): any
    test.is_nil(value)
    if err == nil then
        error("expected an error")
    end
    return err
end

local function fake(call_impl)
    return html2pdf.new({
        funcs = { call = call_impl },
        http_client = { get = function() return nil, "http_client must not be used in this test" end },
    })
end

local function define()
    test.describe("html2pdf.render", function()
        test.it("renders a simple HTML string to PDF bytes", function()
            local pdf, err = html2pdf.render({ html = "<html><body><h1>Hello</h1><p>world</p></body></html>" })
            test.is_nil(err)
            if pdf == nil then
                error("expected PDF bytes")
            end
            test.eq(is_pdf(pdf), true)
            test.eq(#pdf > 100, true)
        end)

        test.it("embeds the bundled font and emits text operators", function()
            local pdf, err = html2pdf.render({ html = "<html><body><p>Hello world</p></body></html>" })
            test.is_nil(err)
            if pdf == nil then
                error("expected PDF bytes")
            end
            test.eq(pdf:find("DejaVu", 1, true) ~= nil, true)
            test.eq(pdf:find("BT", 1, true) ~= nil, true)
        end)

        test.it("renders UTF-8 (accented Latin) without error", function()
            local pdf, err = html2pdf.render({ html = "<p>Ação, coração e pão — àéîõü</p>" })
            test.is_nil(err)
            test.eq(is_pdf(pdf), true)
            test.eq(pdf:find("DejaVu", 1, true) ~= nil, true)
        end)

        test.it("lays out a flexbox row producing real content", function()
            local html = [[<html><body>
                <div style="display:flex">
                  <div style="flex:1;background:#eee">left</div>
                  <div style="flex:2;background:#ccc">right</div>
                </div>
              </body></html>]]
            local pdf, err = html2pdf.render({ html = html })
            test.is_nil(err)
            test.eq(is_pdf(pdf), true)
            test.eq(#pdf > 500, true)
        end)

        test.it("honours page size and landscape options", function()
            local pdf, err = html2pdf.render({ html = "<p>letter landscape</p>" }, {
                page_size = "Letter",
                landscape = true,
            })
            test.is_nil(err)
            test.eq(is_pdf(pdf), true)
        end)

        test.it("honours a custom margin", function()
            local pdf, err = html2pdf.render({ html = "<p>margins</p>" }, { margin_mm = 25 })
            test.is_nil(err)
            test.eq(is_pdf(pdf), true)
        end)

        test.it("rejects an empty html source", function()
            local err = expect_error(html2pdf.render({ html = "" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "non-empty string")
        end)

        test.it("rejects a non-table request", function()
            local dynamic = html2pdf.render :: any
            local err = expect_error(dynamic(42))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "request must be a table")
        end)

        test.it("rejects a request with neither source", function()
            local err = expect_error(html2pdf.render({}))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "exactly one source")
        end)

        test.it("rejects a request with both sources", function()
            local err = expect_error(html2pdf.render({ html = "<p>x</p>", url = "https://example.com/x.html" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "exactly one source")
        end)

        test.it("rejects a non-http url scheme", function()
            local err = expect_error(html2pdf.render({ url = "ftp://example.com/x.html" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "http(s) URL")
        end)

        test.it("rejects an unknown option", function()
            local err = expect_error(html2pdf.render({ html = "<p>x</p>" }, { pagesize = "A4" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "unknown option: pagesize")
        end)

        test.it("rejects an unknown page size", function()
            local err = expect_error(html2pdf.render({ html = "<p>x</p>" }, { page_size = "A6" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "page_size must be one of")
        end)

        test.it("rejects a non-boolean landscape", function()
            local err = expect_error(html2pdf.render({ html = "<p>x</p>" }, { landscape = "yes" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "landscape must be a boolean")
        end)

        test.it("rejects a negative margin", function()
            local err = expect_error(html2pdf.render({ html = "<p>x</p>" }, { margin_mm = -1 }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "margin_mm must be a non-negative")
        end)

        test.it("rejects a non-table options value", function()
            local err = expect_error(html2pdf.render({ html = "<p>x</p>" }, "nope" :: any))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "options must be a table")
        end)

        test.it("rejects an oversized html source", function()
            local oversized = string.rep("A", (128 * 1024 * 1024) + 1)
            local err = expect_error(html2pdf.render({ html = oversized }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "exceeds")
        end)

        test.it("reports a download failure when the http client errors", function()
            local mod = html2pdf.new({
                funcs = { call = function() return nil, "funcs must not be used in this test" end },
                http_client = {
                    get = function() return nil, "connection refused" end,
                },
            })
            local err = expect_error(mod.render({ url = "http://example.com/missing.html" }))
            test.eq(err:details().code, "download_failed")
            test.contains(tostring(err), "download failed")
            test.contains(tostring(err), "connection refused")
        end)

        test.it("surfaces an internal error when the component returns no result", function()
            local err = expect_error(fake(function() return nil, nil end).render({ html = "<p>x</p>" }))
            test.eq(err:details().code, "internal")
            test.contains(tostring(err), "no result")
        end)

        test.it("surfaces an internal error when funcs.call fails", function()
            local err = expect_error(fake(function() return nil, "host exploded" end).render({ html = "<p>x</p>" }))
            test.eq(err:details().code, "internal")
            test.contains(tostring(err), "host exploded")
        end)

        test.it("passes through a component render error", function()
            local err = expect_error(fake(function() return { err = { message = "layout boom" } }, nil end).render({ html = "<p>x</p>" }))
            test.eq(err:details().code, "render_failed")
            test.contains(tostring(err), "layout boom")
        end)

        test.it("flags an empty PDF result", function()
            local err = expect_error(fake(function() return { ok = "" }, nil end).render({ html = "<p>x</p>" }))
            test.eq(err:details().code, "render_failed")
            test.contains(tostring(err), "no PDF bytes")
        end)

        test.it("exposes a tostring-able native error", function()
            local err = expect_error(html2pdf.render({}))
            test.eq(type(err), "userdata")
            test.eq(type(err:message()), "string")
            test.eq(type(err:details().code), "string")
            test.eq(("got: " .. err):find("kickside.convert.html2pdf:") ~= nil, true)
        end)

        test.it("exposes max_input_bytes through limits", function()
            test.eq(type(html2pdf.limits), "table")
            test.eq(type(html2pdf.limits.max_input_bytes), "number")
            test.eq(html2pdf.limits.max_input_bytes > 0, true)
        end)
    end)
end

local run_cases = test.run_cases(define)

local function run(options)
    return run_cases(options)
end

return { run = run }
