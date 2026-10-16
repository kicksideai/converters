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

local function run_off()
    return run_mode(false)
end

local function run_on()
    return run_mode(true)
end

return {
    run_off = run_off,
    run_on = run_on,
}
