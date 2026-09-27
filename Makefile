# Build, package and try out the duckers DuckDB extension.
#
#   make                 debug build plus footer: build/debug/duckers.duckdb_extension
#   make release         the same, optimised: build/release/duckers.duckdb_extension
#   make shell           the pinned DuckDB v2 preview CLI with the debug build loaded
#
# Variables worth overriding:
#   TARGET=<triple>      cross-compile with `cargo build --target`, e.g. aarch64-apple-darwin
#   PYTHON=<python>      the interpreter that runs scripts/append_footer.py

.DEFAULT_GOAL := debug

EXTENSION_NAME := duckers
# The C API version the extension targets; the loader refuses a footer version newer than its own.
C_API_VERSION := v2.0.0

# The pinned DuckDB v2 preview. The staged "latest alpha" builds live under versioned URLs and
# stay put; the nightly at artifacts.duckdb.org is overwritten daily, so it is not pinnable.
# The Python wheel in pyproject.toml is the same build.
DUCKDB_VERSION := v2.0.0-alpha43385
DUCKDB_SOURCE_ID := ca15f79c32
DUCKDB_RELEASE_URL := https://duckdb-staging.duckdb.org/$(DUCKDB_SOURCE_ID)/$(DUCKDB_VERSION)/duckdb/duckdb/github_release

PYTHON ?= $(shell command -v python3 2>/dev/null || command -v python 2>/dev/null)
CARGO ?= cargo

HOST_TRIPLE := $(shell rustc -vV | sed -n 's/^host: //p')
TARGET_TRIPLE := $(if $(TARGET),$(TARGET),$(HOST_TRIPLE))
CARGO_TARGET_FLAG := $(if $(TARGET),--target $(TARGET))

# cargo's target directory may be configured anywhere (CARGO_TARGET_DIR, build.target-dir), so ask
# cargo instead of assuming ./target.
CARGO_METADATA := $(CARGO) metadata --format-version 1 --no-deps
CARGO_TARGET_DIR_RAW := $(shell $(CARGO_METADATA) | jq -r .target_directory)
ifeq ($(OS),Windows_NT)
CARGO_TARGET_DIR_RESOLVED := $(shell cygpath -u '$(CARGO_TARGET_DIR_RAW)')
else
CARGO_TARGET_DIR_RESOLVED := $(CARGO_TARGET_DIR_RAW)
endif
CARGO_OUT_DIR := $(CARGO_TARGET_DIR_RESOLVED)$(if $(TARGET),/$(TARGET))

EXTENSION_VERSION := v$(shell $(CARGO_METADATA) | jq -r '.packages[] | select(.name == "duckers") | .version')

# The cdylib's file name: duckers.dll on Windows, libduckers.dylib on macOS, libduckers.so elsewhere.
ifneq ($(findstring windows,$(TARGET_TRIPLE)),)
LIBRARY := $(EXTENSION_NAME).dll
else ifneq ($(findstring apple,$(TARGET_TRIPLE)),)
LIBRARY := lib$(EXTENSION_NAME).dylib
else
LIBRARY := lib$(EXTENSION_NAME).so
endif

EXTENSION_FILE := $(EXTENSION_NAME).duckdb_extension
DEBUG_EXTENSION := build/debug/$(EXTENSION_FILE)
RELEASE_EXTENSION := build/release/$(EXTENSION_FILE)

# The host's DuckDB platform, for picking the preview CLI to download.
HOST_PLATFORM := $(strip \
  $(if $(findstring x86_64-unknown-linux-gnu,$(HOST_TRIPLE)),linux_amd64, \
  $(if $(findstring aarch64-unknown-linux-gnu,$(HOST_TRIPLE)),linux_arm64, \
  $(if $(findstring x86_64-apple-darwin,$(HOST_TRIPLE)),osx_amd64, \
  $(if $(findstring aarch64-apple-darwin,$(HOST_TRIPLE)),osx_arm64, \
  $(if $(findstring x86_64-pc-windows,$(HOST_TRIPLE)),windows_amd64, \
  $(if $(findstring aarch64-pc-windows,$(HOST_TRIPLE)),windows_arm64, \
  unknown)))))))
CLI_ARCHIVE := duckdb-cli-$(subst _,-,$(HOST_PLATFORM)).tar.gz
CLI_DIR := .duckdb/$(DUCKDB_VERSION)/$(HOST_PLATFORM)
DUCKDB := $(CLI_DIR)/duckdb$(if $(findstring windows,$(HOST_PLATFORM)),.exe)
SHA256SUM := $(shell command -v sha256sum >/dev/null 2>&1 && echo sha256sum || echo 'shasum -a 256')

.PHONY: all debug release build_debug build_release footer_debug footer_release duckdb shell \
        shell_release bindings clean

all: debug

debug: build_debug footer_debug
release: build_release footer_release

build_debug:
	$(CARGO) build --lib $(CARGO_TARGET_FLAG)

build_release:
	$(CARGO) build --lib --release $(CARGO_TARGET_FLAG)

# The footer step: copy the library and append the 512-byte metadata footer the loader requires.
footer_debug:
	$(PYTHON) scripts/append_footer.py "$(CARGO_OUT_DIR)/debug/$(LIBRARY)" $(DEBUG_EXTENSION) \
		--target $(TARGET_TRIPLE) --c-api-version $(C_API_VERSION) --extension-version $(EXTENSION_VERSION)

footer_release:
	$(PYTHON) scripts/append_footer.py "$(CARGO_OUT_DIR)/release/$(LIBRARY)" $(RELEASE_EXTENSION) \
		--target $(TARGET_TRIPLE) --c-api-version $(C_API_VERSION) --extension-version $(EXTENSION_VERSION)

# The pinned preview CLI, downloaded once into .duckdb/ and checked against scripts/duckdb-cli.sha256.
duckdb: $(DUCKDB)

$(DUCKDB):
	@test "$(HOST_PLATFORM)" != unknown || { echo "no DuckDB preview CLI for host $(HOST_TRIPLE)"; exit 1; }
	mkdir -p $(CLI_DIR)
	curl -fsSL -o $(CLI_DIR)/$(CLI_ARCHIVE) $(DUCKDB_RELEASE_URL)/$(CLI_ARCHIVE)
	grep ' $(CLI_ARCHIVE)$$' scripts/duckdb-cli.sha256 | (cd $(CLI_DIR) && $(SHA256SUM) -c -)
	tar -xzf $(CLI_DIR)/$(CLI_ARCHIVE) -C $(CLI_DIR)
	rm $(CLI_DIR)/$(CLI_ARCHIVE)
	touch $@

shell: debug $(DUCKDB)
	$(DUCKDB) -unsigned -cmd "LOAD '$(DEBUG_EXTENSION)'"

shell_release: release $(DUCKDB)
	$(DUCKDB) -unsigned -cmd "LOAD '$(RELEASE_EXTENSION)'"

# Regenerates crates/duckers-sys/src/bindings.rs from the vendored headers (needs libclang).
bindings:
	$(CARGO) build --lib --features duckers-sys/generate-bindings
	$(CARGO) fmt

clean:
	$(CARGO) clean
	rm -rf build
