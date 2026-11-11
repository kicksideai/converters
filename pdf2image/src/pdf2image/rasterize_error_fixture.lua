-- SPDX-License-Identifier: MIT
local types = require("types")
local converter = require("converter")
local M: {stage: string, err: error?} = {stage = "open"}
function M.open(ref: types.RasterizeRef): (types.PdfDocument?, error?)
    return converter.new({funcs = {call = function(id)
        if id == "kickside.convert.pdf2image:" .. (M.stage == "open" and "pages" or "render") then
            return nil, M.err
        end
        return {ok = 2}, nil
    end}}).open(ref)
end
return M
