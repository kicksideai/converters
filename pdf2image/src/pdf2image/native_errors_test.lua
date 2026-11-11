local test = require("test")
local converter = require("converter")
local function define()
    test.describe("native converter errors", function()
        test.it("exposes native classification and a stable validation code", function()
            local value, err = converter.open({})
            test.is_nil(value)
            test.eq(err:kind(), errors.INVALID)
            test.eq(err:retryable(), false)
            test.eq(err:details().code, "invalid_input")
            test.contains(err:message(), "exactly one source")
        end)
        test.it("classifies downloads as retryable with a stable code", function()
            local client = converter.new({http_client = {get = function() return nil, "network refused" end}})
            local value, err = client.open({url = "https://example.test/input"})
            test.is_nil(value)
            test.eq(err:kind(), errors.UNAVAILABLE)
            test.eq(err:retryable(), true)
            test.eq(err:details().code, "download_failed")
            test.contains(err:message(), "network refused")
        end)
        for _, stage in ipairs({"pages", "render"}) do
            for _, spec in ipairs({{errors.TIMEOUT, true, "render_timeout"},
                {errors.UNAVAILABLE, true, "pool_exhausted"},
                {errors.INVALID, false, "decode_failed"}, {errors.INVALID, false, "out_of_range"}}) do
                test.it("preserves native " .. spec[3] .. " from " .. stage, function()
                    local cause = errors.new({kind = spec[1], retryable = spec[2],
                        message = "native call failed", details = {code = spec[3]}})
                    local client = converter.new({funcs = {call = function(id)
                        if id == "kickside.convert.pdf2image:" .. stage then return nil, cause end
                        return {ok = 2}, nil
                    end}})
                    local value, err = client.open({bytes = "input"})
                    if stage == "render" then
                        test.is_nil(err)
                        value, err = value.render({pages = {1}})
                    end
                    test.is_nil(value)
                    test.eq(err:kind(), cause:kind())
                    test.eq(err:retryable(), cause:retryable())
                    test.eq(err:details().code, cause:details().code)
                    test.eq(err:message(), cause:message())
                end)
            end
        end
        test.it("classifies component failures without retrying invalid input", function()
            local client = converter.new({funcs = {call = function() return {err = {message = "invalid document"}}, nil end}})
            local value, err = client.open({bytes = "input"})
            test.is_nil(value)
            test.eq(err:kind(), errors.INVALID)
            test.eq(err:retryable(), false)
            test.eq(err:details().code, "decode_failed")
            test.eq(err:message(), "invalid document")
        end)
    end)
end
return {run = test.run_cases(define)}
