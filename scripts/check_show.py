"""show() checks that need a controlled terminal and environment, run against the preview CLI.

- The terminal viewer, in a pseudo-terminal with TERM=xterm-kitty: the kitty graphics sequences
  reach the terminal before the result table, one per shown row up to max_show, and decode to PNGs
  of the requested size. With stdout redirected to a file, the images still reach the terminal
  and the file holds only the table.
- Without a controlling terminal: the terminal viewer fails with the reason, and nothing is
  written to stdout.
- The browser viewer, with a stand-in `xdg-open` on PATH that records what it was asked to open,
  so no real browser starts: the page embeds the chart, and max_show caps the pages.
- test/show/*.test, sqllogictests run with no terminal and no display.

Every process gets a scrubbed environment and a temporary HOME, so the user's terminal, display,
DUCKERS_* variables and ~/.duckdbrc play no part and no window or browser can open.
The checks are Linux-only (the pty and error texts are Linux's); elsewhere the script says so and
exits 0.

Usage: check_show.py DUCKDB_CLI EXTENSION
"""

import argparse
import base64
import os
import re
import select
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

TIMEOUT = 60
TABLE_TOP = "┌".encode()
KITTY = re.compile(rb"\x1b_G([^;\x1b]*);([^\x1b]*)\x1b\\")


class Failure(Exception):
    pass


def check(condition: bool, message: str) -> None:
    if not condition:
        raise Failure(message)


def base_env(home: Path, **extra: str) -> dict[str, str]:
    env = {"HOME": str(home), "PATH": "/usr/bin:/bin", "LC_ALL": "C.UTF-8"}
    env.update(extra)
    return env


def cli(duckdb: Path, extension: Path, sql: str) -> list[str]:
    return [str(duckdb), "-unsigned", "-cmd", f"LOAD '{extension}'", "-c", sql]


def run_in_pty(
    argv: list[str], env: dict[str, str], stdout: Path | None = None
) -> tuple[bytes, int]:
    """Runs argv with a pseudo-terminal as its controlling terminal, stdin, stdout and stderr,
    or with stdout sent to a file. Returns everything written to the terminal and the exit code."""
    import pty

    pid, fd = pty.fork()
    if pid == 0:
        try:
            if stdout is not None:
                out = os.open(stdout, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
                os.dup2(out, 1)
            os.execve(argv[0], argv, env)
        finally:
            os._exit(127)
    chunks = []
    deadline = time.monotonic() + TIMEOUT
    while True:
        ready, _, _ = select.select([fd], [], [], max(0, deadline - time.monotonic()))
        if not ready:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
            raise Failure(f"timed out after {TIMEOUT}s: {argv}")
        try:
            data = os.read(fd, 65536)
        except OSError:  # EIO: the terminal's last writer has gone
            break
        if not data:
            break
        chunks.append(data)
    os.close(fd)
    _, status = os.waitpid(pid, 0)
    return b"".join(chunks), os.waitstatus_to_exitcode(status)


def run_without_tty(
    argv: list[str], env: dict[str, str]
) -> subprocess.CompletedProcess[bytes]:
    """Runs argv in a new session, so it has no controlling terminal."""
    return subprocess.run(
        argv,
        env=env,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        start_new_session=True,
        timeout=TIMEOUT,
        check=False,
    )


def png_size(png: bytes) -> tuple[int, int]:
    check(png[:8] == b"\x89PNG\r\n\x1a\n", f"not a PNG: {png[:16]!r}")
    return int.from_bytes(png[16:20], "big"), int.from_bytes(png[20:24], "big")


def kitty_images(stream: bytes) -> list[tuple[int, bytes]]:
    """The images in a byte stream as (offset of the first chunk, PNG bytes)."""
    images: list[tuple[int, bytes]] = []
    payload = b""
    start = None
    for m in KITTY.finditer(stream):
        params = dict(p.split(b"=", 1) for p in m.group(1).split(b",") if p)
        if start is None:
            check(params.get(b"a") == b"T", f"image without a=T: {m.group(1)!r}")
            check(params.get(b"f") == b"100", "payload is not PNG (f=100)")
            check(params.get(b"q") == b"2", "the terminal is asked to reply")
            start = m.start()
        payload += m.group(2)
        if params.get(b"m", b"0") == b"0":
            images.append((start, base64.b64decode(payload)))
            payload, start = b"", None
    check(start is None, "the last image is incomplete")
    return images


def terminal_env(home: Path) -> dict[str, str]:
    return base_env(home, TERM="xterm-kitty")


def check_terminal(duckdb: Path, ext: Path, tmp: Path) -> None:
    """Images reach the terminal before the table, at the requested size; wait is moot."""
    sql = (
        "SELECT i, chart().caption('row ' || i).show(width := 320, height := 200, "
        "wait := true) AS c FROM range(3) r(i)"
    )
    out, code = run_in_pty(cli(duckdb, ext, sql), terminal_env(tmp))
    check(code == 0, f"exit code {code}: {out[-500:]!r}")
    images = kitty_images(out)
    check(len(images) == 3, f"expected 3 images, got {len(images)}")
    table = out.find(TABLE_TOP)
    check(table > 0, "no result table")
    for offset, png in images:
        check(offset < table, "an image comes after the start of the table")
        check(png_size(png) == (320, 200), f"image size {png_size(png)}")
    check(out.count(b"CHART(") == 3, "the table does not list three charts")


def check_max_show(duckdb: Path, ext: Path, tmp: Path) -> None:
    """max_show caps each show() call; a prepared statement keeps its count across EXECUTEs."""
    sql = (
        "SELECT duckers_set('max_show', 2);"
        "SELECT i, chart().show(width := 64, height := 48) AS c FROM range(5) r(i);"
        "PREPARE p AS SELECT chart().show(width := 64, height := 48) FROM range(3);"
        "EXECUTE p; EXECUTE p;"
        "SELECT chart().show(width := 64, height := 48) AS a, "
        "chart().show(width := 64, height := 48) AS b FROM range(3);"
    )
    out, code = run_in_pty(cli(duckdb, ext, sql), terminal_env(tmp))
    check(code == 0, f"exit code {code}: {out[-500:]!r}")
    images = kitty_images(out)
    # 2 of 5, 2 for both EXECUTEs together, then 2 for each of the two calls.
    check(len(images) == 2 + 2 + 4, f"expected 8 images, got {len(images)}")
    check(out.count(b"CHART(") == 5 + 3 + 3 + 6, "rows past the cap are missing")


def check_stdout_redirected(duckdb: Path, ext: Path, tmp: Path) -> None:
    """With stdout in a file, the image still reaches the terminal and the file stays clean."""
    stdout = tmp / "stdout.txt"
    sql = "SELECT chart().show(width := 64, height := 48) AS c"
    out, code = run_in_pty(cli(duckdb, ext, sql), terminal_env(tmp), stdout=stdout)
    check(code == 0, f"exit code {code}: {out[-500:]!r}")
    check(len(kitty_images(out)) == 1, "the image did not reach the terminal")
    table = stdout.read_bytes()
    check(TABLE_TOP in table and b"CHART(" in table, f"no table in stdout: {table!r}")
    check(b"\x1b" not in table, "stdout holds escape sequences")


def check_no_tty(duckdb: Path, ext: Path, tmp: Path) -> None:
    """Without a controlling terminal the terminal viewer fails, and stdout stays clean."""
    sql = "SELECT chart().show(viewer := 'terminal') AS c"
    r = run_without_tty(cli(duckdb, ext, sql), terminal_env(tmp))
    output = r.stdout + r.stderr
    check(r.returncode != 0, "the query succeeded without a terminal")
    check(
        b"viewer 'terminal' is unavailable: cannot open /dev/tty" in output,
        f"unexpected error: {output!r}",
    )
    check(b"\x1b" not in output, "escape sequences were written to stdout or stderr")


def check_browser(duckdb: Path, ext: Path, tmp: Path) -> None:
    """DUCKERS_VIEWER picks the browser; the stand-in opener receives one page per shown row."""
    bin_dir = tmp / "bin"
    bin_dir.mkdir()
    log = tmp / "opened.txt"
    opener = bin_dir / "xdg-open"
    opener.write_text(f'#!/bin/sh\nprintf "%s\\n" "$1" >> "{log}"\n')
    opener.chmod(0o755)
    env = base_env(
        tmp,
        PATH=f"{bin_dir}:/usr/bin:/bin",
        XDG_CACHE_HOME=str(tmp / "cache"),
        # The browser viewer only checks that a graphical session is named; the window viewer,
        # which would connect to it, is never tried with DUCKERS_VIEWER=browser.
        DISPLAY="duckers-test-no-such-display:0",
        DUCKERS_VIEWER="browser",
    )
    sql = "SELECT duckers_set('max_show', 2); SELECT chart().show() AS c FROM range(3)"
    r = run_without_tty(cli(duckdb, ext, sql), env)
    check(r.returncode == 0, f"exit code {r.returncode}: {r.stderr!r}")
    check(b"\x1b" not in r.stdout, "stdout holds escape sequences")
    # The opener is not waited for.
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        lines = log.read_text().splitlines() if log.exists() else []
        if len(lines) >= 2:
            break
        time.sleep(0.05)
    time.sleep(0.2)
    lines = log.read_text().splitlines() if log.exists() else []
    check(len(lines) == 2, f"expected 2 pages opened, got {lines}")
    for line in lines:
        page = Path(line)
        check(
            page.parent == tmp / "cache" / "duckers", f"page outside the cache: {page}"
        )
        html = page.read_text()
        m = re.search(r"data:image/png;base64,([A-Za-z0-9+/=]+)", html)
        check(m is not None, "the page embeds no PNG")
        check(png_size(base64.b64decode(m.group(1))) == (640, 480), "PNG size")


def check_headless_sql(ext: Path, tmp: Path) -> None:
    """test/show/*.test with no terminal and no display, and invalid or set DUCKERS_* variables."""
    env = base_env(
        tmp,
        TERM="xterm-kitty",
        DUCKERS_VIEWER="kitty",
        DUCKERS_WAIT="yes",
        DUCKERS_TEST_HEADLESS="1",
    )
    argv = [sys.executable, "-m", "duckdb_sqllogictest", "--test-dir", "test/show"]
    argv += ["--external-extension", str(ext)]
    r = run_without_tty(argv, env)
    sys.stdout.write(r.stdout.decode(errors="replace"))
    sys.stdout.write(r.stderr.decode(errors="replace"))
    check(r.returncode == 0, "sqllogictests failed")
    check(b"SUCCESS" in r.stdout and b"SKIPPED" not in r.stdout, "tests were skipped")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("duckdb", type=Path)
    parser.add_argument("extension", type=Path)
    args = parser.parse_args()
    if not sys.platform.startswith("linux"):
        print(f"check_show: skipped on {sys.platform}, the checks are Linux-only")
        return 0
    duckdb, ext = args.duckdb.resolve(), args.extension.resolve()

    checks = [
        ("terminal: images before the table", check_terminal),
        ("terminal: max_show", check_max_show),
        ("terminal: stdout redirected", check_stdout_redirected),
        ("terminal: no controlling terminal", check_no_tty),
        ("browser: stand-in opener", check_browser),
    ]
    failed = 0
    for name, f in checks:
        with tempfile.TemporaryDirectory(prefix="duckers-show-") as tmp:
            try:
                f(duckdb, ext, Path(tmp))
                print(f"{name}: ok")
            except Failure as e:
                failed += 1
                print(f"{name}: FAILED: {e}")
    with tempfile.TemporaryDirectory(prefix="duckers-show-") as tmp:
        try:
            check_headless_sql(ext, Path(tmp))
            print("test/show: ok")
        except Failure as e:
            failed += 1
            print(f"test/show: FAILED: {e}")
    if failed:
        print(f"{failed} check(s) failed")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
