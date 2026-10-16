FROM debian:bookworm-slim

ARG WASI_SDK_VERSION=33.0
ARG WIT_BINDGEN_VERSION=0.58.0
ARG WASM_TOOLS_VERSION=1.252.0

RUN apt-get update && apt-get install -y --no-install-recommends \
      curl ca-certificates xz-utils cmake make git python3 binaryen wabt \
    && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    arch="$(uname -m)"; \
    case "${arch}" in \
      x86_64) wasi_arch="x86_64" ;; \
      aarch64|arm64) wasi_arch="arm64" ;; \
      *) echo "unsupported build architecture: ${arch}" >&2; exit 1 ;; \
    esac; \
    pkg="wasi-sdk-${WASI_SDK_VERSION}-${wasi_arch}-linux"; \
    curl -fsSL "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-${WASI_SDK_VERSION%%.*}/${pkg}.tar.gz" \
      | tar xz -C /opt; \
    ln -s "/opt/${pkg}" /opt/wasi-sdk

RUN set -eux; \
    arch="$(uname -m)"; \
    case "${arch}" in \
      x86_64) tool_arch="x86_64" ;; \
      aarch64|arm64) tool_arch="aarch64" ;; \
      *) echo "unsupported build architecture: ${arch}" >&2; exit 1 ;; \
    esac; \
    pkg="wit-bindgen-${WIT_BINDGEN_VERSION}-${tool_arch}-linux"; \
    curl -fsSL "https://github.com/bytecodealliance/wit-bindgen/releases/download/v${WIT_BINDGEN_VERSION}/${pkg}.tar.gz" \
      | tar xz -C /opt; \
    ln -s "/opt/${pkg}/wit-bindgen" /usr/local/bin/wit-bindgen

RUN set -eux; \
    arch="$(uname -m)"; \
    case "${arch}" in \
      x86_64) tool_arch="x86_64" ;; \
      aarch64|arm64) tool_arch="aarch64" ;; \
      *) echo "unsupported build architecture: ${arch}" >&2; exit 1 ;; \
    esac; \
    pkg="wasm-tools-${WASM_TOOLS_VERSION}-${tool_arch}-linux"; \
    curl -fsSL "https://github.com/bytecodealliance/wasm-tools/releases/download/v${WASM_TOOLS_VERSION}/${pkg}.tar.gz" \
      | tar xz -C /opt; \
    ln -s "/opt/${pkg}/wasm-tools" /usr/local/bin/wasm-tools

ENV PATH="/opt/wasi-sdk/bin:${PATH}"
WORKDIR /work
