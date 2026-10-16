#include "fonts.h"

static const unsigned char sans_regular[] = {
#embed "assets/fonts/DejaVuLGCSans.ttf"
};
static const unsigned char sans_bold[] = {
#embed "assets/fonts/DejaVuLGCSans-Bold.ttf"
};
static const unsigned char sans_italic[] = {
#embed "assets/fonts/DejaVuLGCSans-Oblique.ttf"
};
static const unsigned char sans_bolditalic[] = {
#embed "assets/fonts/DejaVuLGCSans-BoldOblique.ttf"
};

static const unsigned char serif_regular[] = {
#embed "assets/fonts/DejaVuLGCSerif.ttf"
};
static const unsigned char serif_bold[] = {
#embed "assets/fonts/DejaVuLGCSerif-Bold.ttf"
};
static const unsigned char serif_italic[] = {
#embed "assets/fonts/DejaVuLGCSerif-Italic.ttf"
};
static const unsigned char serif_bolditalic[] = {
#embed "assets/fonts/DejaVuLGCSerif-BoldItalic.ttf"
};

static const unsigned char mono_regular[] = {
#embed "assets/fonts/DejaVuLGCSansMono.ttf"
};
static const unsigned char mono_bold[] = {
#embed "assets/fonts/DejaVuLGCSansMono-Bold.ttf"
};
static const unsigned char mono_italic[] = {
#embed "assets/fonts/DejaVuLGCSansMono-Oblique.ttf"
};
static const unsigned char mono_bolditalic[] = {
#embed "assets/fonts/DejaVuLGCSansMono-BoldOblique.ttf"
};

const unsigned char* embedded_font(int family, int bold, int italic, unsigned int* size_out) {
    const unsigned char* data = sans_regular;
    unsigned int size = sizeof(sans_regular);

    if (family == EMBEDDED_FONT_SERIF) {
        if (bold && italic) { data = serif_bolditalic; size = sizeof(serif_bolditalic); }
        else if (bold) { data = serif_bold; size = sizeof(serif_bold); }
        else if (italic) { data = serif_italic; size = sizeof(serif_italic); }
        else { data = serif_regular; size = sizeof(serif_regular); }
    } else if (family == EMBEDDED_FONT_MONO) {
        if (bold && italic) { data = mono_bolditalic; size = sizeof(mono_bolditalic); }
        else if (bold) { data = mono_bold; size = sizeof(mono_bold); }
        else if (italic) { data = mono_italic; size = sizeof(mono_italic); }
        else { data = mono_regular; size = sizeof(mono_regular); }
    } else {
        if (bold && italic) { data = sans_bolditalic; size = sizeof(sans_bolditalic); }
        else if (bold) { data = sans_bold; size = sizeof(sans_bold); }
        else if (italic) { data = sans_italic; size = sizeof(sans_italic); }
        else { data = sans_regular; size = sizeof(sans_regular); }
    }

    if (size_out != NULL) {
        *size_out = size;
    }
    return data;
}
