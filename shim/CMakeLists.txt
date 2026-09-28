# The VISUALIZE grammar shim, built inside DuckDB's own CMake build (see the Makefile's shim
# target): a v2 C++ loadable extension links DuckDB statically, so it is built per DuckDB version.
cmake_minimum_required(VERSION 3.15...3.29)

project(DuckersVisualizeExtension)

set(EXTENSION_NAME duckers_visualize)
set(EXTENSION_SOURCES src/duckers_visualize_extension.cpp)

build_loadable_extension(${EXTENSION_NAME} " " ${EXTENSION_SOURCES})
