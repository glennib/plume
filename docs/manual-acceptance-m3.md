# M3 manual acceptance: `show()`

The checks below need a person looking at a screen.
Everything that can be asserted from bytes is automated in `make test_show_debug` (`scripts/check_show.py`);
this list covers what those checks cannot see: pixels in a terminal, windows on a desktop, and the feel of the shell.

Each check names the SQL to paste into the shell and what should be seen.
Where a check was also run by the M3 agent on Linux, the result is noted;
the agent drove the shell through a pseudo-terminal and read windows from `swaymsg -t get_tree`,
and captured ghostty with ImageMagick's `import -window`.

## Setup

In the repository, in the terminal under test (not inside tmux or screen):

```sh
make shell
```

Then paste the setup once:

```sql
CREATE TABLE weather AS
SELECT city, DATE '2024-01-01' + d::INTEGER AS day,
       round(-2 + 4 * c + 0.3 * d + 5 * sin(d / 5 + c), 1) AS temp
FROM (VALUES ('Oslo', 0), ('Bergen', 1), ('Tromsø', 2)) cities(city, c), range(60) r(d);
```

## Linux

1. **Terminal viewer above the table (ghostty, kitty, Konsole, WezTerm).**

   ```sql
   SELECT chart().caption('Temperature').draw_series(line_series(day, temp, key := city)).show() AS c FROM weather;
   ```

   A 640×480 chart with three coloured lines and a legend appears directly below the statement,
   and the one-row result table (`CHART(line, 3 series, 180 points, caption 'Temperature')`) below the image.
   Typing at the next prompt, Up for history and Ctrl-A/Ctrl-E redraw only the prompt line; the image stays.
   *Agent, ghostty 1.3.1 under Xwayland: seen as described.*

2. **One image per row, capped.**

   ```sql
   SELECT plume_set('max_show', 2);
   SELECT city, chart().caption(city).draw_series(line_series(day, temp)).show(width := 320, height := 200) AS c
   FROM weather GROUP BY city;
   SELECT plume_set('max_show', 10);
   ```

   Two small charts, each captioned with a city, stacked above a three-row table.

3. **Redirected output stays clean.**

   ```sql
   .once /tmp/plume-once.txt
   SELECT chart().show(width := 200, height := 150) AS c;
   ```

   The image still appears in the terminal; `/tmp/plume-once.txt` holds only the table
   (`grep -c $'\e' /tmp/plume-once.txt` prints 0).

4. **A window that does not block.**

   ```sql
   SELECT chart().caption('window').draw_series(line_series(day, temp, key := city)).show(viewer := 'window') AS c
   FROM weather;
   SELECT 42;
   ```

   A window titled `plume chart 1` opens with the chart; the result table and then `42` print at once,
   and the shell stays usable while the window is open.
   Resizing the window scales the chart, keeping its aspect ratio.
   Escape or the title-bar button closes it, and nothing is printed in the shell when it closes.
   *Agent, sway with Xwayland: the table and the next statement printed within 0.1 s; closing printed nothing.*

5. **`wait := true` blocks.**

   ```sql
   SELECT chart().caption('wait').show(viewer := 'window', wait := true) AS c;
   ```

   The prompt does not return while the window is open.
   Text typed meanwhile is echoed but not run until the window is closed;
   then the table prints and the typed statement runs.
   *Agent: the result printed 0.1 s after the close; a statement typed while blocked ran afterwards.*

6. **Ctrl-C does not close a waiting window.** Repeat 5 and press Ctrl-C.
   The window stays open and the prompt stays away; after closing the window the prompt returns without the result.
   *Agent: as described.*

7. **Exit with windows open.**

   ```sql
   SELECT chart().show(viewer := 'window') AS c FROM range(3);
   .quit
   ```

   Three windows open; `.quit` exits at once, the windows disappear, and `echo $?` prints 0.
   *Agent: exit 0 within 20 ms, no window left, nothing on stderr.*

8. **Auto without an image terminal picks the window.**
   In a terminal without inline images (xterm, alacritty, or inside tmux):

   ```sql
   SELECT chart().show() AS c;
   ```

   A window opens instead of an image.

9. **The browser.**

   ```sql
   SELECT chart().caption('browser').show(viewer := 'browser') AS c;
   ```

   The default browser opens a page with the chart scaled to fit, and the shell returns at once.
   The page is in `~/.cache/plume/` (or `$XDG_CACHE_HOME/plume/`).
   *Agent: not run with a real browser; `make test_show_debug` checks the page with a stand-in opener.*

10. **Environment defaults.**
    Quit, then start the shell with `PLUME_VIEWER=window PLUME_WAIT=true make shell` and run

    ```sql
    SELECT chart().show() AS c;
    ```

    A window opens and the shell blocks until it is closed.
    With `PLUME_VIEWER=kitty make shell` the same query fails with an error naming `PLUME_VIEWER='kitty'`,
    and works after `SELECT plume_set('viewer', 'auto');`.

11. **Native Wayland (optional build).**
    Build with `cargo build --lib --features plume-view/wayland && make footer_debug`, start the CLI directly
    (`make shell` would rebuild without the feature):
    `.duckdb/v2.0.0-alpha43385/linux_amd64/duckdb -unsigned -cmd "LOAD 'build/debug/plume.duckdb_extension'"`.
    Run check 4 in a Wayland session and close the window:
    about 20 lines of `warning: queue ... destroyed while proxies still attached` appear in the shell.
    That warning is why the default build uses X11 only.

## macOS (unverified)

Run checks 1–3 in iTerm2 (iTerm2 images) and in ghostty or kitty (kitty graphics).
Then:

- `SELECT chart().show(viewer := 'window') AS c;` fails with "on macOS a window that does not block needs an event loop
  on the main thread ... use wait := true".
- `SET threads = 1;` then `SELECT chart().show(viewer := 'window', wait := true) AS c;` opens a window and blocks
  until it is closed.
  Without `SET threads = 1` the call may run on a worker thread and fail with a message naming the main thread.
- With `TERM=dumb` (no image terminal), `SELECT chart().show() AS c;` opens the browser.

## Windows (unverified)

In Windows Terminal 1.22 or later (sixel):

- Checks 1–3: the image appears above the table (sixel through `CONOUT$`).
- Checks 4, 5 and 7: windows open from the shell without blocking, `wait := true` blocks, and `.quit` exits with
  windows open.
- Check 9: the browser opens through `ShellExecuteW`.
