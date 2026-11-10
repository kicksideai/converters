-- SPDX-License-Identifier: MIT

-- A page spec: `images` image XObjects of `size` x `size` gray pixels (the first
-- one shared across pages), optional `text` lines as { size, text } drawn top
-- down in Helvetica, an optional Form XObject wrapping the shared image, and an
-- optional inline image.
type Line = { size: integer, text: string }

type PageSpec = {
    images: integer?,
    size: integer?,
    text: { Line }?,
    form: boolean?,
    inline: boolean?,
}

local function escape(text: string): string
    return (text:gsub("[\\()]", "\\%0"))
end

local function document(specs: { PageSpec }): string
    local objects = { "<< /Type /Catalog /Pages 2 0 R >>", "" }
    local function add(body: string): integer objects[#objects + 1] = body; return #objects end
    local function stream(body: string, attributes: string): integer
        return add("<< /Length " .. #body .. " " .. attributes .. " >>\nstream\n" .. body .. "\nendstream")
    end
    local function image(size: integer, fill: string): integer
        return stream(string.rep(fill, size * size), "/Type /XObject /Subtype /Image /Width " .. size .. " /Height " .. size .. " /ColorSpace /DeviceGray /BitsPerComponent 8")
    end
    local font = add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    local shared: { [integer]: integer } = {}
    local kids = {}
    for page, spec in ipairs(specs) do
        local size = spec.size or 4
        if shared[size] == nil then shared[size] = image(size, "\127") end
        local resources, commands = {}, {}
        local y = 740
        for _, line in ipairs(spec.text or {}) do
            commands[#commands + 1] = "BT /F1 " .. line.size .. " Tf 72 " .. y .. " Td (" .. escape(line.text) .. ") Tj ET"
            y = y - line.size * 2
        end
        for index = 1, spec.images or 0 do
            local name = "I" .. index
            local id = index == 1 and shared[size] or image(size, string.char(page % 256))
            resources[#resources + 1] = "/" .. name .. " " .. id .. " 0 R"
            commands[#commands + 1] = "q 100 0 0 100 0 0 cm /" .. name .. " Do Q"
        end
        if spec.form then
            local nested = stream("q 100 0 0 100 100 100 cm /I Do Q", "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /XObject << /I " .. shared[size] .. " 0 R >> >>")
            resources[#resources + 1] = "/F " .. nested .. " 0 R"
            commands[#commands + 1] = "/F Do"
        end
        if spec.inline then
            commands[#commands + 1] = "q 100 0 0 100 200 200 cm BI /W 1 /H 1 /CS /G /BPC 8 ID \127 EI Q"
        end
        local contents = stream(table.concat(commands, "\n"), "")
        local id = add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 " .. font .. " 0 R >> /XObject << " .. table.concat(resources, " ") .. " >> >> /Contents " .. contents .. " 0 R >>")
        kids[#kids + 1] = id .. " 0 R"
    end
    objects[2] = "<< /Type /Pages /Count " .. #specs .. " /Kids [" .. table.concat(kids, " ") .. "] >>"
    local output, offsets, length = {"%PDF-1.4\n"}, {}, 9
    for index, body in ipairs(objects) do
        offsets[index] = length
        local object = index .. " 0 obj\n" .. body .. "\nendobj\n"
        output[#output + 1], length = object, length + #object
    end
    output[#output + 1] = "xref\n0 " .. (#objects + 1) .. "\n0000000000 65535 f \n"
    for _, offset in ipairs(offsets) do output[#output + 1] = string.format("%010d 00000 n \n", offset) end
    output[#output + 1] = "trailer\n<< /Size " .. (#objects + 1) .. " /Root 1 0 R >>\nstartxref\n" .. length .. "\n%%EOF\n"
    return table.concat(output)
end

-- pdf builds one page per entry of image_counts, each with that many small images.
local function pdf(image_counts: { integer }, form: boolean?, inline: boolean?): string
    local specs: { PageSpec } = {}
    for index, count in ipairs(image_counts) do
        specs[index] = { images = count, form = form, inline = inline }
    end
    return document(specs)
end

return { pdf = pdf, document = document }
