# Viewer spike, standalone part: the `plume-view` viewers

Research date: 2026-09-27.
Code: `crates/plume-view` (library and `examples/show.rs`), with `minifb` 0.28.0, `png` 0.18.1, `base64` 0.22.1,
`libc` 0.2.189 (Unix) and `windows-sys` 0.61.2 (Windows).
Hands-on: Arch Linux (kernel 7.2.6), Rust 1.98.1, sway 1.12
(Wayland, Xwayland 24.1.13 on `:1`),
libwayland 1.26.0, libX11 1.8.13, ghostty 1.3.1, Konsole 26.08.1, XTerm 411, Chromium 153,
libsixel 1.10.5 (`sixel2png`), ImageMagick 7.1.2.
The screen was locked for the whole session.
Terminals were therefore run under Xwayland and captured with ImageMagick's `import -window`,
which reads the window's own contents.
Wayland windows could only be confirmed through `swaymsg -t get_tree`, since `grim` showed the lock screen.
No macOS or Windows machine was available.

This covers the standalone half of the viewer spike: the viewers as a pure-Rust library, exercised from a test binary.
Calling them from a scalar function in the v2 preview CLI waits for the extension skeleton.

## Summary

- **The terminal viewer draws correctly in three real terminals (verified by capture).**
  - ghostty: kitty graphics, chosen automatically from the environment.
  - Konsole: kitty graphics (chosen automatically) and iTerm2 (forced).
  - xterm started as a VT340: sixel (forced).
  - In every case the text written after the image started on the line below it.
- **The bytes are right where no terminal could be run.**
  - Under `script(1)` the kitty sequence reached the pty while stdout (redirected to a file) stayed empty.
    The chunks reassembled to the original PNG.
  - The sixel output decodes with libsixel's `sixel2png` to the source image within the expected quantization error.
- **A non-blocking window opens in 10–20 ms and does not delay exit.**
  - Wayland and X11, one or three windows: the process exited 0 at the end of its hold time.
  - 20 runs each on Wayland and X11 opened four windows and exited 0–90 ms later: all exited 0, within 114–431 ms
    in total, and no window survived the process.
- **`wait = true` blocks until the window is closed**, on Wayland and X11, and returns 60–70 ms after the close
  request.
- **Closing a Wayland window prints libwayland's `queue … destroyed while proxies still attached` warning**
  (about 20 lines on stderr).
  X11 prints nothing.
  Process exit with Wayland windows open prints nothing, because the windows are never dropped.
- **The browser viewer writes a self-contained page and returns without waiting for the opener.**
  Headless Chromium rendered the page as expected.
  The real browser was not opened; a stand-in `xdg-open` that lingers for 3 s confirmed the call and
  that `show` returned in about 100 ms.
- **`auto` falls through with a reason per viewer.**
  Without a controlling terminal it picked the window.
  With neither a terminal nor a display it failed with one message naming all three reasons.
- **Terminal detection must rank the innermost terminal's variables first.**
  Konsole started from ghostty inherits `GHOSTTY_*` and `TERM_PROGRAM=ghostty`.
- **Unconfirmed:** the macOS main-thread check and the Windows `CONOUT$` path, including sixel in Windows Terminal.
  Both compile, the Windows build with `cargo clippy --target x86_64-pc-windows-gnu`
  and macOS without the window feature, but neither ran.
  They stay open for the manual acceptance of `show()`.

## The crate

`plume-view` has no DuckDB dependency.
The entry point for `show()`:

```rust
pub struct Image;                                 // PNG bytes, RGB8 pixels, or both; the missing one is made lazily
impl Image {
    pub fn from_png(png: Vec<u8>) -> Result<Image, ImageError>;
    pub fn from_rgb(width: u32, height: u32, rgb: Vec<u8>) -> Result<Image, ImageError>;
    pub fn from_png_and_rgb(png: Vec<u8>, width: u32, height: u32, rgb: Vec<u8>) -> Result<Image, ImageError>;
}
pub enum Viewer { Auto, Terminal, Window, Browser }   // FromStr with a message listing the names
pub struct ShowOptions { pub viewer: Viewer, pub wait: bool }
impl ShowOptions { pub fn from_settings() -> Result<ShowOptions, SettingsError>; }
pub enum Shown { Terminal(Protocol), Window { waited: bool }, Browser { path: PathBuf } }
pub enum ShowError { Unavailable { viewer, reason }, Failed { viewer, message }, NoViewer(Vec<ShowError>), Image(ImageError) }
pub fn show(image: &Image, options: &ShowOptions) -> Result<Shown, ShowError>;

pub mod settings {
    pub fn current() -> Result<Settings, SettingsError>;  // Settings { viewer, wait, max_show }
    pub fn set(key: &str, value: &str) -> Result<(), SettingsError>;
    pub fn get(key: &str) -> Result<String, SettingsError>;
}
```

- plotters' `BitMapBackend` draws RGB8, so `Image::from_rgb` lets `show()` skip PNG encoding when the window viewer is used.
  Kitty, iTerm2 and the browser need the PNG; sixel and the window need the pixels.
- The `window` feature (default on) gates `minifb`.
  Without it the dependency tree shrinks from 43 to 14 crates on Linux, and `Viewer::Window` reports itself unavailable.
- Settings start from `PLUME_VIEWER` and `PLUME_WAIT` on first use and hold `max_show`
  (default 10, `0` shows nothing).
  An invalid environment value is not replaced by the default.
  `current()` returns an error naming the variable until `set` gives that key a valid value; other keys stay usable.

## Terminal

### Detection

Only environment variables are read; the terminal is never queried.
The first rule that matches wins:

| # | Condition | Result |
|---|---|---|
| 1 | `TMUX` set, or `TERM` starting with `screen`/`tmux` | none |
| 2 | `TERM=xterm-kitty` or `xterm-ghostty` / `foot*` / `wezterm` | kitty / sixel / iTerm2 |
| 3 | `KONSOLE_VERSION` / `XTERM_VERSION` | kitty / none |
| 4 | `TERM_PROGRAM=ghostty` / `WezTerm` or `iTerm.app`, `LC_TERMINAL=iTerm2` | kitty / iTerm2 |
| 5 | `KITTY_WINDOW_ID`, `GHOSTTY_RESOURCES_DIR`, `GHOSTTY_BIN_DIR` / `WEZTERM_*` / `WT_SESSION` | kitty / iTerm2 / sixel |
| 6 | anything else | none, with `TERM` and `TERM_PROGRAM` in the reason |

After a protocol is chosen, `/dev/tty` must open (`CONOUT$` on Windows); otherwise the viewer is unavailable.

- **Inherited variables.**
  The first version checked `GHOSTTY_*` before `KONSOLE_VERSION`.
  Konsole started from this ghostty session got kitty graphics for that reason, not for its own.
  A terminal always sets `TERM`, Konsole and xterm set their `*_VERSION` but not `TERM_PROGRAM`, and the `GHOSTTY_*`,
  `KITTY_WINDOW_ID` and `WEZTERM_*` variables are the ones most often inherited.
  The rules are ordered accordingly, and a unit test covers terminal-in-terminal cases.
- **Konsole gets kitty graphics.**
  Konsole 26.08 drew the PNG sent with `a=T,f=100` (direct transfer) correctly, and iTerm2 too.
  The native-window report lists Konsole's kitty support as partial
  (direct transfer only), and direct transfer is all that plume sends.
- **xterm is never chosen automatically.**
  It draws sixel only when started with `-ti vt340`, and `XTERM_VERSION` does not say how it was started.
- **VS Code's terminal and alacritty** fall to rule 6.
  VS Code draws images only with `terminal.integrated.enableImages` set.

### Encoding

- **kitty:** `ESC _G a=T,f=100,q=2[,m=1] ; <base64> ESC \`, in chunks of 4096 base64 characters.
  `m=1` is set on every chunk but the last, `m=0` on the last, and a single chunk carries no `m`.
  `q=2` suppresses replies, which would otherwise arrive on the line editor's stdin.
- **iTerm2:** `ESC ]1337;File=inline=1;size=N;preserveAspectRatio=1:<base64> BEL`.
- **sixel:** `ESC P0;1;0q "1;1;W;H`, then the palette
  (`#i;2;r;g;b` in percent),
  then six-row bands with run-length encoding (`!n<char>` for runs longer than three), terminated by `ESC \`.
  Up to 256 distinct colours are encoded exactly.
  Beyond that, pixels are grouped into 4096 bins (4 bits per channel), and the 256 fullest bins become the palette.
  The test image, with a soft gradient background, came out with 8 palette entries.
  `sixel2png` decoded it with a mean absolute error of 1.3 % and a maximum of 3.5 % per channel.
  Flat chart colours are exact; gradients band.
- **Cursor placement** after the image differs by terminal, as seen in the captures:
  - ghostty (kitty) and Konsole (iTerm2) leave the cursor on the image's last row, so kitty and iTerm2 output ends with
    `\r\n`.
    iTerm2 itself may then show one blank line; not checked.
  - xterm and Konsole put the cursor below a sixel image, so sixel output ends without a line break.
- In release builds, encoding a 640×480 test image took 6 ms for kitty and 5 ms for sixel.
  The sequences were 15 KB and 21 KB.

### Verified

| Check | How | Result |
|---|---|---|
| ghostty, auto | ghostty under Xwayland running the example, captured | kitty chosen; image drawn; text resumes below |
| Konsole, auto and forced iTerm2 | same with Konsole | kitty chosen; both protocols drawn; text below after the `\r\n` fix |
| xterm, forced sixel | `xterm -ti vt340 -xrm 'XTerm*numColorRegisters: 256'` | image drawn, text below |
| Sixel decoding | `sixel2png`, `magick compare -metric MAE/PAE` | 1.3 % mean, 3.5 % max error |
| pty byte stream | `script -q -e -c 'show terminal > stdout.txt'` | 3 kitty commands (`m=1`, `m=1`, `m=0`, 4096/4096/2568 chars); PNG reassembled; stdout empty |
| No controlling terminal | the agent shell, and `setsid -w` | "cannot open /dev/tty (No such device or address (os error 6))" |
| tmux, unknown `TERM` | `TMUX=…`, `TERM=alacritty` | unavailable with the reason |

Not verified: kitty itself, WezTerm, iTerm2, foot, Windows Terminal.

## Window

### Design

- One thread per window, named `plume-window-N`.
  The thread creates the `minifb` window, reports success or failure over a channel,
  and then redraws every 33 ms until the window is closed (title-bar button or Escape).
  X11 keeps no copy of the window contents, so the redraw loop also repairs exposed areas.
- `wait = false` returns as soon as the first frame is drawn.
  `wait = true` also joins the thread.
  A failure to open the window is returned as `Failed`, so `auto` moves on to the browser.
- The window is resizable, with `ScaleMode::AspectRatioStretch`.
  sway tiled it to 1920×1107, and the chart was scaled to fit.
- **macOS:** only `wait = true` on the main thread, checked with `pthread_main_np()`.
  The window then runs on the calling thread.
  Otherwise the viewer is unavailable with a reason (`wait := true`, or `SET threads = 1`).
- **Linux and the BSDs:** available when `WAYLAND_DISPLAY` or `DISPLAY` is set.
  minifb tries Wayland first and falls back to X11.
- **Windows:** always available.

### Process exit with windows open

- Window threads are detached and never joined.
  The `minifb::Window` is a local of its thread, and Rust runs no destructors for other threads at exit,
  so exit never runs `minifb`'s `Drop`.
  That `Drop` would call `XDestroyWindow`, or tear down the Wayland objects and print the warning below.
  The operating system removes the windows with the process.
- On Unix an `atexit` handler, registered with the first window, marks the process as exiting.
  It then waits up to 250 ms for any window thread that is inside a `minifb` call.
  A window thread checks the mark before each call and parks forever once it is set.
  So no thread is inside Xlib or libwayland while `exit` runs the host's own destructors.
  The handler is registered from the library.
  With glibc, an `atexit` handler registered from a shared object also runs on `dlclose`.
  A `dlclose` would unmap the window threads' code in any case; DuckDB does not unload extensions.
- On Windows `ExitProcess` terminates the other threads before a DLL's `atexit` handlers run,
  so no handler is registered there.
  That is inferred from Microsoft's documentation; it was not run.

### Verified (Linux)

| # | Experiment | Result |
|---|---|---|
| 1 | Wayland, `wait = false`, exit after 1.5 s | sway showed `plume chart 1` (`xdg_shell`); `show` returned in 17 ms; exit 0 after 1.54 s; window gone |
| 2 | X11 (`WAYLAND_DISPLAY` unset), same | `xwayland` window; returned in 20 ms; exit 0 |
| 3 | Three windows, Wayland and X11 | all three listed at once; exit 0 after 1.6 s |
| 4 | 20 × four windows, exit after 0–90 ms, Wayland and X11 | 40 of 40 exited 0 in 114–431 ms; no stderr output; no window left |
| 5 | `wait = true`, closed through `swaymsg '[title=…] kill'` after 1.2 s, Wayland and X11 | returned at 1.26–1.27 s |
| 6 | `wait = false`, closed after 0.8 s while the process lives on | thread ended; process exited 0 at 2 s |
| 7 | Wayland close in 5 and 6 | libwayland printed `warning: queue … destroyed while proxies still attached` with 19 proxies; X11 printed nothing |
| 8 | Neither display variable set | unavailable: "no display: neither WAYLAND_DISPLAY nor DISPLAY is set" |

The `swaymsg … kill` close sends the same `xdg_toplevel.close` / `WM_DELETE_WINDOW` as the title-bar button.
Nothing was shown on screen, since it was locked;
that the pixels look right in a `minifb` window is covered by the screenshot in the native-window report.

### The Wayland warning

It comes from `minifb`'s Wayland backend
(wayland-client 0.29), which destroys its event queue before the objects attached to it.
It appears only when a user closes a window, not at process exit.
In the DuckDB shell it will print between prompts.
Ways to remove it, none done here:

- Fix the drop order in `minifb` upstream.
- Replace libwayland's log handler with `wl_log_set_handler_client`.
  This is process-wide, and the handler is a C variadic function, which stable Rust cannot define.
- Keep closed Wayland windows alive in a hidden state: not possible, since closing requires destroying the surface.

## Browser

- The page is one HTML file with the PNG as a `data:` URI, scaled down to fit the browser window.
  It is written to `plume` in the cache directory: `$XDG_CACHE_HOME` or `~/.cache` on Linux,
  `~/Library/Caches` on macOS, `%LOCALAPPDATA%` on Windows.
  File names are `chart-<ms>-<pid>-<n>.html`, and pages older than a day are removed on the next write.
- The opener is `xdg-open` (Linux, BSD) or `open` (macOS),
  spawned with null stdio and reaped by a thread without waiting.
  Windows calls `ShellExecuteW` directly.
  The `open`/`opener` crates were not needed.
- On Linux the browser requires `WAYLAND_DISPLAY` or `DISPLAY`.
  Without one, `xdg-open` can start a text-mode browser on the terminal the shell is using.
- Verified: the page renders in headless Chromium 153.
  With a stand-in `xdg-open` that sleeps 3 s, `show` returned in about 100 ms and the stand-in received the page's path.
- Not verified: the user's real default browser (not opened on purpose), Snap-packaged browsers (which cannot read
  `~/.cache`), macOS `open`, Windows `ShellExecuteW`.

## Auto

- Order: terminal, window, browser.
  An unavailable or failed viewer is skipped; image errors stop the search.
- In the agent's shell (no controlling terminal, display set), `auto` opened a window.
- With no terminal and no display the error was:
  "no viewer can show the chart here: terminal: cannot open /dev/tty (…); the process has no controlling terminal;
  window: no display: neither WAYLAND_DISPLAY nor DISPLAY is set; browser: no graphical session: neither
  WAYLAND_DISPLAY nor DISPLAY is set".

## Unconfirmed, for the manual acceptance of `show()`

- **macOS:**
  - the `pthread_main_np()` check;
  - a blocking `minifb` window on the main thread returning when closed;
  - `open` for the browser;
  - iTerm2 inline images;
  - the macOS window code, which was not even compiled: `minifb`'s build script needs the Apple SDK.
- **Windows:**
  - `CONOUT$` with `ENABLE_VIRTUAL_TERMINAL_PROCESSING`;
  - sixel in Windows Terminal 1.22 or later;
  - `minifb` windows on worker threads and process exit with them open;
  - `ShellExecuteW`.
- **Inside the v2 preview CLI:** everything above, from a volatile scalar function.
  In particular, placement of the terminal image above the result table,
  and the Wayland close warning appearing between prompts.
- **Terminals not run:** kitty, WezTerm, iTerm2, foot, Windows Terminal, and the cursor position after an iTerm2
  image in iTerm2 itself.

## How to repeat

```sh
cargo run --example show -- terminal                  # auto-detected protocol, to /dev/tty
cargo run --example show -- terminal --protocol sixel # forced
cargo run --example show -- window --hold 3           # non-blocking; process exits after 3 s
cargo run --example show -- window --wait             # blocks until closed
cargo run --example show -- browser
cargo run --example show -- --out seq.bin --protocol kitty   # bytes to a file instead of the terminal
```

Unit tests (`cargo nextest run`) cover kitty chunking, iTerm2 framing, sixel bands, palettes and run-length encoding,
the detection rules with injected environments, window availability per platform, cache paths, and settings parsing.
