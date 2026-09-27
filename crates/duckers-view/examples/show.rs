//! Shows a generated test image with a chosen viewer.
//!
//! ```text
//! cargo run --example show -- [auto|terminal|window|browser] [options]
//!
//!   --wait              block until the window is closed
//!   --protocol P        terminal only: force kitty, iterm2 or sixel instead of detecting it
//!   --out FILE          terminal only: write the escape sequence to FILE instead of the terminal
//!   --hold SECS         stay alive SECS seconds after showing, then exit (checks exit with windows open)
//!   --size WxH          image size, default 480x320
//!   --count N           show N images, default 1
//! ```
//!
//! Timings go to stderr, so they do not mix with the image when stdout is the terminal.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use duckers_view::terminal::{self, Protocol};
use duckers_view::{Image, ShowOptions, Viewer, show};

struct Args {
    viewer: Viewer,
    wait: bool,
    protocol: Option<Protocol>,
    out: Option<String>,
    hold: Option<f64>,
    size: (u32, u32),
    count: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        viewer: Viewer::Auto,
        wait: false,
        protocol: None,
        out: None,
        hold: None,
        size: (480, 320),
        count: 1,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--wait" => args.wait = true,
            "--protocol" => {
                args.protocol = Some(match value()?.as_str() {
                    "kitty" => Protocol::Kitty,
                    "iterm2" => Protocol::ITerm2,
                    "sixel" => Protocol::Sixel,
                    other => return Err(format!("unknown protocol {other}")),
                })
            }
            "--out" => args.out = Some(value()?),
            "--hold" => args.hold = Some(value()?.parse().map_err(|e| format!("--hold: {e}"))?),
            "--size" => {
                let v = value()?;
                let (w, h) = v.split_once('x').ok_or("--size wants WxH")?;
                args.size = (
                    w.parse().map_err(|e| format!("--size: {e}"))?,
                    h.parse().map_err(|e| format!("--size: {e}"))?,
                );
            }
            "--count" => args.count = value()?.parse().map_err(|e| format!("--count: {e}"))?,
            v if !v.starts_with('-') => args.viewer = v.parse().map_err(|e| format!("{e}"))?,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(args)
}

/// A chart-like test picture: a soft gradient, a frame, grid lines, and a red and a blue curve.
fn test_image(w: u32, h: u32, seed: u32) -> Image {
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    let mut put = |x: i64, y: i64, c: [u8; 3]| {
        if (0..w as i64).contains(&x) && (0..h as i64).contains(&y) {
            let i = ((y as u32 * w + x as u32) * 3) as usize;
            rgb[i..i + 3].copy_from_slice(&c);
        }
    };
    for y in 0..h {
        for x in 0..w {
            let g = 235 + (20 * x / w) as u8;
            put(x as i64, y as i64, [g, g, 255 - (30 * y / h) as u8]);
        }
    }
    for x in 0..w as i64 {
        for y in (0..h as i64).step_by(40) {
            put(x, y, [200, 200, 200]);
        }
        put(x, 0, [0, 0, 0]);
        put(x, h as i64 - 1, [0, 0, 0]);
    }
    for y in 0..h as i64 {
        for x in (0..w as i64).step_by(40) {
            put(x, y, [200, 200, 200]);
        }
        put(0, y, [0, 0, 0]);
        put(w as i64 - 1, y, [0, 0, 0]);
    }
    let phase = seed as f64 * 0.7;
    for x in 0..w as i64 {
        let t = x as f64 / w as f64 * std::f64::consts::TAU * 2.0;
        let ys = (h as f64 / 2.0 * (1.0 - 0.8 * (t + phase).sin())) as i64;
        let yc = (h as f64 / 2.0 * (1.0 - 0.6 * (t * 0.5 + phase).cos())) as i64;
        for d in -1..=1 {
            put(x, ys + d, [220, 30, 30]);
            put(x, yc + d, [30, 60, 220]);
        }
    }
    Image::from_rgb(w, h, rgb).expect("buffer matches size")
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("show: {e}");
            return ExitCode::from(2);
        }
    };
    let start = Instant::now();
    for i in 0..args.count {
        let image = test_image(args.size.0, args.size.1, i as u32);
        let t = Instant::now();
        let result = match (&args.out, args.protocol) {
            (Some(path), p) => {
                let p = p.unwrap_or(Protocol::Kitty);
                terminal::encode(&image, p)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()))
                    .map(|()| format!("wrote {p} sequence to {path}"))
            }
            (None, Some(p)) => terminal::show_with(&image, p)
                .map(|()| format!("terminal ({p})"))
                .map_err(|e| e.to_string()),
            (None, None) => show(
                &image,
                &ShowOptions {
                    viewer: args.viewer,
                    wait: args.wait,
                },
            )
            .map(|shown| format!("{shown:?}"))
            .map_err(|e| e.to_string()),
        };
        match result {
            Ok(what) => eprintln!("show: {what} in {:.1?}", t.elapsed()),
            Err(e) => {
                eprintln!("show: error: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Some(secs) = args.hold {
        std::thread::sleep(Duration::from_secs_f64(secs));
    }
    eprintln!("show: exiting after {:.1?}", start.elapsed());
    ExitCode::SUCCESS
}
