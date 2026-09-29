# The extension config DuckDB's build reads for the VISUALIZE grammar shim: the Makefile passes it
# as DUCKDB_EXTENSION_CONFIGS, and PLUME_VERSION for the version the footer carries.
duckdb_extension_load(plume_visualize
    SOURCE_DIR ${CMAKE_CURRENT_LIST_DIR}
    EXTENSION_VERSION "${PLUME_VERSION}"
    DONT_LINK
)
