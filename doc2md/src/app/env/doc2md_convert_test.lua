local test = require("test")
local doc2md = require("doc2md")
local fixtures = require("fixtures")
local pdf_pages = require("pdf_pages")

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


local BULLET = "\149"

-- Synthetic documents for page-range equivalence: text pages with headings and
-- trailing list bullets, full-page image pages, and a mix with empty pages,
-- shared resources, Forms and inline images.
local function range_documents(): { [string]: string }
    local text = {}
    for page = 1, 9 do
        local lines = { { size = 20, text = "Section " .. page } }
        for line = 1, 3 do
            lines[#lines + 1] = { size = 11, text = "Paragraph text for page " .. page .. " line " .. line .. "." }
        end
        if page % 3 == 0 then
            lines[#lines + 1] = { size = 11, text = BULLET }
        end
        text[page] = { text = lines }
    end
    local images = {}
    for page = 1, 7 do
        images[page] = { images = page == 4 and 2 or 1, size = 80 }
    end
    local mixed = {
        { text = { { size = 20, text = "Cover" }, { size = 11, text = "Summary of the document." } } },
        { images = 1, size = 80 },
        {},
        { text = { { size = 11, text = "Notes continue -" } }, images = 1, size = 80 },
        { text = { { size = 11, text = "1." } } },
        { images = 2, size = 4, form = true, inline = true },
        { text = { { size = 14, text = "Closing" }, { size = 11, text = "Last page text." } }, images = 1, size = 80 },
    }
    return {
        text = pdf_pages.document(text),
        images = pdf_pages.document(images),
        mixed = pdf_pages.document(mixed),
    }
end

-- convert_in_ranges converts bytes in consecutive page ranges of `size` pages and
-- concatenates the results in page order.
local function convert_in_ranges(bytes: string, options: any, size: integer): (any, any?)
    local markdown, images, first, total = {}, {}, 1, nil
    repeat
        local ranged = {}
        for key, value in pairs(options) do ranged[key] = value end
        ranged.pages = { first = first, last = first + size - 1 }
        local document, err = doc2md.convert({ bytes = bytes }, ranged)
        if err ~= nil then return nil, err end
        if total ~= nil and document.page_count ~= total then
            error("page_count changed between ranges")
        end
        total = document.page_count
        markdown[#markdown + 1] = document.markdown
        for _, image in ipairs(document.images) do images[#images + 1] = image end
        first = first + size
    until first > total
    return { markdown = table.concat(markdown), images = images, page_count = total }, nil
end

local function same_images(actual: { any }, expected: { any })
    test.eq(#actual, #expected)
    for index, image in ipairs(expected) do
        test.eq(actual[index].name, image.name)
        test.eq(actual[index].page, image.page)
        test.eq(actual[index].data == image.data, true)
    end
end

local function define()
    test.describe("doc2md.convert", function()
        test.it("returns PDF image page identity in page order, including shared resources and incomplete coverage", function()
            for _, counts in ipairs({ {1,1,1,1,1,1,1,1,1,1,1,1}, {2,0,1,1} }) do
                local document, err = doc2md.convert({bytes = pdf_pages.pdf(counts)}, {extract_images = true, page_markers = true})
                test.is_nil(err)
                local index = 0
                for page, count in ipairs(counts) do
                    for _ = 1, count do
                        index = index + 1
                        test.eq(document.images[index].page, page)
                    end
                end
                test.eq(#document.images, index)
                test.eq(#doc2md.markers(document.markdown), #counts)
            end
        end)
        test.it("converts PDF page ranges that concatenate to the whole-document conversion", function()
            for name, bytes in pairs(range_documents()) do
                for _, options in ipairs({
                    { extract_images = true, page_markers = true },
                    { extract_images = false, page_markers = true },
                    { extract_images = true, page_markers = false },
                }) do
                    local whole, whole_err = doc2md.convert({ bytes = bytes }, options)
                    test.is_nil(whole_err)
                    for _, size in ipairs({ 1, 2, 3, 100 }) do
                        local ranged, err = convert_in_ranges(bytes, options, size)
                        test.is_nil(err)
                        if ranged.markdown ~= whole.markdown then
                            error(name .. " in ranges of " .. size .. " differs:\n--- whole\n" .. whole.markdown .. "\n--- ranged\n" .. ranged.markdown)
                        end
                        test.eq(ranged.page_count, whole.page_count)
                        same_images(ranged.images, whole.images)
                    end
                end
            end
        end)

        test.it("reports the document page count with every conversion", function()
            local documents = range_documents()
            local whole, err = doc2md.convert({ bytes = documents.mixed }, { page_markers = true })
            test.is_nil(err)
            test.eq(whole.page_count, 7)
            local ranged, ranged_err = doc2md.convert({ bytes = documents.mixed }, { page_markers = true, pages = { first = 2, last = 3 } })
            test.is_nil(ranged_err)
            test.eq(ranged.page_count, 7)
            local found = doc2md.markers(ranged.markdown)
            test.eq(#found, 2)
            test.eq(found[1].number, 2)
            test.eq(found[2].number, 3)
            local docx, docx_err = doc2md.convert({ bytes = fixtures.docx })
            test.is_nil(docx_err)
            test.eq(docx.page_count, 1)
        end)

        test.it("keeps one marker line per PDF page whatever the previous page ends with", function()
            local page_counts = { text = 9, images = 7, mixed = 7 }
            for name, bytes in pairs(range_documents()) do
                local document, err = doc2md.convert({ bytes = bytes }, { page_markers = true })
                test.is_nil(err)
                local found = doc2md.markers(document.markdown)
                if #found ~= page_counts[name] then
                    error(name .. ": " .. #found .. " marker lines for " .. page_counts[name] .. " pages:\n" .. document.markdown)
                end
                for index, marker in ipairs(found) do test.eq(marker.number, index) end
            end
        end)

        test.it("converts a range ending past the last page through the last page", function()
            local bytes = range_documents().mixed
            local whole, err = doc2md.convert({ bytes = bytes }, { page_markers = true })
            test.is_nil(err)
            local head, head_err = doc2md.convert({ bytes = bytes }, { page_markers = true, pages = { first = 1, last = 5 } })
            test.is_nil(head_err)
            local tail, tail_err = doc2md.convert({ bytes = bytes }, { page_markers = true, pages = { first = 6, last = 50 } })
            test.is_nil(tail_err)
            test.eq(head.markdown .. tail.markdown, whole.markdown)
        end)

        test.it("treats a non-PDF document as one page", function()
            local whole, err = doc2md.convert({ bytes = fixtures.docx })
            test.is_nil(err)
            local ranged, ranged_err = doc2md.convert({ bytes = fixtures.docx }, { pages = { first = 1, last = 4 } })
            test.is_nil(ranged_err)
            test.eq(ranged.markdown, whole.markdown)
            test.eq(ranged.page_count, 1)
            local beyond = expect_error(doc2md.convert({ bytes = fixtures.docx }, { pages = { first = 2, last = 2 } }))
            test.eq(beyond:details().code, "invalid_input")
        end)

        test.it("rejects a page range that starts past the last page", function()
            local err = expect_error(doc2md.convert({ bytes = range_documents().mixed }, { pages = { first = 8, last = 9 } }))
            test.eq(err:details().code, "invalid_input")
            test.contains(tostring(err), "page")
            local resources_err = expect_error(doc2md.page_resources({ bytes = range_documents().mixed }, { pages = { first = 8, last = 9 } }))
            test.eq(resources_err:details().code, "invalid_input")
        end)

        test.it("rejects malformed page ranges", function()
            for _, pages in ipairs({
                "1-2",
                { first = 0, last = 1 },
                { first = 3, last = 2 },
                { first = 1.5, last = 2 },
                { first = 1 },
                { first = 1, last = 2, step = 1 },
            }) do
                local err = expect_error(doc2md.convert({ bytes = fixtures.pdf }, { pages = pages }))
                test.eq(err:details().code, "invalid_input")
                local resources_err = expect_error(doc2md.page_resources({ bytes = fixtures.pdf }, { pages = pages }))
                test.eq(resources_err:details().code, "invalid_input")
            end
            local unknown = expect_error(doc2md.page_resources({ bytes = fixtures.pdf }, { page_markers = true }))
            test.eq(unknown:details().code, "invalid_input")
        end)

        test.it("passes the page range to the component", function()
            local seen
            local mod = fake(function(_, _, pages) seen = pages; return { ok = { markdown = "", images = {}, ["page-count"] = 3 } }, nil end)
            local document, err = mod.convert({ bytes = fixtures.pdf }, { pages = { first = 2, last = 3 } })
            test.is_nil(err)
            test.eq(seen.first, 2)
            test.eq(seen.last, 3)
            test.eq(document.page_count, 3)
            local _, whole_err = mod.convert({ bytes = fixtures.pdf })
            test.is_nil(whole_err)
            test.is_nil(seen)
        end)

        test.it("reports page resources for a page range as the matching slice of the whole document", function()
            local bytes = range_documents().mixed
            local whole, err = doc2md.page_resources({ bytes = bytes })
            test.is_nil(err)
            test.eq(#whole, 7)
            local ranged, ranged_err = doc2md.page_resources({ bytes = bytes }, { pages = { first = 4, last = 6 } })
            test.is_nil(ranged_err)
            test.eq(#ranged, 3)
            for index, page in ipairs(ranged) do
                local expected = whole[index + 3]
                test.eq(page.page, expected.page)
                test.eq(page.text_len, expected.text_len)
                test.eq(page.image_count, expected.image_count)
            end
        end)

        test.it("counts every image resource and Form before size filtering", function()
            for _, form in ipairs({false, true}) do
                local pages, err = doc2md.page_resources({bytes = pdf_pages.pdf({2,1}, form)})
                test.is_nil(err)
                test.eq(pages[1].image_count, form and 3 or 2)
                test.eq(pages[2].image_count, form and 2 or 1)
            end
        end)
        test.it("counts inline images even without direct image resources", function()
            local pages, err = doc2md.page_resources({bytes = pdf_pages.pdf({1,0}, false, true)})
            test.is_nil(err)
            test.eq(pages[1].image_count, 2)
            test.eq(pages[2].image_count, 1)
        end)
        test.it("preserves optional page identity through the Lua normalizer", function()
            local document, err = fake(function()
                return {ok = {markdown = "", images = {{name = "scan.png", data = "bytes", page = 12}, {name = "figure.png", data = "bytes"}}, ["page-count"] = 12}}, nil
            end).convert({bytes = "data"})
            test.is_nil(err)
            test.eq(document.images[1].page, 12)
            test.is_nil(document.images[2].page)
        end)

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
            local mod = fake(function(_, request) seen = request.options; return { ok = { markdown = "", images = {}, ["page-count"] = 1 } }, nil end)
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

        test.it("rejects a document without a page count from the component", function()
            local err = expect_error(fake(function()
                return { ok = { markdown = "ok", images = {} } }, nil
            end).convert({ bytes = fixtures.pdf }))
            test.eq(err:details().code, "internal")
            test.contains(tostring(err), "invalid page count")
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
