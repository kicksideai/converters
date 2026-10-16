#!/usr/bin/env bash
set -euo pipefail

LIBHARU_TAG=v2.4.6
LITEHTML_COMMIT=d96c0209f15624c3d3c82aa81c36032e87602cd9

WASIP2CC=/opt/wasi-sdk/bin/wasm32-wasip2-clang
WASIP2CXX=/opt/wasi-sdk/bin/wasm32-wasip2-clang++
TOOLCHAIN=/opt/wasi-sdk/share/cmake/wasi-sdk-p2.cmake
NOEXC="-fno-exceptions"

cd /work/component

if [ ! -d third_party/libharu ]; then
    git clone --depth 1 --branch "${LIBHARU_TAG}" https://github.com/libharu/libharu.git third_party/libharu
    git -C third_party/libharu apply /work/component/patches/libharu-tounicode-cmap.patch
fi

if [ ! -d third_party/litehtml ]; then
    git clone --no-checkout https://github.com/litehtml/litehtml.git third_party/litehtml
    git -C third_party/litehtml archive "${LITEHTML_COMMIT}" | tar -x -C third_party/litehtml
    git -C third_party/litehtml apply /work/component/patches/litehtml-no-exceptions-prescan.patch
fi

echo "=== generate wit-bindgen C bindings ==="
rm -rf gen && mkdir -p gen
wit-bindgen c --world html2pdf --out-dir gen ./wit

echo "=== build libharu static (wasip2, no zlib/png) ==="
rm -rf build/libharu
cmake -S third_party/libharu -B build/libharu \
    -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
    -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_SHARED_LIBS=OFF \
    -DLIBHPDF_SHARED=OFF -DLIBHPDF_STATIC=ON -DLIBHPDF_EXAMPLES=OFF \
    -DCMAKE_DISABLE_FIND_PACKAGE_ZLIB=ON -DCMAKE_DISABLE_FIND_PACKAGE_PNG=ON >/dev/null
cmake --build build/libharu -j"$(nproc)" >/dev/null
HPDF_LIB=$(find build/libharu -name 'libhpdf*.a' | head -1)
HPDF_INC_SRC=third_party/libharu/include
HPDF_INC_GEN=$(dirname "$(find build/libharu -name hpdf_config.h | head -1)")

echo "=== build litehtml static (wasip2, -fno-exceptions, RTTI on) ==="
rm -rf build/litehtml
cmake -S third_party/litehtml -B build/litehtml \
    -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
    -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_SHARED_LIBS=OFF -DLITEHTML_BUILD_TESTING=OFF \
    -DCMAKE_CXX_FLAGS="${NOEXC}" -DCMAKE_C_FLAGS="${NOEXC}" >/dev/null
cmake --build build/litehtml -j"$(nproc)" >/dev/null
LITEHTML_A=$(find build/litehtml -name 'liblitehtml.a' | head -1)
GUMBO_A=$(find build/litehtml -name 'libgumbo.a' | head -1)

echo "=== compile + link component ==="
rm -rf build/obj && mkdir -p build/obj
"${WASIP2CC}" -Os -I gen -c gen/html2pdf.c -o build/obj/bindings.o
"${WASIP2CC}" -Os -std=c23 --embed-dir=. -c src/fonts.c -o build/obj/fonts.o
"${WASIP2CXX}" -Os ${NOEXC} -std=c++17 -Wl,--no-entry \
    -I gen -I src -I "${HPDF_INC_SRC}" -I "${HPDF_INC_GEN}" \
    -I third_party/litehtml/include -I third_party/litehtml/include/litehtml \
    src/render.cpp src/pdf_container.cpp \
    build/obj/bindings.o build/obj/fonts.o gen/html2pdf_component_type.o \
    "${LITEHTML_A}" ${GUMBO_A:-} "${HPDF_LIB}" \
    -Wl,-z,stack-size=8388608 \
    -o build/html2pdf.wasm

echo "=== validate + EH-opcode scan ==="
wasm-tools validate build/html2pdf.wasm
if wasm-tools print build/html2pdf.wasm \
    | grep -qE '\b(try|try_table|catch|catch_all|throw|throw_ref|rethrow|delegate)\b'; then
    echo "FATAL: EH opcodes present (will not run on wazero v1.10.1)" >&2
    exit 1
fi
echo "no EH opcodes"

mkdir -p /work/src/html2pdf/assets
cp build/html2pdf.wasm /work/src/html2pdf/assets/html2pdf.wasm
echo "staged: src/html2pdf/assets/html2pdf.wasm ($(wc -c < build/html2pdf.wasm) bytes)"
