local test = require("test")
local doc2md = require("doc2md")
local fixtures = require("fixtures")

local function assert_markdown_document(document, expected)
    test.eq(type(document), "table")
    test.eq(type(document.markdown), "string")
    test.contains(document.markdown, expected)
    test.eq(type(document.images), "table")
end

local function convert(bytes, extract_images)
    return doc2md.convert({ bytes = bytes }, { extract_images = extract_images })
end

local function expect_error(value: any, err: any?): any
    test.is_nil(value)
    if err == nil then
        error("expected an error")
    end
    return err
end

local function fake(call_impl)
    return doc2md.new({
        funcs = { call = call_impl },
        http_client = { get = function() return nil, "http_client must not be used in this test" end },
    })
end

local function define()
    test.describe("doc2md.convert", function()
        test.it("stringifies structured conversion and page-resource error messages", function()
            local mod = fake(function() return { err = { message = 409 } }, nil end)
            for _, operation in ipairs({ mod.convert, mod.page_resources }) do
                local value, err = operation({ bytes = fixtures.pdf })
                test.is_nil(value)
                test.not_nil(err)
                test.eq(err:details().code, "decode_failed")
                test.eq(err:message(), "409")
            end
        end)
        test.it("converts a PDF blob to markdown without images by default", function()
            local document, err = doc2md.convert({ bytes = fixtures.pdf })
            test.is_nil(err)
            assert_markdown_document(document, "Hello from PDF")
            test.eq(#document.images, 0)
        end)

        test.it("converts DOCX and only returns images when requested", function()
            local plain, plain_err = convert(fixtures.docx, false)
            test.is_nil(plain_err)
            assert_markdown_document(plain, "Hello from DOCX")
            test.contains(plain.markdown, "Second paragraph")
            test.eq(#plain.images, 0)

            local with_images, image_err = convert(fixtures.docx, true)
            test.is_nil(image_err)
            assert_markdown_document(with_images, "Hello from DOCX")
            test.eq(#with_images.images, 1)
            test.eq(type(with_images.images[1].name), "string")
            test.contains(with_images.images[1].name, ".")
            test.eq(type(with_images.images[1].data), "string")
            test.eq(#with_images.images[1].data > 0, true)
        end)

        test.it("marks PDF pages and PPTX slides with hidden comments when asked", function()
            local pdf, pdf_err = doc2md.convert({ bytes = fixtures.pdf }, { page_markers = true })
            test.is_nil(pdf_err)
            test.contains(pdf.markdown, "<!-- page 1 -->")
            local pptx, pptx_err = doc2md.convert({ bytes = fixtures.pptx }, { page_markers = true })
            test.is_nil(pptx_err)
            test.contains(pptx.markdown, "<!-- slide 1")
            local plain, plain_err = doc2md.convert({ bytes = fixtures.pdf })
            test.is_nil(plain_err)
            test.is_nil((plain.markdown:find("<!-- page", 1, true)))
        end)

        test.it("passes page_markers to the component", function()
            local seen
            local mod = fake(function(_, request) seen = request.options; return { ok = { markdown = "", images = {} } }, nil end)
            local _, err = mod.convert({ bytes = fixtures.pdf }, { page_markers = true, extract_images = true })
            test.is_nil(err)
            test.eq(seen["page-markers"], true)
            test.eq(seen["extract-images"], true)
            local _, default_err = mod.convert({ bytes = fixtures.pdf })
            test.is_nil(default_err)
            test.eq(seen["page-markers"], false)
        end)

        test.it("rejects a non-boolean page_markers option", function()
            local err = expect_error(doc2md.convert({ bytes = fixtures.pdf }, { page_markers = "yes" }))
            test.eq(err:details().code, "invalid_input")
        end)

        test.it("locates page and slide markers and strips them from text", function()
            local markdown = "<!-- page 1 -->\n\nIntro\n\n<!-- page 2 -->\n\nTerms\n<!-- slide 3: Pricing -->\nPlans"
            local found = doc2md.markers(markdown)
            test.eq(#found, 3)
            test.eq(found[1].kind, "page");test.eq(found[1].number, 1);test.eq(found[1].start, 1)
            test.eq(found[2].kind, "page");test.eq(found[2].number, 2);test.eq(markdown:sub(found[2].start, found[2].finish), "<!-- page 2 -->")
            test.eq(found[3].kind, "slide");test.eq(found[3].number, 3);test.eq(found[3].name, "Pricing")
            test.is_nil(found[1].name)
            local stripped = doc2md.strip_markers(markdown)
            test.is_nil((stripped:find("<!--", 1, true)))
            test.contains(stripped, "Intro");test.contains(stripped, "Terms");test.contains(stripped, "Plans")
            test.eq(#doc2md.markers("no markers here"), 0)
            test.eq(#doc2md.markers("inline <!-- page 4 --> is not a marker line"), 0)
        end)

        test.it("converts XLSX to a markdown table", function()
            local document, err = convert(fixtures.xlsx, false)
            test.is_nil(err)
            assert_markdown_document(document, "Hello from XLSX")
            test.eq(#document.images, 0)
        end)

        test.it("converts PPTX to markdown", function()
            local document, err = convert(fixtures.pptx, false)
            test.is_nil(err)
            assert_markdown_document(document, "Hello from PPTX")
            test.eq(#document.images, 0)
        end)

        test.it("converts legacy DOC and honors image extraction", function()
            local plain, plain_err = convert(fixtures.doc, false)
            test.is_nil(plain_err)
            assert_markdown_document(plain, "Hello from DOC legacy")
            test.contains(plain.markdown, "Second paragraph here")
            test.eq(#plain.images, 0)

            local with_images, image_err = convert(fixtures.doc, true)
            test.is_nil(image_err)
            assert_markdown_document(with_images, "Hello from DOC legacy")
            test.eq(#with_images.images >= 1, true)
            test.eq(type(with_images.images[1].data), "string")
            test.contains(with_images.images[1].name, ".")
            test.eq(#with_images.images[1].data > 0, true)
        end)

        test.it("converts legacy XLS to a markdown table", function()
            local document, err = convert(fixtures.xls, false)
            test.is_nil(err)
            assert_markdown_document(document, "Hello from XLS")
            test.contains(document.markdown, "42")
            test.eq(#document.images, 0)
        end)

        test.it("converts legacy PPT and honors image extraction", function()
            local plain, plain_err = convert(fixtures.ppt, false)
            test.is_nil(plain_err)
            assert_markdown_document(plain, "Hello from PPT legacy")
            test.eq(#plain.images, 0)

            local with_images, image_err = convert(fixtures.ppt, true)
            test.is_nil(image_err)
            assert_markdown_document(with_images, "Hello from PPT legacy")
            test.eq(#with_images.images >= 1, true)
            test.eq(type(with_images.images[1].data), "string")
        end)

        test.it("converts RTF and honors image extraction without leaking hex image data", function()
            local plain, plain_err = convert(fixtures.rtf, false)
            test.is_nil(plain_err)
            assert_markdown_document(plain, "Hello from RTF legacy")
            test.contains(plain.markdown, "Second line of rtf")
            test.eq(#plain.images, 0)
            test.eq(plain.markdown:find("89504e47", 1, true), nil)

            local with_images, image_err = convert(fixtures.rtf, true)
            test.is_nil(image_err)
            assert_markdown_document(with_images, "Hello from RTF legacy")
            test.eq(#with_images.images >= 1, true)
            test.eq(type(with_images.images[1].data), "string")
        end)

        test.it("converts plain-text bytes as a text document", function()
            local document, err = doc2md.convert({ bytes = fixtures.garbage })
            test.is_nil(err)
            assert_markdown_document(document, "plain text bytes")
            test.eq(#document.images, 0)
        end)

        test.it("rejects genuinely unsupported binary with a decode error", function()
            local binary = string.rep(string.char(0x00, 0xFF, 0x13, 0x37, 0x80, 0x01), 16)
            local err = expect_error(doc2md.convert({ bytes = binary }))
            test.eq(err:details().code, "decode_failed")
            test.contains(tostring(err), "unsupported")
        end)

        test.it("rejects a non-table request", function()
            local err = expect_error(doc2md.convert(""))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "request must be a table")
        end)

        test.it("rejects a request with neither source", function()
            local err = expect_error(doc2md.convert({}))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "exactly one source")
        end)

        test.it("rejects a request with both sources", function()
            local err = expect_error(doc2md.convert({ bytes = fixtures.pdf, url = "https://example.com/x.pdf" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "exactly one source")
        end)

        test.it("rejects empty bytes", function()
            local err = expect_error(doc2md.convert({ bytes = "" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "non-empty")
        end)

        test.it("rejects a non-boolean extract_images option", function()
            local err = expect_error(doc2md.convert({ bytes = fixtures.pdf }, { extract_images = "yes" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "extract_images")
        end)

        test.it("rejects an unknown option", function()
            local err = expect_error(doc2md.convert({ bytes = fixtures.pdf }, { extract = true }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "unknown option: extract")
        end)

        test.it("rejects a non-http url scheme", function()
            local err = expect_error(doc2md.convert({ url = "ftp://example.com/x.pdf" }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "http(s) URL")
        end)

        test.it("rejects an oversized bytes source", function()
            local oversized = string.rep("A", (128 * 1024 * 1024) + 1)
            local err = expect_error(doc2md.convert({ bytes = oversized }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "exceeds")
        end)

        test.it("surfaces an internal error when the component returns no result", function()
            local err = expect_error(fake(function() return nil, nil end).convert({ bytes = fixtures.pdf }))
            test.eq(err:details().code, "internal")
            test.contains(tostring(err), "no result")
        end)

        test.it("surfaces an internal error when funcs.call fails", function()
            local err = expect_error(fake(function() return nil, "host exploded" end).convert({ bytes = fixtures.pdf }))
            test.eq(err:details().code, "internal")
            test.contains(tostring(err), "host exploded")
        end)

        test.it("passes through a component decode error", function()
            local err = expect_error(fake(function() return { err = { message = "unsupported format" } }, nil end).convert({ bytes = fixtures.pdf }))
            test.eq(err:details().code, "decode_failed")
            test.contains(tostring(err), "unsupported format")
        end)

        test.it("rejects an invalid document shape from the component", function()
            local err = expect_error(fake(function() return { ok = "not a table" }, nil end).convert({ bytes = fixtures.pdf }))
            test.eq(err:details().code, "internal")
            test.contains(tostring(err), "invalid document")
        end)

        test.it("rejects an invalid images shape from the component", function()
            local err = expect_error(fake(function()
                return { ok = { markdown = "ok", images = { { name = "x.png", data = 42 } } } }, nil
            end).convert({ bytes = fixtures.pdf }))
            test.eq(err:details().code, "internal")
            test.contains(tostring(err), "invalid image data")
        end)

        test.it("exposes a tostring-able native error", function()
            local err = expect_error(doc2md.convert({}))
            test.eq(type(err), "userdata")
            test.eq(type(err:message()), "string")
            test.eq(type(err:details().code), "string")
            test.eq(("got: " .. err):find("doc2md:") ~= nil, true)
        end)

        test.it("exposes max_input_bytes through limits", function()
            test.eq(type(doc2md.limits), "table")
            test.eq(type(doc2md.limits.max_input_bytes), "number")
            test.eq(doc2md.limits.max_input_bytes > 0, true)
        end)
    end)
end

local run_cases = test.run_cases(define)

local function run(options)
    return run_cases(options)
end

return { run = run }
