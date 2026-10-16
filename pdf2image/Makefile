.PHONY: help wippy build-component install lint test check clean

RUNTIME_DIR ?= ../runtime
OS   := $(shell uname -s | tr '[:upper:]' '[:lower:]')
ARCH := $(shell uname -m | sed -e 's/x86_64/amd64/' -e 's/aarch64/arm64/' -e 's/arm64/arm64/')
WIPPY ?= wippy
FIXTURE_DIR ?= $(CURDIR)/.local/fixtures
WIPPY_BIN := $(RUNTIME_DIR)/dist/wippy-$(OS)-$(ARCH)

WASM_SRC := component/target/wasm32-wasip2/release/pdf2image.wasm
WASM_DST := src/pdf2image/assets/pdf2image.wasm
INDEX    := src/pdf2image/_index.yaml

help:
	@echo "pdf2image make targets:"
	@echo "  wippy            build the runtime binary (from $(RUNTIME_DIR)) and point ./wippy at it"
	@echo "  build-component  build the Rust WASM component, stage it, inject its sha256"
	@echo "  fixtures         regenerate test fixtures (src/pdf2image/fixtures.lua)"
	@echo "  install          $(WIPPY) install $(WIPPY_INSTALL_ARGS) (fetch module dependencies)"
	@echo "  lint             $(WIPPY) lint $(WIPPY_LINT_ARGS) --level error"
	@echo "  test             $(WIPPY) test $(WIPPY_TEST_ARGS)"
	@echo "  check            build-component + lint + test"
	@echo "  clean            remove build artifacts"

wippy:
	@$(MAKE) -C $(RUNTIME_DIR) build-wippy
	@ln -sf $(WIPPY_BIN) ./wippy
	@./wippy version

build-component:
	rustup target add wasm32-wasip2
	cd component && cargo build --release --target wasm32-wasip2
	@mkdir -p $(dir $(WASM_DST))
	cp $(WASM_SRC) $(WASM_DST)
	@HASH=$$(shasum -a 256 $(WASM_DST) | cut -d' ' -f1); \
	  python3 -c "import re,sys; p='$(INDEX)'; s=open(p).read(); s=re.sub(r'hash: sha256:[0-9a-f]+', 'hash: sha256:'+'$$HASH', s); open(p,'w').write(s)"; \
	  echo "staged $(WASM_DST) (sha256:$$HASH)"

install:
	$(WIPPY) install $(WIPPY_INSTALL_ARGS)

lint:
	$(WIPPY) lint $(WIPPY_LINT_ARGS) --level error

test:
	$(WIPPY) test $(WIPPY_TEST_ARGS)

check: build-component lint test

clean:
	rm -rf component/target dist

.PHONY: lockrefresh
lockrefresh:
	$(WIPPY) update $(WIPPY_UPDATE_ARGS)
