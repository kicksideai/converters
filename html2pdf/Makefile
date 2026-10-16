.PHONY: help image wippy build-component install lint test check clean

RUNTIME_DIR ?= ../runtime
OS   := $(shell uname -s | tr '[:upper:]' '[:lower:]')
ARCH := $(shell uname -m | sed -e 's/x86_64/amd64/' -e 's/aarch64/arm64/' -e 's/arm64/arm64/')
WIPPY ?= wippy
FIXTURE_DIR ?= $(CURDIR)/.local/fixtures
WIPPY_BIN := $(RUNTIME_DIR)/dist/wippy-$(OS)-$(ARCH)

IMAGE    := html2pdf-wasi:33
WASM_DST := src/html2pdf/assets/html2pdf.wasm
INDEX    := src/html2pdf/_index.yaml

help:
	@echo "html2pdf make targets:"
	@echo "  image            build the wasi-sdk toolchain Docker image"
	@echo "  build-component  build the WASM component in Docker, stage it, inject its sha256"
	@echo "  wippy            build the runtime binary (from $(RUNTIME_DIR)) and point ./wippy at it"
	@echo "  install          $(WIPPY) install $(WIPPY_INSTALL_ARGS) (fetch module dependencies)"
	@echo "  lint             $(WIPPY) lint $(WIPPY_LINT_ARGS) --level error"
	@echo "  test             $(WIPPY) test $(WIPPY_TEST_ARGS)"
	@echo "  check            build-component + lint + test"
	@echo "  clean            remove build artifacts"

image:
	docker build -t $(IMAGE) component

build-component: image
	docker run --rm --user "$$(id -u):$$(id -g)" -e GIT_CONFIG_GLOBAL=/dev/null -e XDG_CONFIG_HOME=/work/.local/container-config -v "$(CURDIR)":/work $(IMAGE) bash component/build.sh
	@HASH=$$(shasum -a 256 $(WASM_DST) | cut -d' ' -f1); \
	  python3 -c "import re,sys; p='$(INDEX)'; s=open(p).read(); s=re.sub(r'hash: sha256:[0-9a-f]+', 'hash: sha256:'+'$$HASH', s); open(p,'w').write(s)"; \
	  echo "injected sha256:$$HASH into $(INDEX)"

wippy:
	@$(MAKE) -C $(RUNTIME_DIR) build-wippy
	@ln -sf $(WIPPY_BIN) ./wippy
	@./wippy version

install:
	$(WIPPY) install $(WIPPY_INSTALL_ARGS)

lint:
	$(WIPPY) lint $(WIPPY_LINT_ARGS) --level error

test:
	$(WIPPY) test $(WIPPY_TEST_ARGS)

check: build-component lint test

clean:
	rm -rf component/build component/gen component/third_party dist

.PHONY: lockrefresh
lockrefresh:
	$(WIPPY) update $(WIPPY_UPDATE_ARGS)
