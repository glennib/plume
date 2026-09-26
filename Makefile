.PHONY: clean clean_all shell uv_sync

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXTENSION_NAME=duckers

# Set to 1 to enable Unstable API (binaries will only work on TARGET_DUCKDB_VERSION, forwards compatibility will be broken)
# Note: duckdb-rs relies on unstable C API functionality, so this is required
USE_UNSTABLE_C_API=1

# Target DuckDB version
TARGET_DUCKDB_VERSION=v1.5.5

# extension-ci-tools ref, same as `ci_tools_version` in .github/workflows/MainDistributionPipeline.yml
CI_TOOLS_REF=v1.5-variegata

all: configure debug

# Include makefiles from DuckDB. CI checks out extension-ci-tools itself;
# locally, make clones it on first use and restarts with the included files.
include extension-ci-tools/makefiles/c_api_extensions/base.Makefile
include extension-ci-tools/makefiles/c_api_extensions/rust.Makefile

extension-ci-tools/makefiles/c_api_extensions/%.Makefile:
	test -d extension-ci-tools || git clone --depth 1 --branch $(CI_TOOLS_REF) https://github.com/duckdb/extension-ci-tools extension-ci-tools

# When uv is available, configure/venv is synced from pyproject.toml and
# uv.lock. Without uv (as in CI), base.Makefile's pip-based venv target
# builds it instead.
ifneq ($(shell command -v uv),)
VENV_TARGET=uv_sync
else
VENV_TARGET=venv
endif

uv_sync:
	UV_PROJECT_ENVIRONMENT=configure/venv uv sync

# rust.Makefile assumes cargo builds into ./target. Resolve the real target
# directory instead, so a CARGO_TARGET_DIR or build.target-dir setting works.
CARGO_TARGET_DIR_RESOLVED = $(shell cargo metadata --format-version 1 --no-deps | $(PYTHON_BIN) -c "import json,sys;print(json.load(sys.stdin)['target_directory'])")
override TARGET_PATH = $(CARGO_TARGET_DIR_RESOLVED)$(if $(TARGET),/$(TARGET))

configure: $(VENV_TARGET) platform extension_version

debug: build_extension_library_debug build_extension_with_metadata_debug
release: build_extension_library_release build_extension_with_metadata_release

test: test_debug
test_debug: test_extension_debug
test_release: test_extension_release

shell: debug
	duckdb -unsigned -cmd "LOAD '$(EXTENSION_BUILD_PATH)/debug/$(EXTENSION_FILENAME)'"

clean: clean_build clean_rust
clean_all: clean_configure clean
