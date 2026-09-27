#!/usr/bin/env python3
"""Turn a shared library into a loadable DuckDB extension by appending the metadata footer.

DuckDB refuses a library without the 512-byte footer, even with `allow_unsigned_extensions`.
The layout (DuckDB `src/main/extension/extension_load.cpp`) is eight 32-byte NUL-padded fields
followed by a 256-byte signature, all at the very end of the file. Read back to front, the fields
are: the magic value "4", the platform, the DuckDB or C API version, the extension version, the
ABI type, and three unused fields. With ABI `C_STRUCT` and a `v2.x.y` version, the loader calls
the entrypoint `<name>_init_c_api_v2`.

Standard library only, so it runs under any Python 3.
"""

import argparse
import shutil
import sys
from pathlib import Path

FIELD_LEN = 32
SIGNATURE_LEN = 256

# Rust target triples to DuckDB platform names (`PRAGMA platform`).
PLATFORMS = {
    "x86_64-unknown-linux-gnu": "linux_amd64",
    "aarch64-unknown-linux-gnu": "linux_arm64",
    "x86_64-unknown-linux-musl": "linux_amd64_musl",
    "aarch64-unknown-linux-musl": "linux_arm64_musl",
    "x86_64-apple-darwin": "osx_amd64",
    "aarch64-apple-darwin": "osx_arm64",
    "x86_64-pc-windows-msvc": "windows_amd64",
    "aarch64-pc-windows-msvc": "windows_arm64",
    "x86_64-pc-windows-gnu": "windows_amd64_mingw",
}


def field(value: str) -> bytes:
    encoded = value.encode("ascii")
    if len(encoded) > FIELD_LEN:
        raise SystemExit(f"footer field longer than {FIELD_LEN} bytes: {value!r}")
    return encoded + b"\0" * (FIELD_LEN - len(encoded))


def footer(platform: str, version: str, extension_version: str, abi: str) -> bytes:
    fields = [
        "",  # unused
        "",  # unused
        "",  # unused
        abi,
        extension_version,
        version,
        platform,
        "4",  # magic
    ]
    data = b"".join(field(f) for f in fields) + b"\0" * SIGNATURE_LEN
    assert len(data) == 512
    return data


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("library", type=Path, help="the shared library built by cargo")
    p.add_argument("output", type=Path, help="the .duckdb_extension file to write")
    where = p.add_mutually_exclusive_group(required=True)
    where.add_argument("--platform", help="DuckDB platform, e.g. linux_amd64")
    where.add_argument(
        "--target", help="Rust target triple to derive the platform from"
    )
    p.add_argument(
        "--c-api-version", default="v2.0.0", help="C API version (default v2.0.0)"
    )
    p.add_argument("--extension-version", required=True, help="e.g. v0.1.0")
    p.add_argument("--abi", default="C_STRUCT", help="ABI type (default C_STRUCT)")
    args = p.parse_args()

    platform = args.platform
    if platform is None:
        platform = PLATFORMS.get(args.target)
        if platform is None:
            print(f"no DuckDB platform known for target {args.target}", file=sys.stderr)
            return 1

    tmp = args.output.with_name(args.output.name + ".tmp")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(args.library, tmp)
    with tmp.open("ab") as f:
        f.write(footer(platform, args.c_api_version, args.extension_version, args.abi))
    tmp.replace(args.output)
    print(
        f"{args.output}: platform={platform} c_api={args.c_api_version} "
        f"version={args.extension_version} abi={args.abi}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
