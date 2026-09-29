# Showing a rendered chart from inside a DuckDB extension

Research date: 2026-09-27.
Sources inspected: crate sources of `minifb` 0.28.0, `winit` 0.30.13, `softbuffer` 0.4.8, `show-image` 0.14.1,
`eframe` 0.36.2, `egui-plotter` 0.6.0, `plotters-piston` 0.3.0, `viuer` 0.11.0, `open` 5.4.4, `opener` 0.8.5,
`duckdb` 1.10505.0 (the Rust crate), Rust std 1.98.1; Apple, Microsoft,
man-page and terminal documentation listed under [Sources](#sources).
Hands-on: Arch Linux, sway 1.12 (Wayland, XWayland on `:1`), ghostty 1.3.1, tmux 3.7c, DuckDB CLI 1.5.5,
Python `duckdb` 1.5.5.
No macOS or Windows machine was available,
so everything said about those platforms comes from source code and documentation.

## Summary

- **Linux (verified) and Windows (inferred): an in-process window from any thread works.**
  `minifb` and `winit` + `softbuffer` opened windows from a worker thread
  while the main thread was blocked reading stdin, on both Wayland and X11.
  They also worked inside the real DuckDB 1.5.5 CLI and the Python `duckdb` 1.5.5 host.
  Both display modes worked: (a) blocking until the window is closed, and (b) detached,
  with the window closing silently when the process exits.
- **macOS: in-process windows depend on the host's main thread.** winit refuses
  (`expect`s a `MainThreadMarker`).
  minifb creates the window on the calling thread and then waits synchronously on the main run loop.
  Blocks sent to the main dispatch queue run only if the main thread runs `dispatch_main`,
  `NSApplicationMain` or a `CFRunLoop`, and a CLI blocked in its line editor does none of these.
  - **Blocking mode:** feasible only when the function happens to run on the main thread.
    DuckDB ran a single-row scalar call there in 27 of 30 tries, and always with `SET threads=1`.
  - **Detached mode:** not feasible in-process on macOS.
- **A subprocess viewer supports both modes on every platform, because the child owns its own main thread.**
  - Verified on Linux: the child sees EOF on its stdin pipe when the parent exits, including after `SIGKILL`.
  - Gotcha: Rust's `Child::wait()` closes the child's stdin before waiting.
  - The cost is distribution.
    A `.duckdb_extension` is one file,
    and extracting an embedded helper binary runs into Apple Silicon signing
    and managed-Windows application-control policies.
- **Inline images inside the DuckDB shell work in ghostty (verified with a screenshot).**
  - A scalar function wrote the kitty graphics protocol to `/dev/tty`.
    The image lands after the echoed statement and before the result table,
    because duckbox renders only once the result is materialised.
  - The line editor did not paint over it.
  - `.once`/`.output` send the table to the file while the image still reaches the terminal.
  - tmux breaks it: the sequence is swallowed, or with passthrough the image is drawn at the wrong place.
- **The browser is the simplest cross-platform viewer, but offers no reliable "close when DuckDB exits".**
  Snap-packaged browsers cannot read `/tmp` or `~/.cache`.
- **Display functions must be marked volatile.**
  A non-volatile function with constant arguments is subject to constant folding
  and runs on the client thread at plan time.

## Constraints from the host

Verified in the DuckDB 1.5.5 CLI with a test extension built from the repo's scaffold (git `47de8784aad1`):

- **The calling thread varies.**
  - A single-row `SELECT plume_tid(0)` ran on the client (main) thread in 27 of 30 runs and on worker threads in 3.
  - A parallel scan of a 20M-row table ran the function on 12 distinct threads (`threads` = 12).
    About 2.2M of the 20M rows ran on the same thread as a scalar subquery in that statement,
    normally the client thread.
  - With `SET threads=1` all rows ran on one thread.
  - In Python the tested calls also ran on the main thread.
- **Volatility.**
  duckdb-rs exposes `VScalar::volatile()`, backed by `duckdb_scalar_function_set_volatile`.
  - Without it, a zero-argument `plume_tid()` reported the client thread for every row of the parallel scan, which is
    consistent with plan-time constant folding.
  - With `volatile() -> true` and a column argument it reported 12 threads.
  - A side-effecting `show()` must be volatile, or it may run at plan time or run a different number of times than
    expected.
- **Blocking and Ctrl-C.**
  - While a blocking window was open the prompt did not return, and text typed meanwhile was not executed.
  - Ctrl-C did not close the window or return the prompt.
    After the window was closed the prompt came back with no result printed
    (the interrupted query's result was dropped), and the typed-ahead line was discarded.
  - A blocking implementation that should honour Ctrl-C would have to poll for interruption itself.
    No C API hook for that was checked.
- **Result rendering.**
  - The default duckbox renderer materialises the full result before printing
    (`ModeDuckBoxRenderer::RequireMaterializedResult()` returns true in `tools/shell/shell_renderer.cpp`).
  - The progress bar (`enable_progress_bar`, default on, after `progress_bar_time` = 2000 ms) is updated on the client
    thread between tasks.
- **stderr noise.**
  When a minifb window is dropped on Wayland,
  libwayland prints a multi-line `queue ... destroyed while proxies still attached` warning to stderr,
  and it shows up in the DuckDB shell.

## In-process window libraries

| Crate                                  | Latest stable (date)                     | Linux X11 / Wayland     | macOS             | Windows                   | Event loop off the main thread                                                                            | Weight (unique crates, incl. plotters) |
| -------------------------------------- | ---------------------------------------- | ----------------------- | ----------------- | ------------------------- | --------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| `minifb`                               | 0.28.0 (2025-01-20)                      | yes / yes               | yes (Metal)       | yes                       | Linux: works (verified). Windows: no check in source. macOS: no check, but see [macOS](#macos-feasibility) | 38 (Linux), 97 (all targets)           |
| `winit` + `softbuffer`                 | 0.30.13 (0.31.0-beta.3 out) + 0.4.8 (2025-12-13) | yes / yes        | yes               | yes                       | Linux, Windows: `with_any_thread(true)`, otherwise panics (verified on Linux). macOS: panics               | 103 (Linux), 219 (all targets)         |
| `show-image`                           | 0.14.1 (2025-02-23)                      | via winit 0.28.6 + wgpu 0.17 | same         | same                      | No, on any platform: `run_context` must run on the main thread and never returns                           | heavy (wgpu, image)                    |
| `eframe` (+ `egui-plotter`)            | 0.36.2 (2026-09-08) + 0.6.0 (2025-08-20) | yes / yes               | yes               | yes                       | Via the `event_loop_builder` hook on Linux/Windows; not on macOS                                           | default features pull wgpu, accesskit  |
| `plotters-piston`                      | 0.3.0 (2020-09-13)                       | via `piston_window` 0.112 | not inspected   | not inspected             | not inspected                                                                                              | stale                                  |

### minifb 0.28.0

- Default features are `wayland`, `x11` and `dlopen`.
  `Window::new` tries Wayland first and falls back to X11 (`src/os/posix/mod.rs`).
- A window is a plain value owned by the thread that created it.
  `update_with_buffer` pumps that window's events, so there is no global event loop and no per-process singleton.
- The X11 backend calls `XInitThreads` before `XOpenDisplay`.
- The macOS backend (`src/native/macosx/MacMiniFB.m`) does no main-thread check.
  `mfb_open` calls `[NSApplication sharedApplication]`, allocates the `NSWindow` on the calling thread,
  calls `performSelectorOnMainThread:@selector(makeKeyAndOrderFront:) … waitUntilDone:YES`
  and `dispatch_async(dispatch_get_main_queue(), …)`,
  and `update_events` calls `[NSApp nextEventMatchingMask:…]` on the calling thread.
- Verified on Linux (see [Hands-on results](#hands-on-results)):
  - It works from any thread, including two windows on two threads at the same time.
  - Closing the window returns control.
  - Process exit with the window open is clean.

### winit 0.30.13 + softbuffer 0.4.8

- `EventLoopBuilder::build` documents that the event loop "must be created on the main thread,
  and only once per application".
  A second `build()` returns `EventLoopError::RecreationAttempt`, enforced by a process-wide atomic
  (`src/event_loop.rs`).
- **Linux:** the check compares `gettid()` with `getpid()`
  and panics with "Initializing the event loop outside of the main thread is a significant cross-platform compatibility
  hazard…" unless `with_any_thread(true)` is set (`src/platform_impl/linux/mod.rs`).
  Verified.
- **Windows:** the same check, against a main-thread id captured by a `.CRT$XCU` initializer
  (`src/platform_impl/windows/event_loop.rs`).
  Inferred: in a DLL loaded at runtime that initializer runs on the thread that loads the DLL,
  so the "main thread" is whichever thread ran `LOAD`.
  `with_any_thread(true)` avoids the question.
- **macOS:** `MainThreadMarker::new().expect("on macOS, `EventLoop` must be created on the main thread!")`
  (`src/platform_impl/macos/event_loop.rs`).
  There is no `any_thread` option on macOS.
- Because of the one-loop-per-process rule,
  the workable in-process design is one long-lived GUI thread that owns the `EventLoop`.
  Callers send it requests through an `EventLoopProxy` and wait on a channel (blocking mode) or return immediately
  (detached mode).
  This was built and verified on Linux.
- softbuffer 0.4.8 supports AppKit, Wayland, Win32, XCB and Xlib, among others.

### show-image 0.14.1

- **How it handles macOS:** it requires the application to hand over its main thread.
  `#[show_image::main]` or `run_context` runs the event loop on the main thread
  and moves user code to a background thread.
  `run_context` is `-> !` and "panics
  if it is called from any thread other than the main thread … this restriction is also enforced on other platforms"
  (`src/backend/mod.rs`).
- A shared library cannot take over the host's main thread, so show-image is unusable in an extension on every
  platform, not only macOS.
- It is also built on winit 0.28.6 and wgpu 0.17, and exits the process (`std::process::exit(-1)`) if no GPU adapter is
  found.

### eframe 0.36.2, egui-plotter 0.6.0, plotters-piston 0.3.0

- eframe uses winit 0.30.13.
  `NativeOptions::run_and_return` (default `true`) uses `run_app_on_demand` with a thread-local event loop "so we can
  support closing and opening an eframe window multiple times" (`src/native/run.rs`).
  - Inferred from that code and the winit rule: a second `run_native` from a different thread tries to build a second
    event loop and gets `RecreationAttempt`.
  - The `event_loop_builder` hook can set `with_any_thread` on Linux and Windows.
- egui-plotter 0.6.0 depends on egui 0.32.1, four minor versions behind eframe 0.36, so using it pins an old egui.
- plotters-piston 0.3.0 has not been released since 2020 and depends on `piston_window` 0.112.
- Neither adds anything for this use case over drawing a plotters bitmap into minifb or softbuffer.

Not inspected: `tao`, `fltk`, `sdl2`, `glfw`.
On macOS they are bound by the same AppKit rule.

## macOS feasibility

Verified from source:

- winit asserts it is on the main thread (above), and minifb does not.
- minifb's `performSelectorOnMainThread:… waitUntilDone:YES` returns only after the main thread has processed the
  message.

Verified from Apple documentation:

- `dispatch_get_main_queue`: blocks on the main queue are invoked by one of: "Calling dispatch_main; Calling …
  NSApplicationMain (macOS); Using a CFRunLoop on the main thread."
- `performSelectorOnMainThread:withObject:waitUntilDone:` queues the message on the main thread's run loop.
  It "depends on that runloop being run on a regular basis".
  If called on the main thread with `YES`, "the message is delivered and processed immediately".
- The (archived, outdated) Thread Safety Summary says NSView must be used "only from the main thread" and
  that "the main thread of the application is responsible for handling events".
  The same page's claim that windows may be created on a secondary thread is contradicted by current behaviour
  (next point).
- Since Mojave, AppKit raises for off-main-thread window work:
  QEMU hit "NSWindow drag regions should only be invalidated on the Main Thread!" on 10.14.2.
  "NSWindow should only be instantiated on the main thread!" is a widely reported `NSInternalInconsistencyException` on
  current macOS.
  No Apple page naming the version was found.

Inferred (not tested, no Mac available):

- **Dispatching to the main queue does not help.**
  In the DuckDB CLI the main thread is blocked in the line editor's `read()` between statements,
  and in Python in the interpreter.
  Neither runs a run loop, so blocks never execute.
  From a worker thread, minifb would either raise on `NSWindow` allocation or hang forever in `waitUntilDone:YES`.
- **Blocking mode is sound when the function is on the main thread.**
  Check with `pthread_main_np()`, then create the window and pump `nextEventMatchingMask` until it closes,
  as minifb does.
  DuckDB does not guarantee that thread (3/30 single-row calls ran elsewhere).
  `SET threads=1` made it deterministic in the tests on Linux.
- **Detached mode is impossible in-process.**
  Once the function returns, nothing pumps AppKit events.
  A window left open would stop redrawing and show as not responding.
- **Python has a hook.** matplotlib's macOS backend and Tk register `PyOS_InputHook`,
  which Python calls while the REPL waits for input, to pump GUI events.
  An extension could in principle install such a hook when hosted in Python,
  but that is Python-specific and does not exist in the DuckDB CLI's linenoise loop.
- **`fork()` without `exec` does not help on macOS**: CoreFoundation and AppKit are unsafe to use after fork.
  On Linux a forked child's single thread has `tid == pid`, so this is a Linux-only option, not tried.

## Hands-on results

Scratch code (not checked in): a `render` crate drawing two plotters `LineSeries` into a 480×320 buffer
(plotters 0.3.7, `bitmap_backend` only),
cdylibs `libminifb` and `libwinit`, a `host` that loads them with `libloading` 0.8, and a `viewer-child` binary.
Windows were confirmed with `swaymsg -t get_tree`
(title, `shell` = `xdg_shell` or `xwayland`, pid) and one `grim` screenshot showing the rendered chart.
Closing a window was simulated with `swaymsg '[title=…] kill'`,
which sends the same close request as the title-bar button.

| # | Experiment                                                                  | Result                                                                                                            |
| - | --------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| 1 | minifb, worker thread, main blocked on stdin, Wayland                        | Window opened (`xdg_shell`); on close the call returned 0 while main kept reading stdin                           |
| 2 | Same, host exits (stdin EOF) with the window open, Wayland and X11           | Exit code 0, no crash, window gone                                                                                |
| 3 | minifb twice on the main thread                                              | Second window opened after the first closed                                                                       |
| 4 | minifb, two detached windows on two threads, Wayland and X11                 | Both open at once; host exit removed both                                                                         |
| 5 | winit, default builder, worker thread                                        | Panic: "Initializing the event loop outside of the main thread …"                                                  |
| 6 | winit, default builder, main thread, twice                                   | First window fine; second `build()` returned `RecreationAttempt`                                                  |
| 7 | winit GUI-thread design (`with_any_thread`, proxy), calls from two threads   | Both blocking calls got windows from the same loop; detached windows and host exit also clean (Wayland and X11)   |
| 8 | Subprocess: detached child, parent exits normally                            | Child logged stdin EOF and exited; window gone                                                                    |
| 9 | Subprocess: detached child, parent `SIGKILL`ed                               | Same                                                                                                              |
| 10 | Subprocess: blocking, parent calls `child.wait()`                           | Child got EOF immediately and exited: `wait()` drops `stdin` first. Fixed by taking `child.stdin` before waiting  |
| 11 | DuckDB CLI: `SELECT plume_show(0)` (blocking minifb)                      | Ran on the main thread; shell blocked until close; next statement then ran                                        |
| 12 | DuckDB CLI interactive (tmux): `plume_show_detached()`, then `SELECT 42`, then `.quit` | Window open, shell usable, `.quit` exited 0 and the window vanished                                  |
| 13 | Python `duckdb` 1.5.5: blocking, then detached, then interpreter exit        | Both worked; exit code 0                                                                                          |

The workspace (plotters, minifb, winit, softbuffer, libloading) built in about a minute.
Scaffold notes: `make configure` wrote a git error into `configure/extension_version.txt`
because the exported tree is not a git repository, which made DuckDB fail the load with "Unknown ABI type".
Writing `v0.0.1` there fixed it.

## Subprocess viewer

A helper process owns the window on its own main thread.
That sidesteps every threading rule above, macOS included.

### Detecting parent exit

- **Stdin pipe EOF** (verified on Linux, clean exit and `SIGKILL`).
  - The parent keeps the write end open for its lifetime; the kernel closes it on any kind of exit.
  - Works the same with anonymous pipes on Windows and macOS (inferred).
  - Rust's std creates its pipes with `O_CLOEXEC` (`pipe2`), so exec'd grandchildren do not hold the write end.
  - If the host forks without exec (Python `multiprocessing` with the `fork` start method), the forked process inherits
    the write end and EOF is delayed until it exits too (inferred).
- **`prctl(PR_SET_PDEATHSIG)`** (Linux only):
  the man page says the parent "is considered to be the thread that created this process … the signal will be sent when
  that thread terminates".
  Spawned from a short-lived host thread (for example a Python thread), it kills the viewer early.
- **Windows job objects:** with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`,
  "closing the last job object handle terminates all associated processes",
  and process termination closes the parent's handles.
  Nested jobs exist since Windows 8, so this works even if the host is already in a job.
  By default the child's own children join the job.
- **PID watching:** `pidfd_open` on Linux
  (readable when the process exits), `kqueue` `EVFILT_PROC`/`NOTE_EXIT` on macOS, `OpenProcess` + wait on Windows.
  These are more robust than polling `getppid()`, which avoids PID-reuse races only on Linux.

### Blocking and detached

- **Blocking:** spawn, take `child.stdin`, then `wait()`.
- **Detached:** spawn and keep `ChildStdin` alive in a static for the rest of the process.
- Both verified (experiments 8–10).
- A viewer that reads chart updates over the same pipe could also be reused for later charts.

### Where the helper binary comes from

1. **A separate binary on `PATH`** (or a path in a DuckDB setting).
   Simple and signable, but an extra install step outside DuckDB's `INSTALL`.
2. **Embedded in the extension and extracted to a cache directory on first use.**
   - **Apple Silicon:** "the operating system enforces that any executable must be signed
     before it's allowed to run … a simple ad-hoc signature is sufficient", and `ld` ad-hoc signs by default.
     The signature lives in the Mach-O, so it survives being embedded as bytes (inferred).
   - **Gatekeeper:** the quarantine xattr is opt-in, set by downloading apps through `LSFileQuarantineEnabled`
     ("notable exemption is … curl").
     A file written by the DuckDB process should not be quarantined and so should not face the notarization check
     (inferred from Oakley; not confirmed by Apple).
     XProtect's first-run malware scan still applies ("regardless of how it arrived").
   - **Windows:** SmartScreen keys off Mark-of-the-Web, which "Internet clients must explicitly mark",
     and "the CreateProcess API does not care about the MotW"
     (Eric Lawrence), so an extracted exe run with `CreateProcess` should not trigger SmartScreen.
     Managed machines are another matter.
     AppLocker's default executable rules allow only `%windir%` and `%programfiles%`,
     and App Control (WDAC) FilePath rules require admin-only-writable paths,
     so an exe in `%LOCALAPPDATA%` is blocked there.
   - **Linux:** `/tmp` is often `noexec`, so use `$XDG_CACHE_HOME`.
     Also check the extracted file's hash before running it, since the cache dir is user-writable.
   - It also multiplies artefact size and build matrix: one helper per platform inside each platform's extension.
3. **A Python viewer** (`python3 -c …` with tkinter).
   - No binary to ship, and the child's main thread is its own, so macOS is fine.
   - Requires Python with Tk (some distributions package tkinter separately).
   - Tk 8.6 reads PNG, and Tk 9 adds SVG.
   - Not tested.
4. **The OS default viewer** (`xdg-open`/`open`/`start`, or the `open`/`opener` crates).
   - `open::that` may block on some platforms, so use `that_detached`.
   - `opener::open` uses `ShellExecuteW` on Windows and `open` on macOS.
     On Linux it uses `xdg-open`, with an embedded copy of the script as fallback.
   - What is lost:
     - No blocking mode: the launcher returns at once, and the viewer may be a pre-existing single-instance process.
     - No close on exit.
     - No reuse or update of a window.
     - No knowledge of whether a viewer exists.
     - A temp-file lifetime race: the launcher returns before the viewer reads the file.
5. **The extension file itself as the helper** (the `libc.so.6` trick).
   - **Linux x86_64: verified.**
     A Rust cdylib with a `.interp` section (`/lib64/ld-linux-x86-64.so.2`) and `-Wl,-e,<entry>` runs as a program.
     The entry is a `global_asm!` trampoline that realigns the stack.
     - It runs as `./lib.so` with `+x`, and without `+x` as `/lib64/ld-linux-x86-64.so.2 ./lib.so`.
     - `dlopen` (via Python `ctypes`) still loads it and calls its exports.
     - `std::env::args()` was empty in that mode; read `/proc/self/cmdline` instead.
     - Installed extensions are `-rw-r--r--` (`~/.duckdb/extensions/v1.5.5/linux_amd64/`), so the explicit loader form
       is the usable one.
     - The interpreter path is per libc and architecture.
     - DuckDB's appended metadata footer should be ignored by the ELF loader (inferred; not tried on a real
       `.duckdb_extension`).
   - **macOS: not possible (inferred).**
     `execve` needs an `MH_EXECUTE` image, and an extension is an `MH_DYLIB`.
   - **Windows: not directly (inferred).**
     A DLL cannot be started with `CreateProcess`.
     `rundll32.exe ext.dll,Entry` can host an exported function in a new process,
     but it expects a specific signature and is a common target of security tooling.
6. **OS scripting hosts as the helper (untested idea).**
   `osascript -l JavaScript` can create an `NSWindow` through the Objective-C bridge,
   and PowerShell can show a WinForms window.
   Both are Apple- or Microsoft-signed binaries present on every install.
   Not verified.

## Inline terminal graphics

### Protocol support (documentation, 2026)

| Terminal            | kitty graphics                  | iTerm2 OSC 1337                   | Sixel                                  |
| ------------------- | ------------------------------- | --------------------------------- | -------------------------------------- |
| kitty               | yes                             | no                                | no                                     |
| ghostty             | yes                             | not documented; ignored in test   | no                                     |
| WezTerm             | yes (`enable_kitty_graphics`)   | yes                               | yes (experimental)                     |
| iTerm2              | yes                             | yes                               | yes (since 3.3)                        |
| Konsole             | partial (direct transfer only)  | yes                               | yes (since 22.04)                      |
| foot                | no                              | no                                | yes                                    |
| Windows Terminal    | no evidence                     | no evidence                       | yes (since 1.22)                       |
| conhost             | no                              | no                                | Canary build 29558 only                |
| alacritty           | no                              | no                                | no (issue #910 open)                   |
| VS Code terminal    | listed via xterm.js (unconfirmed) | yes, `terminal.integrated.enableImages` | yes, same setting          |

tmux needs `allow-passthrough on` (since 3.3) and DCS `\ePtmux;` wrapping. tmux 3.4+ can render sixel itself
if built with `--enable-sixel`.

### viuer 0.11.0

- It writes to `std::io::stdout()`.
- It probes kitty support by writing queries and reading replies (`console::Term::stdout`, DSR `\e[5n`).
- It detects iTerm2 from `TERM_PROGRAM`/`LC_TERMINAL`/`KONSOLE_VERSION`.
- Sixel is behind the `sixel` feature (`sixel-rs`, which needs libsixel) or the pure-Rust `icy_sixel` feature.
- Inside DuckDB, stdout may be redirected, and reading terminal replies from stdin competes with the line editor.
  A hand-rolled writer (about 40 lines for kitty and iTerm2, as in the test) to `/dev/tty` fits better.

### Inline images inside the DuckDB shell

Setup: the scaffold extension gained `plume_inline(proto, target)`.
It renders the chart to PNG (`png` 0.18) and writes either the kitty sequence
(`a=T,f=100,q=2`, 4096-byte base64 chunks) or iTerm2 `OSC 1337;File=inline=1`.
The target is `/dev/tty`, tmux-wrapped `/dev/tty`, or fd 1.
The PNG payload was 6,546 bytes, against 615,777 for the same image as raw RGB (`f=24`).
Sessions ran in ghostty 1.3.1 and were checked with `grim` screenshots.
The interactive session used a small Python pty driver so that linenoise saw a real terminal
and ghostty rendered the output.
Byte order was checked from `script(1)` typescripts.

- **Placement.**
  In both interactive and batch sessions the image appears directly above the result table,
  and in interactive sessions directly below the echoed statement.
  The function runs during execution and duckbox prints only after materialisation.
  With two rows, two images stacked above one table.
- **Corruption.**
  None: the table and the next prompt printed cleanly below the image.
- **Line editor.**
  Typing a long line, Ctrl-A/Ctrl-E, backspacing, Ctrl-U and recalling history with Up redrew only the prompt line.
  Images above stayed intact.
  A screen clear (Ctrl-L, observed inside tmux) removes them.
- **`.mode line`** with the stdout target: image above the record.
  **`.mode csv`** over 5000 rows, calling the function on 3 of them:
  all 3 images came before the first row in the byte stream,
  so the result was buffered before printing even in this streaming mode.
  With much larger streamed results, images interleaving with rows is conceivable (inferred).
- **`.once file` / `.output file`**: the table went to the file.
  The image reached the terminal with both the `/dev/tty` and the stdout target,
  because the CLI's output redirection does not touch fd 1.
  The file contained no escape bytes.
- **Shell redirection** (`duckdb … > file`): the stdout target put the escape sequence into the file.
  The `/dev/tty` target still drew on the terminal.
  With stdout sent to `/dev/null` under `script`, the three images still reached the pty.
- **iTerm2 protocol in ghostty**: not rendered (blank line).
- **tmux 3.7c inside ghostty**:
  - With the plain kitty sequence, no image appeared.
    The tail of the sequence showed up as the pane title in the status line (`"Gm=0;dD5E…"`).
  - With the DCS-wrapped sequence and `allow-passthrough off`, nothing appeared.
  - With the DCS-wrapped sequence and `allow-passthrough on`, ghostty drew the image at the top-left of the pane over
    earlier text rather than at the cursor, and the next screen clear erased it.
  - `tmux capture-pane` showed only the text.
- **No controlling terminal**: opening `/dev/tty` failed with `ENXIO` (os error 6).
  This is the case for services and notebook kernels, and the function must then fall back to something else.
- **Not tested:** sixel (xterm and Konsole are installed here but were not tried), kitty, WezTerm, iTerm2, and Windows.
  On Windows the equivalent of `/dev/tty` is opening `CONOUT$`.
  Windows Terminal's sixel support suggests that path works there (inferred).

## Browser as viewer

- Write an HTML or SVG file and open it with `opener::open_browser` (honours `$BROWSER`) or `open::that_detached`.
  This works on all three platforms with no extra binary.
- **Snap-packaged Firefox/Chromium** (the Ubuntu default) use a private `/tmp`.
  Launchpad #1972762 reports that files in `/tmp` or `~/.cache` "won't open".
  Serving the chart from a localhost HTTP server in the extension avoids file access entirely.
- **"Close when DuckDB closes"** has no direct equivalent: a process cannot close another program's tab.
  With a localhost server the page can notice the dropped connection (SSE or WebSocket) and show that the session ended.
  Scripts may close only windows they opened themselves (inferred from browser behaviour, not tested).
- **Blocking mode** is possible only with such a server, which learns from the page (a beacon on unload, or a
  WebSocket close) when the tab went away.
- **Temp files:** `xdg-open` and friends return before the browser reads the file, so deleting it immediately races
  (inferred).

## Implications for plume

- Mark every display function volatile.
  Never assume a particular calling thread; check it where it matters (`pthread_main_np` on macOS).
- **Linux and Windows:** an in-process window is viable for both modes.
  - minifb gives one window per thread with no global state.
  - winit needs a single long-lived GUI thread with `with_any_thread(true)`, because the event loop can be created only
    once per process.
- **macOS:** in-process covers only blocking mode, and only when running on the main thread.
  Detached mode needs a helper process or the browser.
- **Inline kitty graphics** work well in capable terminals and need no window system.
  Write to `/dev/tty` and fall back when it cannot be opened or tmux is detected.
- **A helper process** is the only uniform native-window route, and its cost is packaging.
  The Linux executable-`.so` trick removes the separate file on Linux only.

## Sources

- Apple: [dispatch_get_main_queue](https://developer.apple.com/documentation/dispatch/dispatch_get_main_queue),
  [performSelectorOnMainThread](https://developer.apple.com/documentation/objectivec/nsobject-swift.class/performselector(onmainthread:with:waituntildone:)),
  [Thread Safety Summary (archived)](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/ThreadSafetySummary/ThreadSafetySummary.html),
  [AppKit release notes for macOS 14](https://developer.apple.com/documentation/macos-release-notes/appkit-release-notes-for-macos-14),
  [Big Sur universal apps release notes (signing)](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes),
  [Gatekeeper and runtime protection](https://support.apple.com/guide/security/gatekeeper-and-runtime-protection-sec5599b66df/web)
- [QEMU bug 1802684 (Mojave main-thread enforcement)](https://bugs.launchpad.net/qemu/+bug/1802684),
  [opencv_ffi issue 19 ("NSWindow should only be instantiated on the main thread!")](https://github.com/Levi-Lesches/opencv_ffi/issues/19)
- Howard Oakley:
  [Who decides to quarantine files?](https://eclecticlight.co/2025/12/08/who-decides-to-quarantine-files/),
  [Explainer: quarantine](https://eclecticlight.co/2021/12/11/explainer-quarantine/)
- [PyOS_InputHook](https://docs.python.org/3/c-api/veryhigh.html#c.PyOS_InputHook),
  [matplotlib interactive guide](https://matplotlib.org/stable/users/explain/figure/interactive_guide.html)
- [PR_SET_PDEATHSIG(2const)](https://man7.org/linux/man-pages/man2/PR_SET_PDEATHSIG.2const.html),
  [pidfd_open(2)](https://man7.org/linux/man-pages/man2/pidfd_open.2.html),
  [kqueue(2)](https://man.freebsd.org/cgi/man.cgi?query=kqueue&sektion=2)
- Microsoft:
  [JOBOBJECT_BASIC_LIMIT_INFORMATION](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information),
  [Job objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects),
  [Nested jobs](https://learn.microsoft.com/en-us/windows/win32/procthread/nested-jobs),
  [Terminating a process](https://learn.microsoft.com/en-us/windows/win32/procthread/terminating-a-process),
  [SmartScreen](https://learn.microsoft.com/en-us/windows/security/operating-system-security/virus-and-threat-protection/microsoft-defender-smartscreen/),
  [AppLocker executable rules](https://learn.microsoft.com/en-us/windows/security/application-security/application-control/app-control-for-business/applocker/executable-rules-in-applocker),
  [App Control rule types](https://learn.microsoft.com/en-us/windows/security/application-security/application-control/app-control-for-business/design/select-types-of-rules-to-create),
  [Windows Terminal 1.22 (sixel)](https://github.com/microsoft/terminal/discussions/18516),
  [Insider build 29558 (conhost sixel)](https://blogs.windows.com/windows-insider/2026/03/30/announcing-windows-11-insider-preview-build-for-canary-channel-29558-1000/)
- [Eric Lawrence: Downloads and the Mark-of-the-Web](https://textslashplain.com/2016/04/04/downloads-and-the-mark-of-the-web/)
- [Snap security policies (private /tmp)](https://snapcraft.io/docs/explanation/security/security-policies/),
  [Launchpad #1972762](https://bugs.launchpad.net/bugs/1972762)
- Terminals: [kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/),
  [Ghostty features](https://ghostty.org/docs/features),
  [WezTerm features](https://wezterm.org/features.html),
  [iTerm2 images](https://iterm2.com/documentation-images.html),
  [foot](https://codeberg.org/dnkl/foot),
  [alacritty #910](https://github.com/alacritty/alacritty/issues/910),
  [Konsole MR 594](https://invent.kde.org/utilities/konsole/-/merge_requests/594),
  [VS Code terminal images](https://code.visualstudio.com/docs/terminal/advanced),
  [Are We Sixel Yet](https://www.arewesixelyet.com),
  [tmux FAQ (passthrough)](https://github.com/tmux/tmux/wiki/FAQ),
  [tmux CHANGES](https://raw.githubusercontent.com/tmux/tmux/master/CHANGES)
- DuckDB: [configuration](https://duckdb.org/docs/current/configuration/overview.html),
  [CLI output formats](https://duckdb.org/docs/current/clients/cli/output_formats.html),
  [shell.cpp](https://github.com/duckdb/duckdb/blob/main/tools/shell/shell.cpp),
  [shell_renderer.cpp](https://github.com/duckdb/duckdb/blob/main/tools/shell/shell_renderer.cpp),
  [executor.cpp](https://github.com/duckdb/duckdb/blob/main/src/parallel/executor.cpp),
  [task_scheduler.cpp](https://github.com/duckdb/duckdb/blob/main/src/parallel/task_scheduler.cpp),
  [discussion 6632](https://github.com/duckdb/duckdb/discussions/6632)
- Crate docs: [winit EventLoopBuilder](https://docs.rs/winit/0.30.12/winit/event_loop/struct.EventLoopBuilder.html),
  [show-image](https://docs.rs/show-image/latest/show_image/),
  [show_image::run_context](https://docs.rs/show-image/latest/show_image/fn.run_context.html)
- Crate sources inspected: `minifb-0.28.0/src/os/posix/{mod,x11}.rs`, `minifb-0.28.0/src/native/macosx/MacMiniFB.m`,
  `winit-0.30.13/src/event_loop.rs`,
  `winit-0.30.13/src/platform_impl/{linux/mod.rs,windows/event_loop.rs,macos/event_loop.rs}`,
  `softbuffer-0.4.8/README.md`, `show-image-0.14.1/{README.md,src/backend/{mod,context}.rs,Cargo.toml}`,
  `eframe-0.36.2/src/{epi.rs,native/run.rs}`, `egui-plotter-0.6.0/Cargo.toml`, `plotters-piston-0.3.0/Cargo.toml`,
  `viuer-0.11.0/src/{lib.rs,printer/{kitty,iterm,sixel_util}.rs}`, `open-5.4.4/src/lib.rs`, `opener-0.8.5/src/lib.rs`,
  `duckdb-1.10505.0/src/vscalar/{mod,function}.rs`, Rust std 1.98.1 `process.rs` (`Child::wait`) and `sys/pipe/unix.rs`
- Repo: `plan/plume.md`, `docs/reports/*.md`, scaffold at git `47de8784aad1`
