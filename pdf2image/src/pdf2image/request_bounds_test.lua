local test = require("test")
local pdf2image = require("pdf2image")
local fixtures = require("fixtures")

local function define()
    test.describe("pdf2image.request_bounds", function()
        test.it("rejects oversized selections before dispatching any batch", function()
            for _, parallel in ipairs({ false, true }) do
                local renders = 0
                local function render()
                    renders = renders + 1
                    return { ok = {} }, nil
                end
                local funcs = {
                    call = function(target)
                        if target == "kickside.convert.pdf2image:pages" then
                            return { ok = 1025 }, nil
                        end
                        return render()
                    end,
                }
                if parallel then
                    funcs.async = function()
                        local result = render()
                        return {
                            response = function() return { receive = function() end } end,
                            result = function() return { data = function() return result end }, nil end,
                        }, nil
                    end
                end
                local converter = pdf2image.new({ funcs = funcs })
                local doc, open_err = converter.open({ bytes = fixtures.pdf })
                test.is_nil(open_err)
                local pages = {}
                for i = 1, 1025 do pages[i] = 1 end
                for _, selector in ipairs({ "all", { from = 1, to = 1025 }, pages }) do
                    local images, err = doc.render({ pages = selector })
                    test.is_nil(images)
                    test.not_nil(err)
                    test.eq(err:message(), "requested 1025 pages exceeds the limit of 1024")
                    test.eq(err:details().code, "render_failed")
                    test.eq(err:kind(), errors.INTERNAL)
                    test.eq(err:retryable(), false)
                    test.eq(renders, 0)
                end
            end
        end)

        test.it("accepts 1024 pages and refuses 1025 through the component", function()
            local doc, open_err = pdf2image.open({ bytes = fixtures.pdf })
            test.is_nil(open_err)
            local pages = {}
            for i = 1, 1024 do pages[i] = 1 end
            local images, err = doc.render({ pages = pages, scale = 0.02 })
            test.is_nil(err)
            test.eq(#images, 1024)
            test.eq(images[1]:sub(1, 4), "\137PNG")
            test.eq(images[1024], images[1])
            pages[1025] = 1
            local refused, refusal = doc.render({ pages = pages, scale = 0.02 })
            test.is_nil(refused)
            test.not_nil(refusal)
            test.eq(refusal:message(), "requested 1025 pages exceeds the limit of 1024")
            test.eq(refusal:details().code, "render_failed")
        end)

        test.it("encodes JPEG with full chroma sampling", function()
            local doc, open_err = pdf2image.open({ bytes = fixtures.pdf })
            test.is_nil(open_err)
            local jpeg, err = doc.to_image(1, { format = "jpeg", scale = 0.1 })
            test.is_nil(err)
            local offset = 3
            local found = false
            while offset < #jpeg do
                test.eq(jpeg:byte(offset), 255)
                local marker = jpeg:byte(offset + 1)
                local size = jpeg:byte(offset + 2) * 256 + jpeg:byte(offset + 3)
                if marker == 192 then
                    test.eq(jpeg:byte(offset + 9), 3)
                    for component = 0, 2 do
                        test.eq(jpeg:byte(offset + 11 + 3 * component), 17)
                    end
                    found = true
                    break
                end
                offset = offset + 2 + size
            end
            test.eq(found, true)
        end)
    end)
end

return { run = test.run_cases(define) }
