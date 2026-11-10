-- SPDX-License-Identifier: MIT
local function pdf(image_counts: { integer }, form: boolean?, inline: boolean?, texts: { string }?): string
    local objects = { "<< /Type /Catalog /Pages 2 0 R >>", "" }
    local function add(body: string): integer objects[#objects + 1] = body; return #objects end
    local function stream(body: string, attributes: string): integer
        return add("<< /Length " .. #body .. " " .. attributes .. " >>\nstream\n" .. body .. "\nendstream")
    end
    local font = texts and add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    local kids = {}
    local shared = stream(string.rep("\127", 16), "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8")
    for page, count in ipairs(image_counts) do
        local resources, commands = {}, {}
        for index = 1, count do
            local name = "I" .. index
            local image = index == 1 and shared or stream(string.rep(string.char(page), 16), "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8")
            resources[#resources + 1] = "/" .. name .. " " .. image .. " 0 R"
            commands[#commands + 1] = "q 100 0 0 100 0 0 cm /" .. name .. " Do Q"
        end
        if form then
            local nested = stream("q 100 0 0 100 100 100 cm /I Do Q", "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /XObject << /I " .. shared .. " 0 R >> >>")
            resources[#resources + 1] = "/F " .. nested .. " 0 R"
            commands[#commands + 1] = "/F Do"
        end
        if inline then
            commands[#commands + 1] = "q 100 0 0 100 200 200 cm BI /W 1 /H 1 /CS /G /BPC 8 ID \127 EI Q"
        end
        if texts and texts[page] then
            local text = texts[page]:gsub("\\", "\\\\"):gsub("%(", "\\("):gsub("%)", "\\)")
            commands[#commands + 1] = "BT /F1 12 Tf 40 740 Td (" .. text .. ") Tj ET"
        end
        local contents = stream(table.concat(commands, "\n"), "")
        local id = add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << " .. table.concat(resources, " ") .. " >> " .. (font and "/Font << /F1 " .. font .. " 0 R >> " or "") .. ">> /Contents " .. contents .. " 0 R >>")
        kids[#kids + 1] = id .. " 0 R"
    end
    objects[2] = "<< /Type /Pages /Count " .. #image_counts .. " /Kids [" .. table.concat(kids, " ") .. "] >>"
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
return { pdf = pdf }
