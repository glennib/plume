# Build, package and try out the plume DuckDB extension.
#
#   make                 debug build plus footer: build/debug/plume.duckdb_extension
#   make release         the same, optimised: build/release/plume.duckdb_extension
#   make test            Rust unit tests, sqllogictests and SVG snapshots (Python wheel), CLI
#                        tests and show() checks on the debug build
#   make test_release    the same on the release build
#   make shell           the pinned DuckDB v2 preview CLI with the debug build loaded
#   make shim            the VISUALIZE grammar shim: build/shim/plume_visualize.duckdb_extension,
#                        built inside DuckDB's own build against the pinned source (slow the first time)
#   make test_shim       the shim's tests on the debug build; make shell_shim loads both extensions
#
# Tools: cargo, jq, curl, uv (for the test venv) and a Python 3 for the footer script; the shim
# needs cmake, git and a C++17 compiler on top.
#
# Variables worth overriding:
#   TARGET=<triple>      cross-compile with `cargo build --target`, e.g. aarch64-apple-darwin
#   PYTHON=<python>      the interpreter that runs scripts/append_footer.py

.DEFAULT_GOAL := debug

EXTENSION_NAME := plume
# The C API version the extension targets; the loader refuses a footer version newer than its own.
C_API_VERSION := v2.0.0

# The pinned DuckDB v2 preview. The staged "latest alpha" builds live under versioned URLs and
# stay put; the nightly at artifacts.duckdb.org is overwritten daily, so it is not pinnable.
# The Python wheel in pyproject.toml is the same build.
DUCKDB_VERSION := v2.0.0-alpha43385
DUCKDB_SOURCE_ID := ca15f79c32
# The commit the pinned build was made from, in full: DUCKDB_SOURCE_ID is its prefix, and the shim
# build fetches it by this hash.
DUCKDB_SOURCE_COMMIT := ca15f79c32c52c4b51df81f11cf915311db950d1
DUCKDB_RELEASE_URL := https://duckdb-staging.duckdb.org/$(DUCKDB_SOURCE_ID)/$(DUCKDB_VERSION)/duckdb/duckdb/github_release

PYTHON ?= $(shell command -v python3 2>/dev/null || command -v python 2>/dev/null)
CARGO ?= cargo
UV ?= uv

HOST_TRIPLE := $(shell rustc -vV | sed -n 's/^host: //p')
TARGET_TRIPLE := $(if $(TARGET),$(TARGET),$(HOST_TRIPLE))
CARGO_TARGET_FLAG := $(if $(TARGET),--target $(TARGET))

# cargo's target directory may be configured anywhere (CARGO_TARGET_DIR, build.target-dir), so ask
# cargo instead of assuming ./target.
CARGO_METADATA := $(CARGO) metadata --format-version 1 --no-deps
CARGO_TARGET_DIR_RAW := $(shell $(CARGO_METADATA) | jq -r .target_directory | tr -d '\r')
ifeq ($(OS),Windows_NT)
CARGO_TARGET_DIR_RESOLVED := $(shell cygpath -u '$(CARGO_TARGET_DIR_RAW)')
else
CARGO_TARGET_DIR_RESOLVED := $(CARGO_TARGET_DIR_RAW)
endif
CARGO_OUT_DIR := $(CARGO_TARGET_DIR_RESOLVED)$(if $(TARGET),/$(TARGET))

EXTENSION_VERSION := v$(shell $(CARGO_METADATA) | jq -r '.packages[] | select(.name == "plume") | .version' | tr -d '\r')

# The cdylib's file name: plume.dll on Windows, libplume.dylib on macOS, libplume.so elsewhere.
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
        shell_release shell_shim venv test test_debug test_release test_rust test_sql_debug \
        test_sql_release test_cli_debug test_cli_release test_show_debug test_show_release \
        test_svg_debug test_svg_release update_svg duckdb_source shim test_shim test_shim_debug \
        test_shim_release fmt lint bindings clean

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

# The debug build with the VISUALIZE shim loaded and its grammar active.
shell_shim: debug shim $(DUCKDB)
	$(DUCKDB) -unsigned -cmd "LOAD '$(DEBUG_EXTENSION)'" -cmd "LOAD '$(SHIM_EXTENSION)'" \
		-cmd "SET active_grammar_extensions = ['$(SHIM_NAME)']"

# The VISUALIZE grammar shim (M8): a C++ grammar extension for DuckDB's PEG parser, built inside
# DuckDB's own CMake build against the pinned source. A v2 C++ extension links DuckDB statically and
# loads only into the exact DuckDB version it was built from, so unlike the core it is rebuilt per
# DuckDB version. Needs cmake, git and a C++17 compiler; the DuckDB build takes a while the first
# time and is kept in build/shim/cmake.
SHIM_NAME := plume_visualize
DUCKDB_SOURCE_DIR := .duckdb/$(DUCKDB_VERSION)/src
DUCKDB_SOURCE_STAMP := $(DUCKDB_SOURCE_DIR)/.fetched
SHIM_CMAKE_DIR := build/shim/cmake
SHIM_EXTENSION := build/shim/$(SHIM_NAME).duckdb_extension
# Extra configure flags, e.g. SHIM_CMAKE_FLAGS='-G Ninja'; the build's parallelism.
SHIM_CMAKE_FLAGS ?=
SHIM_JOBS ?= $(shell nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)
CMAKE ?= cmake
GIT ?= git

# The pinned DuckDB source: one commit, fetched sparse and without unneeded blobs, with the
# directories the build reads (no tests).
duckdb_source: $(DUCKDB_SOURCE_STAMP)

$(DUCKDB_SOURCE_STAMP):
	rm -rf $(DUCKDB_SOURCE_DIR)
	mkdir -p $(DUCKDB_SOURCE_DIR)
	$(GIT) -C $(DUCKDB_SOURCE_DIR) init -q
	$(GIT) -C $(DUCKDB_SOURCE_DIR) remote add origin https://github.com/duckdb/duckdb.git
	$(GIT) -C $(DUCKDB_SOURCE_DIR) sparse-checkout set src third_party extension scripts tools
	$(GIT) -C $(DUCKDB_SOURCE_DIR) fetch --depth 1 --filter=blob:none -q origin $(DUCKDB_SOURCE_COMMIT)
	$(GIT) -C $(DUCKDB_SOURCE_DIR) checkout -q FETCH_HEAD
	touch $@

shim: $(SHIM_EXTENSION)

# Only the loadable extension target is built, not the shell or the tests; parquet and jemalloc
# are left out of the DuckDB library the shim links, since it uses neither. The EXTENSION profile
# is DuckDB's own for distributed extensions (x86-64-v2 rather than the CLI's v3 on amd64).
$(SHIM_EXTENSION): $(DUCKDB_SOURCE_STAMP) shim/CMakeLists.txt shim/extension_config.cmake $(wildcard shim/src/*)
	$(CMAKE) -S $(DUCKDB_SOURCE_DIR) -B $(SHIM_CMAKE_DIR) -DCMAKE_BUILD_TYPE=Release \
		-DEXTENSION_STATIC_BUILD=1 -DBUILD_SHELL=0 -DBUILD_UNITTESTS=0 -DBUILD_BENCHMARKS=0 \
		-DSKIP_EXTENSIONS=parquet -DENABLE_JEMALLOC=OFF -DDUCKDB_OPTIMIZATION_PROFILE=EXTENSION \
		-DOVERRIDE_GIT_DESCRIBE=$(DUCKDB_VERSION) \
		-DDUCKDB_EXTENSION_CONFIGS=$(abspath shim/extension_config.cmake) \
		-DPLUME_VERSION=$(EXTENSION_VERSION) $(SHIM_CMAKE_FLAGS)
	$(CMAKE) --build $(SHIM_CMAKE_DIR) --parallel $(SHIM_JOBS) --target $(SHIM_NAME)_loadable_extension
	cp $(SHIM_CMAKE_DIR)/extension/$(SHIM_NAME)/$(SHIM_NAME).duckdb_extension $@

# The test venv: the pinned DuckDB wheel and DuckDB's Python sqllogictest runner (pyproject.toml).
venv:
	$(UV) sync --locked

test: test_debug
test_debug: test_rust test_sql_debug test_svg_debug test_cli_debug test_show_debug
test_release: test_rust test_sql_release test_svg_release test_cli_release test_show_release

test_rust:
	$(CARGO) nextest run --no-tests=pass

# The tests write their files (COPY ... TO) into build/test-sql/.
SQLLOGICTEST = $(UV) run --locked python -m duckdb_sqllogictest

test_sql_debug: debug venv
	mkdir -p build/test-sql
	$(SQLLOGICTEST) --test-dir test/sql --external-extension $(DEBUG_EXTENSION)

test_sql_release: release venv
	mkdir -p build/test-sql
	$(SQLLOGICTEST) --test-dir test/sql --external-extension $(RELEASE_EXTENSION)

# SVG snapshots: each test/svg/<name>.sql renders a chart through the wheel, and the SVG must
# equal test/svg/<name>.svg. `make update_svg` rewrites the expected files for review.
CHECK_SVG = $(UV) run --locked python scripts/check_svg.py

test_svg_debug: debug venv
	$(CHECK_SVG) $(DEBUG_EXTENSION)

test_svg_release: release venv
	$(CHECK_SVG) $(RELEASE_EXTENSION)

update_svg: debug venv
	$(CHECK_SVG) $(DEBUG_EXTENSION) --update

# CLI tests: each test/cli/<name>.sql runs in the preview CLI, and its output must equal
# test/cli/<name>.out. They cover what the Python runner cannot see, such as how the shell renders
# custom-type values.
define run_cli_tests
	@mkdir -p build/test-cli
	@set -e; for sql in test/cli/*.sql; do \
		name=$$(basename $$sql .sql); \
		$(DUCKDB) -unsigned -bail -cmd "LOAD '$(1)'" -f $$sql > build/test-cli/$$name.out 2>&1 || { cat build/test-cli/$$name.out; exit 1; }; \
		diff -u --strip-trailing-cr test/cli/$$name.out build/test-cli/$$name.out; \
		echo "$$sql: ok"; \
	done
endef

test_cli_debug: debug $(DUCKDB)
	$(call run_cli_tests,$(DEBUG_EXTENSION))

test_cli_release: release $(DUCKDB)
	$(call run_cli_tests,$(RELEASE_EXTENSION))

# show() checks: the terminal viewer in a pseudo-terminal, the browser viewer with a stand-in
# opener, and test/show/*.test with no terminal and no display, each in a scrubbed environment so
# that nothing opens on the desktop. Linux only; elsewhere the script reports a skip.
CHECK_SHOW = $(UV) run --locked python scripts/check_show.py $(DUCKDB)

test_show_debug: debug venv $(DUCKDB)
	$(CHECK_SHOW) $(DEBUG_EXTENSION)

test_show_release: release venv $(DUCKDB)
	$(CHECK_SHOW) $(RELEASE_EXTENSION)

# Shim tests: test/shim/*.test through the wheel with the core registered and the shim loaded by
# path (PLUME_SHIM), and test/shim/cli.sql in the preview CLI with only the shim loaded, which
# loads the core from the copy next to it.
define run_shim_tests
	@mkdir -p build/test-sql build/test-cli
	cp $(1) build/shim/plume.duckdb_extension
	PLUME_SHIM=$(abspath $(SHIM_EXTENSION)) $(SQLLOGICTEST) --test-dir test/shim --external-extension $(1)
	@set -e; $(DUCKDB) -unsigned -bail -cmd "LOAD '$(SHIM_EXTENSION)'" -f test/shim/cli.sql > build/test-cli/shim.out 2>&1 \
		|| { cat build/test-cli/shim.out; exit 1; }; \
		diff -u --strip-trailing-cr test/shim/cli.out build/test-cli/shim.out; \
		echo "test/shim/cli.sql: ok"
endef

test_shim: test_shim_debug

test_shim_debug: shim debug venv $(DUCKDB)
	$(call run_shim_tests,$(DEBUG_EXTENSION))

test_shim_release: shim release venv $(DUCKDB)
	$(call run_shim_tests,$(RELEASE_EXTENSION))

fmt:
	$(CARGO) fmt
	$(UV) run --locked ruff format scripts

lint:
	$(CARGO) fmt --check
	$(CARGO) clippy --all-targets -- -D warnings
	$(UV) run --locked ruff check scripts
	$(UV) run --locked ruff format --check scripts

# Regenerates crates/plume-sys/src/bindings.rs from the vendored headers (needs libclang).
bindings:
	$(CARGO) build --lib --features plume-sys/generate-bindings
	$(CARGO) fmt

clean:
	$(CARGO) clean
	rm -rf build
