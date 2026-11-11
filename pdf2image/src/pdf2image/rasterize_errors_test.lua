-- SPDX-License-Identifier: MIT
local test = require("test")
local adapter = require("adapter")
local fixture = require("fixture")
local funcs = require("funcs")
local samples = require("samples")
return {run = test.run_cases(function()
    test.describe("Rasterizer native failures", function()
        for _, stage in ipairs({"open", "render"}) do
            for _, spec in ipairs({{errors.INVALID, false, "decode_failed"}, {errors.INVALID, false, "out_of_range"},
                {errors.TIMEOUT, true, "render_timeout"}, {errors.UNAVAILABLE, true, "pool_exhausted"}}) do
                test.it("preserves " .. spec[3] .. " from " .. stage, function()
                    fixture.stage = stage
                    fixture.err = errors.new({message = "renderer fails", kind = spec[1], retryable = spec[2], details = {code = spec[3]}})
                    local result = adapter.run({ref = {bytes = "input"}, pages = {1}})
                    test.eq(type(result.err), "table")
                    test.eq(result.err.kind, spec[1]); test.eq(result.err.retryable, spec[2])
                    test.eq(result.err.code, spec[3])
                end)
            end
        end
        test.it("keeps corrupt documents permanent through the installed adapter", function()
            local result = assert(funcs.call("kickside.convert.pdf2image:rasterize", {ref = {bytes = "corrupt PDF"}, count_only = true}))
            test.eq(type(result.err), "table"); test.eq(result.err.kind, errors.INVALID)
            test.eq(result.err.retryable, false); test.eq(result.err.code, "decode_failed")
        end)
        test.it("keeps encrypted input permanent through the installed adapter", function()
            local result = assert(funcs.call("kickside.convert.pdf2image:rasterize", {ref = {bytes = samples.encrypted}, pages = {1}}))
            test.not_nil(result.err)
            test.eq(result.err.retryable, false)
            test.eq(result.err.kind, errors.INVALID)
            test.eq(result.err.code, "decode_failed")
        end)
        test.it("keeps out of range permanent through the installed adapter", function()
            local result = assert(funcs.call("kickside.convert.pdf2image:rasterize", {ref = {bytes = samples.pdf}, pages = {999}}))
            test.eq(type(result.err), "table"); test.eq(result.err.kind, errors.INVALID)
            test.eq(result.err.retryable, false); test.eq(result.err.code, "out_of_range")
        end)
    end)
end)}
