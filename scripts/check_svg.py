"""SVG snapshot tests: render charts from SQL and compare them with checked-in SVG files.

Each `<dir>/<name>.sql` runs on a fresh connection with the extension loaded, after
`<dir>/_setup.sql`; the last statement must return one VARCHAR, the SVG, which must equal
`<dir>/<name>.svg`. `--update` writes the files instead of comparing.

The files are UTF-8 whatever the locale's encoding (cp1252 on Windows), and the SVG files are
written with LF line endings on every platform. Reading translates CRLF to LF, so a checkout with
CRLF line endings compares equal.

Usage: check_svg.py EXTENSION [--dir test/svg] [--update]
"""

import argparse
import difflib
import sys
from pathlib import Path

import duckdb


def render(extension: Path, setup: str, sql: str) -> str:
    con = duckdb.connect(config={"allow_unsigned_extensions": "true"})
    con.load_extension(str(extension))
    if setup:
        con.execute(setup)
    rows = con.execute(sql).fetchall()
    if len(rows) != 1 or len(rows[0]) != 1 or not isinstance(rows[0][0], str):
        raise ValueError(f"expected one VARCHAR, got {rows!r:.200}")
    return rows[0][0]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("extension", type=Path)
    parser.add_argument("--dir", type=Path, default=Path("test/svg"))
    parser.add_argument(
        "--update", action="store_true", help="write the expected files"
    )
    args = parser.parse_args()
    # A diff can hold characters that the console's code page lacks.
    sys.stdout.reconfigure(errors="backslashreplace")

    setup_file = args.dir / "_setup.sql"
    setup = setup_file.read_text(encoding="utf-8") if setup_file.exists() else ""
    failed = 0
    for sql_file in sorted(args.dir.glob("[!_]*.sql")):
        expected_file = sql_file.with_suffix(".svg")
        actual = render(args.extension, setup, sql_file.read_text(encoding="utf-8"))
        if args.update:
            expected_file.write_text(actual, encoding="utf-8", newline="")
            print(f"{expected_file}: written")
            continue
        expected = (
            expected_file.read_text(encoding="utf-8") if expected_file.exists() else ""
        )
        if actual == expected:
            print(f"{sql_file}: ok")
            continue
        failed += 1
        print(f"{sql_file}: differs from {expected_file}")
        diff = difflib.unified_diff(
            expected.splitlines(keepends=True),
            actual.splitlines(keepends=True),
            fromfile=str(expected_file),
            tofile="actual",
        )
        sys.stdout.writelines(list(diff)[:60])
    if failed:
        print(f"{failed} SVG snapshot(s) differ; `make update_svg` rewrites them")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
