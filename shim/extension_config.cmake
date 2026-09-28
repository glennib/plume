# The extension config DuckDB's build reads for the VISUALIZE grammar shim: the Makefile passes it
# as DUCKDB_EXTENSION_CONFIGS, and DUCKERS_VERSION for the version the footer carries.
duckdb_extension_load(duckers_visualize
    SOURCE_DIR ${CMAKE_CURRENT_LIST_DIR}
    EXTENSION_VERSION "${DUCKERS_VERSION}"
    DONT_LINK
)
