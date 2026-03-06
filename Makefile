TARGET_X64  = x86_64-unknown-linux-gnu
TARGET_ARM  = aarch64-unknown-linux-gnu
LINKER_X64  = $(shell which x86_64-unknown-linux-gnu-gcc)
LINKER_ARM  = $(shell which aarch64-unknown-linux-gnu-gcc)
PKGS        = ruptela-listener galileosky-listener teltonika-listener queclink-listener
DIST        = dist
TS          = $(shell date +%Y%m%d-%H%M%S)

# ── Ubuntu x86_64 ─────────────────────────────────────────────────────────────

.PHONY: build-x64
build-x64:
	@test -n "$(LINKER_X64)" || (echo "ERROR: brew install x86_64-unknown-linux-gnu"; exit 1)
	CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$(LINKER_X64) \
	cargo build --workspace --release --target $(TARGET_X64)
	@mkdir -p $(DIST)
	@for pkg in $(PKGS); do \
		cargo deb -p $$pkg --target $(TARGET_X64) --no-build --no-strip; \
		deb=$$(ls target/$(TARGET_X64)/debian/$${pkg}_*.deb | tail -1); \
		base=$$(basename $$deb .deb); \
		cp $$deb $(DIST)/$${base}-$(TS).deb; \
	done
	@echo "Paquetes x86_64 → $(DIST)/"

# ── Ubuntu ARM64 / Graviton ───────────────────────────────────────────────────

.PHONY: build-arm64
build-arm64:
	@test -n "$(LINKER_ARM)" || (echo "ERROR: brew install aarch64-unknown-linux-gnu"; exit 1)
	CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$(LINKER_ARM) \
	cargo build --workspace --release --target $(TARGET_ARM)
	@mkdir -p $(DIST)
	@for pkg in $(PKGS); do \
		cargo deb -p $$pkg --target $(TARGET_ARM) --no-build --no-strip; \
		deb=$$(ls target/$(TARGET_ARM)/debian/$${pkg}_*.deb | tail -1); \
		base=$$(basename $$deb .deb); \
		cp $$deb $(DIST)/$${base}-$(TS).deb; \
	done
	@echo "Paquetes ARM64 → $(DIST)/"

# ── Ambas arquitecturas de una vez ───────────────────────────────────────────

.PHONY: build-all
build-all: build-x64 build-arm64
	@echo ""
	@ls $(DIST)/*.deb
	@echo ""
	@echo "Todos los paquetes en $(DIST)/"

# ── Tests ─────────────────────────────────────────────────────────────────────

.PHONY: test
test:
	cargo test --workspace

# ── Deploy (uso interno) ──────────────────────────────────────────────────────
# make deploy HOST=ubuntu@1.2.3.4          → x64
# make deploy HOST=ubuntu@1.2.3.4 ARM64=1  → arm64

.PHONY: deploy
deploy:
	@test -n "$(HOST)" || (echo "ERROR: define HOST=usuario@ip"; exit 1)
ifdef ARM64
	$(MAKE) build-arm64
	scp $(DIST)/*-arm64-*.deb $(HOST):~/
else
	$(MAKE) build-x64
	scp $(DIST)/*-amd64-*.deb $(HOST):~/
endif
	@echo "Paquetes desplegados en $(HOST):~/"

# ── Limpieza ──────────────────────────────────────────────────────────────────

.PHONY: clean
clean:
	cargo clean
	rm -rf $(DIST)
