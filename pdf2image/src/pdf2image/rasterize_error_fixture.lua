local types = require("types")
local M: {stage: string, err: error?} = {stage = "open"}
function M.open(_ref: types.RasterizeRef): (types.PdfDocument?, error?)
    if M.stage == "open" then return nil, M.err end
    return {pages = 2, render = function(_options: types.RasterizeOptions): ({string}?, error?) return nil, M.err end}, nil
end
return M
