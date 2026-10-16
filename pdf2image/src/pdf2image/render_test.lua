local test = require("test")
local pdf2image = require("pdf2image")
local fixtures = require("fixtures")

local function is_png(blob)
    return type(blob) == "string"
        and #blob > 8
        and blob:byte(1) == 0x89
        and blob:byte(2) == 0x50
        and blob:byte(3) == 0x4E
        and blob:byte(4) == 0x47
end

local function is_jpeg(blob)
    return type(blob) == "string"
        and #blob > 3
        and blob:byte(1) == 0xFF
        and blob:byte(2) == 0xD8
        and blob:byte(3) == 0xFF
end

local function is_webp(blob)
    return type(blob) == "string"
        and #blob > 12
        and blob:sub(1, 4) == "RIFF"
        and blob:sub(9, 12) == "WEBP"
end

local function open_ok(blob)
    local doc, err = pdf2image.open({ bytes = blob })
    test.is_nil(err)
    if doc == nil then
        error("expected a document handle")
    end
    return doc
end

local function expect_error(value: any, err: any?): any
    test.is_nil(value)
    if err == nil then
        error("expected an error")
    end
    return err
end

local function fake(call_impl)
    return pdf2image.new({
        funcs = { call = call_impl },
        http_client = { get = function() return nil, "http_client must not be used in this test" end },
    })
end

local function define()
    test.describe("pdf2image.render", function()
        test.it("stringifies structured page-count and rendering error messages", function()
            local mod = fake(function() return { err = { message = 409 } }, nil end)
            local value, err = mod.open({ bytes = fixtures.pdf })
            test.is_nil(value)
            test.not_nil(err)
            test.eq(err:details().code, "decode_failed")
            test.eq(err:message(), "409")
            local calls = 0
            local renderer = fake(function()
                calls = calls + 1
                if calls == 1 then return { ok = 1 }, nil end
                return { err = { message = 422 } }, nil
            end)
            local doc, open_err = renderer.open({ bytes = fixtures.pdf })
            test.is_nil(open_err)
            test.not_nil(doc)
            local image, render_err = doc.to_image(1)
            test.is_nil(image)
            test.not_nil(render_err)
            test.eq(render_err:details().code, "render_failed")
            test.eq(render_err:message(), "422")
        end)
        test.it("reports the page count of a multi-page PDF", function()
            local doc = open_ok(fixtures.pdf)
            test.eq(doc.pages, 2)
            test.eq(type(doc.to_image), "function")
            test.eq(type(doc.render), "function")
        end)

        test.it("reports the page count of a single-page PDF", function()
            local doc = open_ok(fixtures.text_pdf)
            test.eq(doc.pages, 1)
        end)

        test.it("renders each page to a PNG blob", function()
            local doc = open_ok(fixtures.pdf)

            for page = 1, doc.pages do
                local png, render_err = doc.to_image(page)
                test.is_nil(render_err)
                if png == nil then
                    error("expected PNG bytes")
                end
                test.eq(is_png(png), true)
                test.eq(#png > 100, true)
            end
        end)

        test.it("honours an explicit DPI option", function()
            local doc = open_ok(fixtures.pdf)
            local low = doc.to_image(1, { dpi = 72 })
            local high = doc.to_image(1, { dpi = 200 })
            if low == nil or high == nil then
                error("expected PNG bytes")
            end
            test.eq(is_png(low), true)
            test.eq(is_png(high), true)
            test.eq(#high > #low, true)
        end)

        test.it("accepts an options table with an explicit DPI", function()
            local doc = open_ok(fixtures.pdf)
            local png, err = doc.to_image(1, { dpi = 150 })
            test.is_nil(err)
            test.eq(is_png(png), true)
        end)

        test.it("scales relative to 72 DPI and matches the equivalent DPI", function()
            local doc = open_ok(fixtures.pdf)
            local one = doc.to_image(1, { scale = 1 })
            local two = doc.to_image(1, { scale = 2 })
            local four = doc.to_image(1, { scale = 4 })
            test.eq(is_png(one), true)
            test.eq(#two > #one, true)
            test.eq(#four > #two, true)

            local dpi144 = doc.to_image(1, { dpi = 144 })
            test.eq(dpi144, two)
        end)

        test.it("rejects a non-positive scale", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { scale = 0 }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "scale must be a positive")
        end)

        test.it("rejects a scale above the maximum", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { scale = 9 }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "scale must be a positive")
        end)

        test.it("rejects a non-positive dpi", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { dpi = 0 }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "dpi must be a positive")
        end)

        test.it("rejects a dpi above the maximum", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { dpi = 5000 }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "dpi must be a positive")
        end)

        test.it("rejects a DPI option of zero", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { dpi = 0 }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "dpi must be a positive")
        end)

        test.it("rejects a quality below one", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { format = "jpeg", quality = 0 }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "between 1 and 100")
        end)

        test.it("rejects setting both dpi and scale", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { dpi = 150, scale = 2 }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "only one of dpi or scale")
        end)

        test.it("rejects an unknown render option", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { dpy = 150 }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "unknown option: dpy")
        end)

        test.it("renders JPEG output", function()
            local doc = open_ok(fixtures.pdf)
            local jpg, err = doc.to_image(1, { format = "jpeg" })
            test.is_nil(err)
            test.eq(is_jpeg(jpg), true)
        end)

        test.it("accepts an uppercase format name", function()
            local doc = open_ok(fixtures.pdf)
            local jpg, err = doc.to_image(1, { format = "JPEG" })
            test.is_nil(err)
            test.eq(is_jpeg(jpg), true)
        end)

        test.it("honours JPEG quality", function()
            local doc = open_ok(fixtures.pdf)
            local low = doc.to_image(1, { format = "jpeg", quality = 10 })
            local high = doc.to_image(1, { format = "jpeg", quality = 95 })
            test.eq(is_jpeg(low), true)
            test.eq(is_jpeg(high), true)
            test.eq(#high > #low, true)
        end)

        test.it("renders WEBP output", function()
            local doc = open_ok(fixtures.pdf)
            local webp, err = doc.to_image(1, { format = "webp" })
            test.is_nil(err)
            test.eq(is_webp(webp), true)
        end)

        test.it("applies a custom background color", function()
            local doc = open_ok(fixtures.pdf)
            local white = doc.to_image(1)
            local red = doc.to_image(1, { background = "#FF0000" })
            test.eq(is_png(white), true)
            test.eq(is_png(red), true)
            test.eq(white ~= red, true)
        end)

        test.it("accepts an 8-digit hex background with alpha", function()
            local doc = open_ok(fixtures.pdf)
            local png, err = doc.to_image(1, { background = "#FF000080" })
            test.is_nil(err)
            test.eq(is_png(png), true)
        end)

        test.it("accepts a hex background without a leading hash", function()
            local doc = open_ok(fixtures.pdf)
            local png, err = doc.to_image(1, { background = "00FF00" })
            test.is_nil(err)
            test.eq(is_png(png), true)
        end)

        test.it("rejects a translucent background for JPEG", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { format = "jpeg", background = "#FF000080" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "translucent")
        end)

        test.it("accepts an opaque 8-digit hex background for JPEG", function()
            local doc = open_ok(fixtures.pdf)
            local jpg, err = doc.to_image(1, { format = "jpeg", background = "#FF0000FF" })
            test.is_nil(err)
            test.eq(is_jpeg(jpg), true)
        end)

        test.it("renders a transparent background", function()
            local doc = open_ok(fixtures.pdf)
            local opaque = doc.to_image(1)
            local transparent = doc.to_image(1, { background = "transparent" })
            test.eq(is_png(transparent), true)
            test.eq(opaque ~= transparent, true)
        end)

        test.it("rejects a transparent background for JPEG", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { format = "jpeg", background = "transparent" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "transparent")
        end)

        test.it("rejects a transparent background for the jpg alias", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { format = "jpg", background = "transparent" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "transparent")
        end)

        test.it("rejects an unknown format", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { format = "gif" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "format")
        end)

        test.it("rejects an invalid background color", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { background = "#zzz" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "background")
        end)

        test.it("rejects a quality just above the maximum", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1, { format = "jpeg", quality = 101 }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "between 1 and 100")
        end)

        test.it("batch renders all pages", function()
            local doc = open_ok(fixtures.pdf)
            local images, err = doc.render({})
            test.is_nil(err)
            test.eq(#images, 2)
            test.eq(is_png(images[1]), true)
            test.eq(is_png(images[2]), true)
        end)

        test.it("batch renders an explicit page range", function()
            local doc = open_ok(fixtures.pdf)
            local images, err = doc.render({ pages = { from = 1, to = 2 } })
            test.is_nil(err)
            test.eq(#images, 2)
        end)

        test.it("batch renders an explicit list of pages", function()
            local doc = open_ok(fixtures.pdf)
            local images, err = doc.render({ pages = { 2 } })
            test.is_nil(err)
            test.eq(#images, 1)
            test.eq(is_png(images[1]), true)
        end)

        test.it("batch render matches the single-page render byte for byte", function()
            local doc = open_ok(fixtures.pdf)
            local single = doc.to_image(1)
            local images = doc.render({ pages = { 1 } })
            test.eq(images[1], single)
        end)

        test.it("batch render honours output options", function()
            local doc = open_ok(fixtures.pdf)
            local images, err = doc.render({ pages = "all", format = "jpeg" })
            test.is_nil(err)
            test.eq(is_jpeg(images[1]), true)
            test.eq(is_jpeg(images[2]), true)
        end)

        test.it("batch render rejects an unknown option", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.render({ formats = "jpeg" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "unknown option: formats")
        end)

        test.it("batch render rejects a non-integer page", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.render({ pages = { 1.5 } }))
            test.eq(err:details().code, "out_of_range")
        end)

        test.it("batch render rejects an out-of-range page", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.render({ pages = { 3 } }))
            test.eq(err:details().code, "out_of_range")
            test.contains(tostring(err), "out of range (1..")
        end)

        test.it("batch render rejects a page index below one", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.render({ pages = { 0 } }))
            test.eq(err:details().code, "out_of_range")
            test.contains(tostring(err), "out of range (1..")
        end)

        test.it("batch render rejects an out-of-bounds range", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.render({ pages = { from = 1, to = 9 } }))
            test.eq(err:details().code, "out_of_range")
            test.contains(tostring(err), "out of bounds")
        end)

        test.it("batch render rejects a range starting below one", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.render({ pages = { from = 0, to = 1 } }))
            test.eq(err:details().code, "out_of_range")
            test.contains(tostring(err), "out of bounds")
        end)

        test.it("batch render rejects a reversed range", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.render({ pages = { from = 2, to = 1 } }))
            test.eq(err:details().code, "out_of_range")
            test.contains(tostring(err), "out of bounds")
        end)

        test.it("rejects a page index above the page count", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(3))
            test.eq(err:details().code, "out_of_range")
            test.contains(tostring(err), "out of range")
        end)

        test.it("rejects a page index below one", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(0))
            test.eq(err:details().code, "out_of_range")
            test.contains(tostring(err), "out of range")
        end)

        test.it("rejects a non-integer page", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image(1.5))
            test.eq(err:details().code, "out_of_range")
        end)

        test.it("rejects a non-number page", function()
            local doc = open_ok(fixtures.pdf)
            local err = expect_error(doc.to_image("1"))
            test.eq(err:details().code, "out_of_range")
        end)

        test.it("rejects an unrecognized blob with a decode error", function()
            local err = expect_error(pdf2image.open({ bytes = fixtures.garbage }))
            test.eq(err:details().code, "decode_failed")
        end)

        test.it("rejects an empty bytes source", function()
            local err = expect_error(pdf2image.open({ bytes = "" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "non-empty binary string")
        end)

        test.it("rejects a non-table request", function()
            local err = expect_error(pdf2image.open("not a table"))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "request must be a table")
        end)

        test.it("rejects a request with neither source", function()
            local err = expect_error(pdf2image.open({}))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "exactly one source")
        end)

        test.it("rejects a request with both sources", function()
            local err = expect_error(pdf2image.open({ bytes = fixtures.pdf, url = "https://example.com/x.pdf" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "exactly one source")
        end)

        test.it("rejects a non-http url scheme", function()
            local err = expect_error(pdf2image.open({ url = "ftp://example.com/x.pdf" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "http(s) URL")
        end)

        test.it("rejects an empty url", function()
            local err = expect_error(pdf2image.open({ url = "" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "non-empty string")
        end)

        test.it("rejects an oversized bytes source", function()
            local oversized = string.rep("A", (128 * 1024 * 1024) + 1)
            local err = expect_error(pdf2image.open({ bytes = oversized }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "exceeds")
        end)

        test.it("surfaces an internal error when the component returns no result", function()
            local mod = fake(function() return nil, nil end)
            local err = expect_error(mod.open({ bytes = fixtures.text_pdf }))
            test.eq(err:details().code, "internal")
            test.contains(tostring(err), "no result")
        end)

        test.it("surfaces an internal error when funcs.call fails", function()
            local mod = fake(function() return nil, "host exploded" end)
            local err = expect_error(mod.open({ bytes = fixtures.text_pdf }))
            test.eq(err:details().code, "internal")
            test.contains(tostring(err), "host exploded")
        end)

        test.it("passes through a component decode error from open", function()
            local mod = fake(function() return { err = { message = "corrupt pdf" } }, nil end)
            local err = expect_error(mod.open({ bytes = fixtures.text_pdf }))
            test.eq(err:details().code, "decode_failed")
            test.contains(tostring(err), "corrupt pdf")
        end)

        test.it("rejects an unexpected page count type", function()
            local mod = fake(function() return { ok = "not a number" }, nil end)
            local err = expect_error(mod.open({ bytes = fixtures.text_pdf }))
            test.eq(err:details().code, "internal")
            test.contains(tostring(err), "page count")
        end)

        test.it("returns the raw render bytes unchanged", function()
            local mod = fake(function(target)
                if target == "kickside.convert.pdf2image:pages" then
                    return { ok = 1 }, nil
                end
                return { ok = { "\137PNG raw" } }, nil
            end)
            local doc = mod.open({ bytes = fixtures.text_pdf })
            local png, err = doc.to_image(1)
            test.is_nil(err)
            test.eq(png, "\137PNG raw")
        end)

        test.it("passes through a component render error", function()
            local mod = fake(function(target)
                if target == "kickside.convert.pdf2image:pages" then
                    return { ok = 2 }, nil
                end
                return { err = { message = "render exploded" } }, nil
            end)
            local doc = mod.open({ bytes = fixtures.text_pdf })
            local err = expect_error(doc.render({}))
            test.eq(err:details().code, "render_failed")
            test.contains(tostring(err), "render exploded")
        end)

        test.it("exposes a tostring-able native error", function()
            local err = expect_error(pdf2image.open({}))
            test.eq(type(err), "userdata")
            test.eq(type(err:message()), "string")
            test.eq(type(err:details().code), "string")
            test.eq(err:retryable(), false)
            test.eq(("got: " .. err):find("kickside.convert.pdf2image:") ~= nil, true)
        end)

        test.it("exposes max_input_bytes through limits", function()
            test.eq(type(pdf2image.limits), "table")
            test.eq(type(pdf2image.limits.max_input_bytes), "number")
            test.eq(pdf2image.limits.max_input_bytes > 0, true)
        end)
    end)
end

local run_cases = test.run_cases(define)

local function run(options)
    return run_cases(options)
end

return { run = run }
