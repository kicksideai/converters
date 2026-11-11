-- SPDX-License-Identifier: MIT
local M: {stage: string, err: error?} = {stage = "acquire"}
function M.get(_id: string)
    if M.stage == "acquire" then return nil, M.err end
    return {
        exists = function(_self, _path: string): boolean return true end,
        readfile = function(_self, _path: string): (string?, error?) return nil, M.err end,
    }, nil
end
return M
