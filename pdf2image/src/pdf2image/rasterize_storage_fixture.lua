-- SPDX-License-Identifier: MIT
local M: {stage: string, err: error?} = {stage = "acquire"}
function M.get(_id: string)
    if M.stage == "acquire" then return nil, M.err end
    local storage = {}
    function storage:readfile(_path: string): (string?, error?) return nil, M.err end
    return storage, nil
end
return M
