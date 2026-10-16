local test = require("test")

local function define()
    test.describe("pdf2image.smoke", function()
        test.it("loads the wippy test harness", function()
            test.eq(type(test.describe), "function")
            test.eq(type(test.it), "function")
            test.eq(type(test.eq), "function")
        end)
    end)
end

local run_cases = test.run_cases(define)

local function run(options)
    return run_cases(options)
end

return { run = run }
