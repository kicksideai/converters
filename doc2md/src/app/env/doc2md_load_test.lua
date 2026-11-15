local doc2md = require("doc2md")
local fixtures = require("fixtures")
local time = require("time")

local REPEATS = 1250

local CASES = {
    { name = "pdf", bytes = fixtures.pdf },
    { name = "docx", bytes = fixtures.docx },
    { name = "xlsx", bytes = fixtures.xlsx },
    { name = "pptx", bytes = fixtures.pptx },
    { name = "doc", bytes = fixtures.doc },
    { name = "xls", bytes = fixtures.xls },
    { name = "ppt", bytes = fixtures.ppt },
    { name = "rtf", bytes = fixtures.rtf },
}

local function run_case(case, extract_images)
    local document, err = doc2md.convert({
        bytes = case.bytes,
    }, {
        extract_images = extract_images,
    })
    if err ~= nil then
        error(case.name .. " conversion failed: " .. tostring(err))
    end
    if type(document) ~= "table" or type(document.markdown) ~= "string" or type(document.images) ~= "table" then
        error(case.name .. " conversion returned an invalid document")
    end
    return document
end

local function run_mode(extract_images)
    local start = time.now()
    local conversions = 0
    local markdown_bytes = 0
    local image_count = 0
    local image_bytes = 0

    for _ = 1, REPEATS do
        for _, case in ipairs(CASES) do
            local document = run_case(case, extract_images)
            conversions = conversions + 1
            markdown_bytes = markdown_bytes + #document.markdown
            image_count = image_count + #document.images
            for _, image in ipairs(document.images) do
                image_bytes = image_bytes + #image.data
            end
        end
    end

    local elapsed_ms = time.now():sub(start):milliseconds()
    local per_second = 0
    if elapsed_ms > 0 then
        per_second = conversions * 1000 / elapsed_ms
    end

    local summary = {
        extract_images = extract_images,
        repeats = REPEATS,
        documents_per_repeat = #CASES,
        conversions = conversions,
        elapsed_ms = elapsed_ms,
        conversions_per_second = per_second,
        markdown_bytes = markdown_bytes,
        image_count = image_count,
        image_bytes = image_bytes,
    }

    print(string.format(
        "doc2md load extract_images=%s repeats=%d documents_per_repeat=%d conversions=%d elapsed_ms=%d conversions_per_second=%.2f markdown_bytes=%d image_count=%d image_bytes=%d",
        tostring(summary.extract_images),
        summary.repeats,
        summary.documents_per_repeat,
        summary.conversions,
        summary.elapsed_ms,
        summary.conversions_per_second,
        summary.markdown_bytes,
        summary.image_count,
        summary.image_bytes
    ))

    return summary
end

-- Text pages sharing one Type0 font whose ToUnicode CMap maps CMAP_ENTRIES two-byte
-- codes; every page shows CMAP_RUNS separate text runs through it. Same document as
-- large_cmap_pdf in component/src/pdf_performance.rs.
local CMAP_ENTRIES = 60000
local CMAP_PAGES = 50
local CMAP_RUNS = 200
local RUN_CODES = 8

local function large_cmap_pdf()
    local cmap = {
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    }
    for first = 1, CMAP_ENTRIES, 100 do
        local last = math.min(first + 99, CMAP_ENTRIES)
        local block = { string.format("%d beginbfchar\n", last - first + 1) }
        for code = first, last do
            block[#block + 1] = string.format("<%04X> <%04X>\n", code, 97 + code % 26)
        end
        block[#block + 1] = "endbfchar\n"
        cmap[#cmap + 1] = table.concat(block)
    end
    cmap[#cmap + 1] = "endcmap\nend\nend\n"
    local cmap_text = table.concat(cmap)

    local objects = {
        "<< /Type /Catalog /Pages 2 0 R >>",
        "",
        "<< /Type /Font /Subtype /Type0 /BaseFont /Synthetic /Encoding /Identity-H /DescendantFonts [4 0 R] /ToUnicode 5 0 R >>",
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Synthetic /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 500 >>",
        string.format("<< /Length %d >>\nstream\n%sendstream", #cmap_text, cmap_text),
    }
    local kids = {}
    for page = 0, CMAP_PAGES - 1 do
        local content = {}
        for run = 0, CMAP_RUNS - 1 do
            local first = ((page * CMAP_RUNS + run) * RUN_CODES) % (CMAP_ENTRIES - RUN_CODES) + 1
            local hex = {}
            for code = first, first + RUN_CODES - 1 do
                hex[#hex + 1] = string.format("%04X", code)
            end
            content[#content + 1] = string.format("BT /F1 6 Tf %d %d Td <%s> Tj ET\n",
                36 + (run // 96) * 280, 780 - (run % 96) * 8, table.concat(hex))
        end
        local content_text = table.concat(content)
        objects[#objects + 1] = string.format("<< /Length %d >>\nstream\n%sendstream", #content_text, content_text)
        objects[#objects + 1] = string.format(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents %d 0 R >>",
            #objects)
        kids[#kids + 1] = string.format("%d 0 R", #objects)
    end
    objects[2] = string.format("<< /Type /Pages /Count %d /Kids [%s] >>", CMAP_PAGES, table.concat(kids, " "))

    local pdf = { "%PDF-1.4\n" }
    local size = #pdf[1]
    local offsets = {}
    for index, object in ipairs(objects) do
        offsets[index] = size
        local chunk = string.format("%d 0 obj\n%s\nendobj\n", index, object)
        pdf[#pdf + 1] = chunk
        size = size + #chunk
    end
    pdf[#pdf + 1] = string.format("xref\n0 %d\n0000000000 65535 f \n", #objects + 1)
    for _, offset in ipairs(offsets) do
        pdf[#pdf + 1] = string.format("%010d 00000 n \n", offset)
    end
    pdf[#pdf + 1] = string.format("trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", #objects + 1, size)
    return table.concat(pdf)
end

local function run_cmap()
    local bytes = large_cmap_pdf()
    local start = time.now()
    local document, convert_err = doc2md.convert({ bytes = bytes }, { extract_images = true, page_markers = true })
    local convert_ms = time.now():sub(start):milliseconds()
    start = time.now()
    local pages, pages_err = doc2md.page_resources({ bytes = bytes })
    local pages_ms = time.now():sub(start):milliseconds()
    print(string.format(
        "doc2md load large_cmap pdf_bytes=%d convert_ms=%d convert_err=%s markdown_bytes=%s page_resources_ms=%d page_resources_err=%s pages=%s",
        #bytes, convert_ms, tostring(convert_err), document and #document.markdown or "-",
        pages_ms, tostring(pages_err), pages and #pages or "-"
    ))
    return { convert_ms = convert_ms, page_resources_ms = pages_ms }
end

local function run_off()
    return run_mode(false)
end

local function run_on()
    return run_mode(true)
end

return {
    run_off = run_off,
    run_on = run_on,
    run_cmap = run_cmap,
}
