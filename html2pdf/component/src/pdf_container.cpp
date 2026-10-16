#include "pdf_container.hpp"
#include "fonts.h"

#include <cstring>
#include <string>
#include <vector>

using namespace litehtml;

namespace {

constexpr float PX_PER_PT = 96.0f / 72.0f;

struct pdf_font {
    HPDF_Font font;
    float size_px;
};

std::string first_family(const std::string& family) {
    size_t comma = family.find(',');
    std::string name = comma == std::string::npos ? family : family.substr(0, comma);
    size_t start = name.find_first_not_of(" \t'\"");
    size_t end = name.find_last_not_of(" \t'\"");
    if (start == std::string::npos) {
        return "";
    }
    std::string out = name.substr(start, end - start + 1);
    for (char& c : out) {
        if (c >= 'A' && c <= 'Z') {
            c = (char)(c - 'A' + 'a');
        }
    }
    return out;
}

int family_enum(const std::string& family) {
    std::string f = first_family(family);
    if (f == "serif" || f == "times" || f == "times new roman" || f == "georgia") {
        return EMBEDDED_FONT_SERIF;
    }
    if (f == "monospace" || f == "courier" || f == "courier new" || f == "consolas" || f == "menlo") {
        return EMBEDDED_FONT_MONO;
    }
    return EMBEDDED_FONT_SANS;
}

int b64_value(char c) {
    if (c >= 'A' && c <= 'Z') return c - 'A';
    if (c >= 'a' && c <= 'z') return c - 'a' + 26;
    if (c >= '0' && c <= '9') return c - '0' + 52;
    if (c == '+') return 62;
    if (c == '/') return 63;
    return -1;
}

std::vector<unsigned char> base64_decode(const std::string& in) {
    std::vector<unsigned char> out;
    out.reserve(in.size() * 3 / 4);
    int bits = 0;
    int acc = 0;
    for (char c : in) {
        int v = b64_value(c);
        if (v < 0) {
            continue;
        }
        acc = (acc << 6) | v;
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            out.push_back((unsigned char)((acc >> bits) & 0xFF));
        }
    }
    return out;
}

}

pdf_container::pdf_container(HPDF_Doc pdf, HPDF_Page page, float page_w_pt, float page_h_pt, float margin_pt)
    : m_pdf(pdf), m_page(page), m_page_w_pt(page_w_pt), m_page_h_pt(page_h_pt), m_margin_pt(margin_pt) {}

float pdf_container::to_pt(float px) const { return px / PX_PER_PT; }
float pdf_container::pdf_x(float px) const { return m_margin_pt + to_pt(px); }
float pdf_container::pdf_y_top(float px) const { return m_page_h_pt - m_margin_pt - to_pt(px); }
float pdf_container::content_width_px() const { return (m_page_w_pt - 2.0f * m_margin_pt) * PX_PER_PT; }

HPDF_Font pdf_container::load_font(int family, bool bold, bool italic) {
    int key = family * 4 + (bold ? 2 : 0) + (italic ? 1 : 0);
    auto it = m_font_cache.find(key);
    if (it != m_font_cache.end()) {
        return it->second;
    }

    HPDF_Font font = nullptr;
    unsigned int size = 0;
    const unsigned char* data = embedded_font(family, bold ? 1 : 0, italic ? 1 : 0, &size);
    const char* name = HPDF_LoadTTFontFromMemory(m_pdf, (const HPDF_BYTE*)data, (HPDF_UINT)size, HPDF_TRUE);
    if (name != nullptr) {
        font = HPDF_GetFont(m_pdf, name, "UTF-8");
    }
    // Fall back to the embedded UTF-8 sans-regular font (never base-14, whose
    // single-byte encodings cannot render the UTF-8 the draw path emits).
    if (font == nullptr && !(family == EMBEDDED_FONT_SANS && !bold && !italic)) {
        font = load_font(EMBEDDED_FONT_SANS, false, false);
    }

    m_font_cache[key] = font;
    return font;
}

uint_ptr pdf_container::create_font(const font_description& descr, const document*, font_metrics* fm) {
    bool bold = descr.weight >= 600;
    bool italic = descr.style == font_style_italic;
    HPDF_Font font = load_font(family_enum(descr.family), bold, italic);

    float size = descr.size;
    if (fm != nullptr) {
        fm->font_size = size;
        fm->ascent = (float)HPDF_Font_GetAscent(font) * size / 1000.0f;
        fm->descent = -(float)HPDF_Font_GetDescent(font) * size / 1000.0f;
        fm->height = fm->ascent + fm->descent;
        fm->x_height = (float)HPDF_Font_GetXHeight(font) * size / 1000.0f;
        HPDF_TextWidth zero = HPDF_Font_TextWidth(font, (const HPDF_BYTE*)"0", 1);
        fm->ch_width = (float)zero.width * size / 1000.0f;
        fm->draw_spaces = true;
        fm->sub_shift = size / 5.0f;
        fm->super_shift = size / 3.0f;
    }

    pdf_font* handle = new pdf_font{font, size};
    return (uint_ptr)handle;
}

void pdf_container::delete_font(uint_ptr hFont) {
    delete (pdf_font*)hFont;
}

pixel_t pdf_container::text_width(const char* text, uint_ptr hFont) {
    pdf_font* f = (pdf_font*)hFont;
    if (f == nullptr || text == nullptr) {
        return 0;
    }
    HPDF_TextWidth tw = HPDF_Font_TextWidth(f->font, (const HPDF_BYTE*)text, (HPDF_UINT)strlen(text));
    return (float)tw.width * f->size_px / 1000.0f;
}

void pdf_container::draw_text(uint_ptr, const char* text, uint_ptr hFont, web_color color, const position& pos) {
    pdf_font* f = (pdf_font*)hFont;
    if (f == nullptr || text == nullptr || text[0] == '\0') {
        return;
    }

    float ascent_px = (float)HPDF_Font_GetAscent(f->font) * f->size_px / 1000.0f;
    float baseline_px = pos.y + ascent_px;

    HPDF_Page_SetRGBFill(m_page, color.red / 255.0f, color.green / 255.0f, color.blue / 255.0f);
    HPDF_Page_BeginText(m_page);
    HPDF_Page_SetFontAndSize(m_page, f->font, to_pt(f->size_px));
    HPDF_Page_TextOut(m_page, pdf_x(pos.x), pdf_y_top(baseline_px), text);
    HPDF_Page_EndText(m_page);
}

pixel_t pdf_container::pt_to_px(float pt) const { return pt * PX_PER_PT; }
pixel_t pdf_container::get_default_font_size() const { return 16; }
const char* pdf_container::get_default_font_name() const { return "sans-serif"; }

void pdf_container::draw_solid_fill(uint_ptr, const background_layer& layer, const web_color& color) {
    if (color.alpha == 0) {
        return;
    }
    const position& box = layer.border_box;
    HPDF_Page_SetRGBFill(m_page, color.red / 255.0f, color.green / 255.0f, color.blue / 255.0f);
    HPDF_Page_Rectangle(m_page, pdf_x(box.x), pdf_y_top(box.bottom()), to_pt(box.width), to_pt(box.height));
    HPDF_Page_Fill(m_page);
}

void pdf_container::draw_borders(uint_ptr, const borders& borders, const position& draw_pos, bool) {
    auto edge = [&](const border& b, float x, float y, float w, float h) {
        if (b.width <= 0 || b.style == border_style_none) {
            return;
        }
        HPDF_Page_SetRGBFill(m_page, b.color.red / 255.0f, b.color.green / 255.0f, b.color.blue / 255.0f);
        HPDF_Page_Rectangle(m_page, pdf_x(x), pdf_y_top(y + h), to_pt(w), to_pt(h));
        HPDF_Page_Fill(m_page);
    };

    edge(borders.top, draw_pos.x, draw_pos.y, draw_pos.width, borders.top.width);
    edge(borders.bottom, draw_pos.x, draw_pos.bottom() - borders.bottom.width, draw_pos.width, borders.bottom.width);
    edge(borders.left, draw_pos.x, draw_pos.y, borders.left.width, draw_pos.height);
    edge(borders.right, draw_pos.right() - borders.right.width, draw_pos.y, borders.right.width, draw_pos.height);
}

void pdf_container::draw_list_marker(uint_ptr, const list_marker& marker) {
    const position& pos = marker.pos;
    HPDF_Page_SetRGBFill(m_page, marker.color.red / 255.0f, marker.color.green / 255.0f, marker.color.blue / 255.0f);

    if (marker.marker_type == list_style_type_square) {
        HPDF_Page_Rectangle(m_page, pdf_x(pos.x), pdf_y_top(pos.bottom()), to_pt(pos.width), to_pt(pos.height));
        HPDF_Page_Fill(m_page);
        return;
    }

    float cx = pdf_x(pos.x + pos.width / 2.0f);
    float cy = pdf_y_top(pos.y + pos.height / 2.0f);
    float r = to_pt(pos.width / 2.0f);
    HPDF_Page_Circle(m_page, cx, cy, r);
    if (marker.marker_type == list_style_type_circle) {
        HPDF_Page_SetLineWidth(m_page, to_pt(1.0f));
        HPDF_Page_SetRGBStroke(m_page, marker.color.red / 255.0f, marker.color.green / 255.0f, marker.color.blue / 255.0f);
        HPDF_Page_Stroke(m_page);
    } else {
        HPDF_Page_Fill(m_page);
    }
}

HPDF_Image pdf_container::load_data_uri_image(const std::string& src) {
    auto it = m_image_cache.find(src);
    if (it != m_image_cache.end()) {
        return it->second;
    }

    HPDF_Image image = nullptr;
    if (src.compare(0, 5, "data:") == 0) {
        size_t comma = src.find(',');
        size_t b64 = src.find(";base64");
        if (comma != std::string::npos && b64 != std::string::npos && b64 < comma) {
            std::string meta = src.substr(5, comma - 5);
            if (meta.find("image/jpeg") != std::string::npos || meta.find("image/jpg") != std::string::npos) {
                std::vector<unsigned char> bytes = base64_decode(src.substr(comma + 1));
                if (!bytes.empty()) {
                    image = HPDF_LoadJpegImageFromMem(m_pdf, (const HPDF_BYTE*)bytes.data(), (HPDF_UINT)bytes.size());
                }
            }
        }
    }

    m_image_cache[src] = image;
    return image;
}

void pdf_container::load_image(const char* src, const char*, bool) {
    if (src != nullptr) {
        load_data_uri_image(src);
    }
}

void pdf_container::get_image_size(const char* src, const char*, size& sz) {
    sz.width = 0;
    sz.height = 0;
    if (src == nullptr) {
        return;
    }
    HPDF_Image image = load_data_uri_image(src);
    if (image != nullptr) {
        sz.width = (pixel_t)HPDF_Image_GetWidth(image);
        sz.height = (pixel_t)HPDF_Image_GetHeight(image);
    }
}

void pdf_container::draw_image(uint_ptr, const background_layer& layer, const std::string& url, const std::string&) {
    HPDF_Image image = load_data_uri_image(url);
    if (image == nullptr) {
        return;
    }
    const position& box = layer.border_box;
    HPDF_Page_DrawImage(m_page, image, pdf_x(box.x), pdf_y_top(box.bottom()), to_pt(box.width), to_pt(box.height));
}
void pdf_container::draw_linear_gradient(uint_ptr, const background_layer&, const background_layer::linear_gradient&) {}
void pdf_container::draw_radial_gradient(uint_ptr, const background_layer&, const background_layer::radial_gradient&) {}
void pdf_container::draw_conic_gradient(uint_ptr, const background_layer&, const background_layer::conic_gradient&) {}

element::ptr pdf_container::create_element(const char*, const string_map&, const document::ptr&) { return nullptr; }

void pdf_container::get_media_features(media_features& media) const {
    media.type = media_type_print;
    media.width = (int)content_width_px();
    media.height = (int)((m_page_h_pt - 2.0f * m_margin_pt) * PX_PER_PT);
    media.device_width = media.width;
    media.device_height = media.height;
    media.color = 8;
    media.resolution = 96;
}

void pdf_container::get_language(string&, string&) const {}
void pdf_container::link(const document::ptr&, const element::ptr&) {}
void pdf_container::transform_text(string&, text_transform) {}
void pdf_container::set_clip(const position&, const border_radiuses&) {}
void pdf_container::del_clip() {}
void pdf_container::set_caption(const char*) {}
void pdf_container::set_base_url(const char*) {}
void pdf_container::on_anchor_click(const char*, const element::ptr&) {}
void pdf_container::on_mouse_event(const element::ptr&, mouse_event) {}
void pdf_container::set_cursor(const char*) {}
void pdf_container::import_css(string&, const string&, string&) {}

void pdf_container::get_viewport(position& viewport) const {
    viewport.x = 0;
    viewport.y = 0;
    viewport.width = content_width_px();
    viewport.height = (m_page_h_pt - 2.0f * m_margin_pt) * PX_PER_PT;
}
