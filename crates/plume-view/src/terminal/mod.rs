//! Inline images in the terminal.
//!
//! The image is written as one escape sequence to the controlling terminal (`/dev/tty` on Unix,
//! `CONOUT$` on Windows), so it reaches the screen even when stdout is redirected or captured by the host.
//!
//! The protocol is chosen from environment variables alone ([`detect_protocol`]).
//! The terminal is never queried: answers would arrive on stdin, which belongs to the shell's line editor.
//! Every sequence written asks for no reply (kitty `q=2`; iTerm2 and sixel have none).

mod sixel;

use std::fmt;
use std::fs::File;
use std::io::{self, Write};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::env::{self, Env};
use crate::{Image, ImageError, ShowError, Viewer};

/// An inline-image protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// The kitty graphics protocol with a PNG payload (`a=T,f=100`): kitty, ghostty, Konsole.
    Kitty,
    /// iTerm2 inline images (`OSC 1337 ; File=`): iTerm2, WezTerm.
    ITerm2,
    /// DEC sixel graphics: Windows Terminal, foot.
    Sixel,
}

impl Protocol {
    pub fn name(self) -> &'static str {
        match self {
            Protocol::Kitty => "kitty",
            Protocol::ITerm2 => "iterm2",
            Protocol::Sixel => "sixel",
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The size of one kitty payload chunk in base64 characters. The protocol allows at most 4096, and every
/// chunk but the last must be a multiple of 4.
const KITTY_CHUNK: usize = 4096;

/// Chooses the protocol for the terminal described by `env`, or says why there is none.
///
/// Terminals export variables that child processes inherit, so a terminal started from another one
/// (Konsole launched from ghostty) sees both sets. The rules therefore look first at what the innermost
/// terminal always sets (`TERM`), then at variables that terminals which leave `TERM` generic set, and last
/// at the ones most likely to be inherited. First match wins:
///
/// 1. `TMUX` set, or `TERM` starting with `screen` or `tmux`: none. Multiplexers swallow the sequences,
///    or with passthrough draw the image at the wrong place.
/// 2. `TERM=xterm-kitty` or `xterm-ghostty`: kitty graphics. `TERM=foot*`: sixel. `TERM=wezterm`: iTerm2.
/// 3. `KONSOLE_VERSION`: kitty graphics (Konsole implements direct PNG transfer, which is all that is sent).
///    `XTERM_VERSION`: none, because xterm draws sixel only when started as a VT340.
/// 4. `TERM_PROGRAM=ghostty`: kitty graphics. `TERM_PROGRAM=WezTerm`, `TERM_PROGRAM=iTerm.app` or
///    `LC_TERMINAL=iTerm2`: iTerm2 (WezTerm's kitty support is off by default).
/// 5. `KITTY_WINDOW_ID`, `GHOSTTY_RESOURCES_DIR`, `GHOSTTY_BIN_DIR`: kitty graphics.
///    `WEZTERM_EXECUTABLE`, `WEZTERM_PANE`: iTerm2. `WT_SESSION` (Windows Terminal, also seen from WSL): sixel.
/// 6. Anything else: none.
pub fn detect_protocol(env: Env<'_>) -> Result<Protocol, String> {
    let term = env::get(env, "TERM").unwrap_or_default();
    let term_program = env::get(env, "TERM_PROGRAM").unwrap_or_default();
    let term_is = |prefix: &str| term.starts_with(prefix);
    let program_is = |name: &str| term_program.eq_ignore_ascii_case(name);

    if env::has(env, "TMUX") {
        return Err("running inside tmux, which does not pass inline images through".into());
    }
    if term_is("screen") || term_is("tmux") {
        return Err(format!(
            "TERM={term} is a terminal multiplexer, which does not pass inline images through"
        ));
    }
    if term_is("xterm-kitty") || term_is("xterm-ghostty") {
        return Ok(Protocol::Kitty);
    }
    if term_is("foot") {
        return Ok(Protocol::Sixel);
    }
    if term_is("wezterm") {
        return Ok(Protocol::ITerm2);
    }
    if env::has(env, "KONSOLE_VERSION") {
        return Ok(Protocol::Kitty);
    }
    if env::has(env, "XTERM_VERSION") {
        return Err(
            "xterm shows sixel images only when started as a VT340 (xterm -ti vt340), \
                    which the environment does not reveal"
                .into(),
        );
    }
    if program_is("ghostty") {
        return Ok(Protocol::Kitty);
    }
    if program_is("WezTerm")
        || program_is("iTerm.app")
        || env::get(env, "LC_TERMINAL").is_some_and(|t| t.eq_ignore_ascii_case("iTerm2"))
    {
        return Ok(Protocol::ITerm2);
    }
    if env::has(env, "KITTY_WINDOW_ID")
        || env::has(env, "GHOSTTY_RESOURCES_DIR")
        || env::has(env, "GHOSTTY_BIN_DIR")
    {
        return Ok(Protocol::Kitty);
    }
    if env::has(env, "WEZTERM_EXECUTABLE") || env::has(env, "WEZTERM_PANE") {
        return Ok(Protocol::ITerm2);
    }
    if env::has(env, "WT_SESSION") {
        return Ok(Protocol::Sixel);
    }
    if term.is_empty() && term_program.is_empty() {
        return Err("no terminal type is set (TERM is unset)".into());
    }
    let mut what = format!("TERM={term}");
    if !term_program.is_empty() {
        what.push_str(&format!(", TERM_PROGRAM={term_program}"));
    }
    Err(format!(
        "the terminal ({what}) is not one known to show inline images \
         (kitty, ghostty, WezTerm, iTerm2, Konsole, Windows Terminal, foot)"
    ))
}

/// Shows `image` in the controlling terminal with the protocol [`detect_protocol`] picks.
pub fn show(image: &Image) -> Result<Protocol, ShowError> {
    let protocol = detect_protocol(&env::process)
        .map_err(|reason| ShowError::unavailable(Viewer::Terminal, reason))?;
    show_with(image, protocol)?;
    Ok(protocol)
}

/// Shows `image` in the controlling terminal with `protocol`, whatever the environment says.
pub fn show_with(image: &Image, protocol: Protocol) -> Result<(), ShowError> {
    let mut tty = open_tty().map_err(|reason| ShowError::unavailable(Viewer::Terminal, reason))?;
    let bytes = encode(image, protocol)?;
    tty.write_all(&bytes)
        .and_then(|()| tty.flush())
        .map_err(|e| ShowError::failed(Viewer::Terminal, format!("writing the image: {e}")))
}

/// The bytes that draw `image` with `protocol`, including the line break that follows it.
pub fn encode(image: &Image, protocol: Protocol) -> Result<Vec<u8>, ImageError> {
    Ok(match protocol {
        Protocol::Kitty => kitty(image.png()?),
        Protocol::ITerm2 => iterm2(image.png()?),
        Protocol::Sixel => sixel::encode(image.width(), image.height(), image.rgb()?),
    })
}

/// kitty graphics, transmit and display (`a=T`) a PNG (`f=100`) with replies suppressed (`q=2`).
/// The payload is split into chunks with `m=1` on all but the last; a single chunk carries no `m`.
/// kitty leaves the cursor on the image's last row, so a line break follows.
fn kitty(png: &[u8]) -> Vec<u8> {
    let data = BASE64.encode(png);
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(KITTY_CHUNK).collect();
    let mut out = Vec::with_capacity(data.len() + chunks.len() * 16 + 32);
    for (i, chunk) in chunks.iter().enumerate() {
        let last = i + 1 == chunks.len();
        out.extend_from_slice(b"\x1b_G");
        if i == 0 {
            out.extend_from_slice(b"a=T,f=100,q=2");
            if !last {
                out.extend_from_slice(b",m=1");
            }
        } else {
            out.extend_from_slice(if last { b"m=0" } else { b"m=1" });
        }
        out.push(b';');
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// iTerm2 inline image: `OSC 1337 ; File=inline=1;size=N;preserveAspectRatio=1 : base64 BEL`, shown at its
/// pixel size. Konsole leaves the cursor on the image's last row, so a line break follows.
fn iterm2(png: &[u8]) -> Vec<u8> {
    let data = BASE64.encode(png);
    let mut out = Vec::with_capacity(data.len() + 64);
    out.extend_from_slice(
        format!(
            "\x1b]1337;File=inline=1;size={};preserveAspectRatio=1:",
            png.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(data.as_bytes());
    out.extend_from_slice(b"\x07\r\n");
    out
}

#[cfg(unix)]
fn open_tty() -> Result<File, String> {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .map_err(|e| no_tty("/dev/tty", e))
}

#[cfg(windows)]
fn open_tty() -> Result<File, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Console::{
        ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, SetConsoleMode,
    };

    // GetConsoleMode needs read access to the output buffer.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONOUT$")
        .map_err(|e| no_tty("CONOUT$", e))?;
    let handle = file.as_raw_handle();
    let mut mode = 0;
    // SAFETY: `handle` is a console output handle owned by `file`, which outlives both calls.
    unsafe {
        if GetConsoleMode(handle, &mut mode) != 0 {
            let wanted = mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
            if wanted != mode {
                // Failure leaves the mode as it was; the write is tried anyway.
                SetConsoleMode(handle, wanted);
            }
        }
    }
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_tty() -> Result<File, String> {
    Err("no controlling terminal on this platform".into())
}

#[allow(dead_code)]
fn no_tty(path: &str, e: io::Error) -> String {
    format!("cannot open {path} ({e}); the process has no controlling terminal")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::fixed;

    fn detect(pairs: &[(&str, &str)]) -> Result<Protocol, String> {
        detect_protocol(&fixed(pairs))
    }

    #[test]
    fn detects_kitty_family() {
        assert_eq!(detect(&[("TERM", "xterm-kitty")]), Ok(Protocol::Kitty));
        assert_eq!(
            detect(&[("TERM", "xterm-256color"), ("KITTY_WINDOW_ID", "1")]),
            Ok(Protocol::Kitty)
        );
        assert_eq!(
            detect(&[("TERM", "xterm-ghostty"), ("TERM_PROGRAM", "ghostty")]),
            Ok(Protocol::Kitty)
        );
        // Over ssh only TERM survives.
        assert_eq!(detect(&[("TERM", "xterm-ghostty")]), Ok(Protocol::Kitty));
        assert_eq!(
            detect(&[
                ("TERM", "xterm-256color"),
                ("GHOSTTY_RESOURCES_DIR", "/usr/share/ghostty")
            ]),
            Ok(Protocol::Kitty)
        );
    }

    #[test]
    fn detects_iterm2_family() {
        for pairs in [
            &[("TERM_PROGRAM", "WezTerm")][..],
            &[("TERM", "xterm-256color"), ("WEZTERM_PANE", "0")],
            &[("TERM_PROGRAM", "iTerm.app")],
            &[("LC_TERMINAL", "iTerm2"), ("TERM", "xterm-256color")],
            &[("TERM", "wezterm")],
        ] {
            assert_eq!(detect(pairs), Ok(Protocol::ITerm2), "{pairs:?}");
        }
    }

    #[test]
    fn konsole_uses_kitty() {
        assert_eq!(
            detect(&[("KONSOLE_VERSION", "260801"), ("TERM", "xterm-256color")]),
            Ok(Protocol::Kitty)
        );
    }

    /// A terminal started from another inherits the outer one's variables.
    #[test]
    fn the_innermost_terminal_wins() {
        let ghostty_leftovers = [
            ("TERM_PROGRAM", "ghostty"),
            ("GHOSTTY_RESOURCES_DIR", "/usr/share/ghostty"),
            ("GHOSTTY_BIN_DIR", "/usr/bin"),
        ];
        let with = |extra: &[(&'static str, &'static str)]| {
            let mut pairs = ghostty_leftovers.to_vec();
            pairs.extend_from_slice(extra);
            detect(&pairs)
        };
        // xterm from ghostty: no sixel unless started as a VT340, so none.
        let e = with(&[("TERM", "xterm"), ("XTERM_VERSION", "XTerm(399)")]).unwrap_err();
        assert!(e.contains("vt340"), "{e}");
        // foot from ghostty.
        assert_eq!(with(&[("TERM", "foot")]), Ok(Protocol::Sixel));
        // ghostty from Konsole.
        assert_eq!(
            detect(&[("KONSOLE_VERSION", "260801"), ("TERM", "xterm-ghostty")]),
            Ok(Protocol::Kitty)
        );
        // WezTerm (which sets TERM_PROGRAM) from ghostty.
        assert_eq!(
            detect(&[
                ("TERM", "xterm-256color"),
                ("TERM_PROGRAM", "WezTerm"),
                ("GHOSTTY_RESOURCES_DIR", "/usr/share/ghostty"),
            ]),
            Ok(Protocol::ITerm2)
        );
    }

    #[test]
    fn detects_sixel_terminals() {
        assert_eq!(
            detect(&[("WT_SESSION", "0f2c1d9e-1111-2222-3333-444455556666")]),
            Ok(Protocol::Sixel)
        );
        assert_eq!(detect(&[("TERM", "foot")]), Ok(Protocol::Sixel));
        assert_eq!(detect(&[("TERM", "foot-extra")]), Ok(Protocol::Sixel));
    }

    #[test]
    fn multiplexers_win_over_the_outer_terminal() {
        let e = detect(&[
            ("TERM", "tmux-256color"),
            ("TERM_PROGRAM", "tmux"),
            ("TMUX", "/tmp/tmux-1000/default,1234,0"),
            ("GHOSTTY_RESOURCES_DIR", "/usr/share/ghostty"),
        ])
        .unwrap_err();
        assert!(e.contains("tmux"), "{e}");
        let e = detect(&[("TERM", "screen-256color"), ("KITTY_WINDOW_ID", "1")]).unwrap_err();
        assert!(e.contains("TERM=screen-256color"), "{e}");
    }

    #[test]
    fn unknown_terminals_are_named() {
        let e = detect(&[("TERM", "alacritty")]).unwrap_err();
        assert!(e.contains("TERM=alacritty"), "{e}");
        let e = detect(&[("TERM", "xterm-256color"), ("TERM_PROGRAM", "vscode")]).unwrap_err();
        assert!(e.contains("TERM_PROGRAM=vscode"), "{e}");
        let e = detect(&[]).unwrap_err();
        assert!(e.contains("TERM is unset"), "{e}");
        // An empty value counts as unset.
        assert!(detect(&[("TMUX", ""), ("TERM", "xterm-kitty")]).is_ok());
    }

    /// Splits a byte stream into the payloads of its `ESC _ G ... ESC \` commands.
    fn kitty_commands(bytes: &[u8]) -> Vec<(String, String)> {
        let s = std::str::from_utf8(bytes).unwrap();
        let body = s.strip_suffix("\r\n").expect("ends with a line break");
        body.split_terminator("\x1b\\")
            .map(|cmd| {
                let cmd = cmd.strip_prefix("\x1b_G").expect("APC G");
                let (keys, payload) = cmd.split_once(';').unwrap();
                (keys.to_owned(), payload.to_owned())
            })
            .collect()
    }

    #[test]
    fn kitty_single_chunk_has_no_m_key() {
        let cmds = kitty_commands(&kitty(b"tiny"));
        assert_eq!(
            cmds,
            vec![("a=T,f=100,q=2".to_owned(), BASE64.encode(b"tiny"))]
        );
    }

    #[test]
    fn kitty_chunks_large_payloads() {
        // 7000 bytes -> 9336 base64 characters -> chunks of 4096, 4096, 1144.
        let payload: Vec<u8> = (0..7000u32).map(|i| (i * 7 % 251) as u8).collect();
        let cmds = kitty_commands(&kitty(&payload));
        let keys: Vec<&str> = cmds.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["a=T,f=100,q=2,m=1", "m=1", "m=0"]);
        let lens: Vec<usize> = cmds.iter().map(|(_, p)| p.len()).collect();
        assert_eq!(lens, [4096, 4096, 1144]);
        let joined: String = cmds.iter().map(|(_, p)| p.as_str()).collect();
        assert_eq!(BASE64.decode(joined).unwrap(), payload);
    }

    #[test]
    fn kitty_exact_multiple_of_chunk_size() {
        // 3072 bytes -> exactly 4096 base64 characters, one chunk.
        let cmds = kitty_commands(&kitty(&[0u8; 3072]));
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].0, "a=T,f=100,q=2");
    }

    #[test]
    fn iterm2_framing() {
        let out = iterm2(b"PNGDATA");
        assert_eq!(
            out,
            format!(
                "\x1b]1337;File=inline=1;size=7;preserveAspectRatio=1:{}\x07\r\n",
                BASE64.encode(b"PNGDATA")
            )
            .into_bytes()
        );
    }

    #[test]
    fn encode_uses_the_png_for_kitty_and_pixels_for_sixel() {
        let img = Image::from_rgb(1, 1, vec![255, 0, 0]).unwrap();
        let k = encode(&img, Protocol::Kitty).unwrap();
        assert!(
            k.starts_with(b"\x1b_Ga=T,f=100,q=2;iVBORw0KGgo"),
            "PNG magic in base64"
        );
        let s = encode(&img, Protocol::Sixel).unwrap();
        assert!(s.starts_with(b"\x1bP"));
    }
}
