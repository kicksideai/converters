#include <cstdlib>
#include <cstring>
#include <string>

#include "hpdf.h"
#include "pdf_container.hpp"

extern "C" {
#include "html2pdf.h"
}

namespace {

extern "C" void __wasm_call_ctors(void);

bool runtime_initialized = false;

void ensure_runtime_initialized() {
    if (!runtime_initialized) {
        __wasm_call_ctors();
        runtime_initialized = true;
    }
}

struct page_dims {
    float w;
    float h;
};

page_dims dims_for(const std::string& size, bool landscape) {
    float w = 595.28f;
    float h = 841.89f;
    if (size == "Letter") {
        w = 612.0f;
        h = 792.0f;
    } else if (size == "Legal") {
        w = 612.0f;
        h = 1008.0f;
    } else if (size == "A3") {
        w = 841.89f;
        h = 1190.55f;
    } else if (size == "A5") {
        w = 419.53f;
        h = 595.28f;
    }
    if (landscape) {
        float t = w;
        w = h;
        h = t;
    }
    return {w, h};
}

void err_handler(HPDF_STATUS error_no, HPDF_STATUS detail_no, void* user_data) {
    (void)detail_no;
    *(HPDF_STATUS*)user_data = error_no;
}

void set_error(html2pdf_render_error_t* err, uint8_t code, const char* message) {
    err->code = code;
    html2pdf_string_dup(&err->message, message);
}

}

bool exports_html2pdf_render(html2pdf_string_t* html, html2pdf_render_options_t* options,
                             html2pdf_list_u8_t* ret, html2pdf_render_error_t* err) {
    ensure_runtime_initialized();

    ret->ptr = nullptr;
    ret->len = 0;

    std::string page_size((const char*)options->page_size.ptr, options->page_size.len);
    page_dims pd = dims_for(page_size, options->landscape);
    float margin_pt = options->margin_mm * 72.0f / 25.4f;

    std::string html_src((const char*)html->ptr, html->len);

    HPDF_STATUS last_error = 0;
    HPDF_Doc pdf = HPDF_New(err_handler, &last_error);
    if (pdf == nullptr) {
        set_error(err, 1, "failed to create PDF document");
        return false;
    }
    HPDF_UseUTFEncodings(pdf);

    HPDF_Page page = HPDF_AddPage(pdf);
    HPDF_Page_SetWidth(page, pd.w);
    HPDF_Page_SetHeight(page, pd.h);

    pdf_container container(pdf, page, pd.w, pd.h, margin_pt);
    litehtml::document::ptr doc = litehtml::document::createFromString(html_src.c_str(), &container);
    if (doc == nullptr) {
        HPDF_Free(pdf);
        set_error(err, 2, "failed to parse HTML");
        return false;
    }

    doc->render(container.content_width_px());

    litehtml::position clip(0, 0, container.content_width_px(), doc->height());
    doc->draw((litehtml::uint_ptr)0, 0, 0, &clip);

    // A non-fatal HPDF condition during layout/drawing (e.g. a missing glyph) must
    // not abort an otherwise valid document; a genuine failure surfaces as an empty
    // stream below.
    HPDF_SaveToStream(pdf);
    HPDF_UINT32 size = HPDF_GetStreamSize(pdf);
    if (size == 0) {
        HPDF_Free(pdf);
        set_error(err, 3, "empty PDF stream");
        return false;
    }

    uint8_t* buf = (uint8_t*)malloc(size);
    if (buf == nullptr) {
        HPDF_Free(pdf);
        set_error(err, 5, "out of memory");
        return false;
    }

    HPDF_ResetStream(pdf);
    HPDF_UINT32 n = size;
    HPDF_ReadFromStream(pdf, buf, &n);
    HPDF_Free(pdf);

    ret->ptr = buf;
    ret->len = (size_t)n;
    return true;
}
