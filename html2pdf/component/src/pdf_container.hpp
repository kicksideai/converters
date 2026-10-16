#pragma once

#include <map>
#include <string>

#include <litehtml.h>
#include "hpdf.h"

class pdf_container : public litehtml::document_container {
public:
    pdf_container(HPDF_Doc pdf, HPDF_Page page, float page_w_pt, float page_h_pt, float margin_pt);

    litehtml::uint_ptr create_font(const litehtml::font_description& descr, const litehtml::document* doc, litehtml::font_metrics* fm) override;
    void delete_font(litehtml::uint_ptr hFont) override;
    litehtml::pixel_t text_width(const char* text, litehtml::uint_ptr hFont) override;
    void draw_text(litehtml::uint_ptr hdc, const char* text, litehtml::uint_ptr hFont, litehtml::web_color color, const litehtml::position& pos) override;
    litehtml::pixel_t pt_to_px(float pt) const override;
    litehtml::pixel_t get_default_font_size() const override;
    const char* get_default_font_name() const override;
    void draw_list_marker(litehtml::uint_ptr hdc, const litehtml::list_marker& marker) override;
    void load_image(const char* src, const char* baseurl, bool redraw_on_ready) override;
    void get_image_size(const char* src, const char* baseurl, litehtml::size& sz) override;
    void draw_image(litehtml::uint_ptr hdc, const litehtml::background_layer& layer, const std::string& url, const std::string& base_url) override;
    void draw_solid_fill(litehtml::uint_ptr hdc, const litehtml::background_layer& layer, const litehtml::web_color& color) override;
    void draw_linear_gradient(litehtml::uint_ptr hdc, const litehtml::background_layer& layer, const litehtml::background_layer::linear_gradient& gradient) override;
    void draw_radial_gradient(litehtml::uint_ptr hdc, const litehtml::background_layer& layer, const litehtml::background_layer::radial_gradient& gradient) override;
    void draw_conic_gradient(litehtml::uint_ptr hdc, const litehtml::background_layer& layer, const litehtml::background_layer::conic_gradient& gradient) override;
    void draw_borders(litehtml::uint_ptr hdc, const litehtml::borders& borders, const litehtml::position& draw_pos, bool root) override;
    litehtml::element::ptr create_element(const char* tag_name, const litehtml::string_map& attributes, const litehtml::document::ptr& doc) override;
    void get_media_features(litehtml::media_features& media) const override;
    void get_language(litehtml::string& language, litehtml::string& culture) const override;
    void link(const litehtml::document::ptr& doc, const litehtml::element::ptr& el) override;
    void transform_text(litehtml::string& text, litehtml::text_transform tt) override;
    void set_clip(const litehtml::position& pos, const litehtml::border_radiuses& bdr_radius) override;
    void del_clip() override;
    void set_caption(const char* caption) override;
    void set_base_url(const char* base_url) override;
    void on_anchor_click(const char* url, const litehtml::element::ptr& el) override;
    void on_mouse_event(const litehtml::element::ptr& el, litehtml::mouse_event event) override;
    void set_cursor(const char* cursor) override;
    void import_css(litehtml::string& text, const litehtml::string& url, litehtml::string& baseurl) override;
    void get_viewport(litehtml::position& viewport) const override;

    float content_width_px() const;

private:
    HPDF_Doc m_pdf;
    HPDF_Page m_page;
    float m_page_w_pt;
    float m_page_h_pt;
    float m_margin_pt;

    float to_pt(float px) const;
    float pdf_x(float px) const;
    float pdf_y_top(float px) const;

    HPDF_Font load_font(int family, bool bold, bool italic);
    HPDF_Image load_data_uri_image(const std::string& src);

    std::map<int, HPDF_Font> m_font_cache;
    std::map<std::string, HPDF_Image> m_image_cache;
};
