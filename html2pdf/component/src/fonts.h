#pragma once

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

enum embedded_font_family {
    EMBEDDED_FONT_SANS = 0,
    EMBEDDED_FONT_SERIF = 1,
    EMBEDDED_FONT_MONO = 2,
};

const unsigned char* embedded_font(int family, int bold, int italic, unsigned int* size_out);

#ifdef __cplusplus
}
#endif
