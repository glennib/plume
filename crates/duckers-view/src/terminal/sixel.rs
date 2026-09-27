//! A small sixel encoder for 8-bit RGB images.
//!
//! Sixel images use a palette of colour registers; terminals commonly provide 256.
//! An image with at most 256 distinct colours is encoded exactly. Otherwise colours are grouped into
//! 4096 bins (4 bits per channel), the 256 most populated bins become the palette (each bin's mean colour),
//! and every pixel takes the palette entry nearest its bin's mean. Charts are mostly flat colours with
//! anti-aliased edges, which this keeps faithful; smooth gradients band.

use std::collections::HashMap;
use std::fmt::Write as _;

const MAX_COLORS: usize = 256;

/// The DCS sequence drawing `rgb` (`width * height * 3` bytes).
///
/// The introducer `ESC P 0;1;0 q` leaves unset pixels unchanged (`P2 = 1`), and the raster attributes
/// `"1;1;W;H` set square pixels and the image size.
pub fn encode(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    assert_eq!(rgb.len(), w * h * 3, "RGB buffer size");
    let (palette, indices) = quantize(rgb);

    let mut out = String::with_capacity(w * h / 2 + 64);
    write!(out, "\x1bP0;1;0q\"1;1;{w};{h}").unwrap();
    for (i, [r, g, b]) in palette.iter().enumerate() {
        let pct = |c: u8| (c as u32 * 100 + 127) / 255;
        write!(out, "#{i};2;{};{};{}", pct(*r), pct(*g), pct(*b)).unwrap();
    }

    // Column bit masks of the current six-row band, one row of `w` per colour register.
    let mut masks = vec![0u8; palette.len() * w];
    let mut used = vec![false; palette.len()];
    let mut order: Vec<usize> = Vec::with_capacity(palette.len());
    for top in (0..h).step_by(6) {
        if top > 0 {
            out.push('-');
        }
        order.clear();
        for dy in 0..(h - top).min(6) {
            let row = &indices[(top + dy) * w..(top + dy + 1) * w];
            for (x, &c) in row.iter().enumerate() {
                let c = c as usize;
                if !used[c] {
                    used[c] = true;
                    order.push(c);
                }
                masks[c * w + x] |= 1 << dy;
            }
        }
        for (n, &c) in order.iter().enumerate() {
            if n > 0 {
                out.push('$');
            }
            write!(out, "#{c}").unwrap();
            let line = &mut masks[c * w..(c + 1) * w];
            let end = line.iter().rposition(|&m| m != 0).map_or(0, |p| p + 1);
            push_runs(&mut out, &line[..end]);
            line.fill(0);
            used[c] = false;
        }
    }
    out.push_str("\x1b\\");
    out.into_bytes()
}

/// Appends sixel characters for `masks`, run-length encoding runs longer than three (`!n<char>`).
fn push_runs(out: &mut String, masks: &[u8]) {
    let mut i = 0;
    while i < masks.len() {
        let m = masks[i];
        let run = masks[i..].iter().take_while(|&&x| x == m).count();
        let ch = char::from(63 + m);
        if run > 3 {
            write!(out, "!{run}{ch}").unwrap();
        } else {
            for _ in 0..run {
                out.push(ch);
            }
        }
        i += run;
    }
}

/// A palette of at most 256 colours and the palette index of every pixel.
fn quantize(rgb: &[u8]) -> (Vec<[u8; 3]>, Vec<u8>) {
    if let Some(exact) = exact_palette(rgb) {
        return exact;
    }
    let bin =
        |p: &[u8]| ((p[0] as usize >> 4) << 8) | ((p[1] as usize >> 4) << 4) | (p[2] as usize >> 4);
    let mut count = vec![0u64; 4096];
    let mut sum = vec![[0u64; 3]; 4096];
    for p in rgb.as_chunks::<3>().0 {
        let b = bin(p);
        count[b] += 1;
        for k in 0..3 {
            sum[b][k] += p[k] as u64;
        }
    }
    let mean =
        |b: usize| -> [u8; 3] { std::array::from_fn(|k| (sum[b][k] / count[b].max(1)) as u8) };
    let mut bins: Vec<usize> = (0..4096).filter(|&b| count[b] > 0).collect();
    // Most populated first; ties by bin number, so the result is deterministic.
    bins.sort_by(|&a, &b| count[b].cmp(&count[a]).then(a.cmp(&b)));
    let palette: Vec<[u8; 3]> = bins.iter().take(MAX_COLORS).map(|&b| mean(b)).collect();
    let mut bin_to_index = vec![0u8; 4096];
    for &b in &bins {
        let m = mean(b);
        let dist = |c: &[u8; 3]| -> u32 {
            (0..3)
                .map(|k| (m[k] as i32 - c[k] as i32).pow(2) as u32)
                .sum()
        };
        let (idx, _) = palette
            .iter()
            .enumerate()
            .min_by_key(|(_, c)| dist(c))
            .expect("non-empty palette");
        bin_to_index[b] = idx as u8;
    }
    let indices = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| bin_to_index[bin(p)])
        .collect();
    (palette, indices)
}

/// The exact palette, in order of first appearance, if the image has at most 256 colours.
fn exact_palette(rgb: &[u8]) -> Option<(Vec<[u8; 3]>, Vec<u8>)> {
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut lookup: HashMap<[u8; 3], u8> = HashMap::new();
    let mut indices = Vec::with_capacity(rgb.len() / 3);
    let mut last: Option<([u8; 3], u8)> = None;
    for p in rgb.as_chunks::<3>().0 {
        let c = [p[0], p[1], p[2]];
        let idx = match last {
            Some((lc, li)) if lc == c => li,
            _ => match lookup.get(&c) {
                Some(&i) => i,
                None => {
                    if palette.len() == MAX_COLORS {
                        return None;
                    }
                    let i = palette.len() as u8;
                    palette.push(c);
                    lookup.insert(c, i);
                    i
                }
            },
        };
        last = Some((c, idx));
        indices.push(idx);
    }
    Some((palette, indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 3] = [255, 0, 0];
    const WHITE: [u8; 3] = [255, 255, 255];

    fn image(pixels: &[[u8; 3]]) -> Vec<u8> {
        pixels.iter().flatten().copied().collect()
    }

    #[test]
    fn two_bands_two_colours() {
        // 2 x 7: column 0 red, column 1 white for six rows, then a red row.
        let mut px = Vec::new();
        for _ in 0..6 {
            px.extend([RED, WHITE]);
        }
        px.extend([RED, RED]);
        let out = String::from_utf8(encode(2, 7, &image(&px))).unwrap();
        assert_eq!(
            out,
            "\x1bP0;1;0q\"1;1;2;7\
             #0;2;100;0;0#1;2;100;100;100\
             #0~$#1?~\
             -#0@@\
             \x1b\\"
        );
    }

    #[test]
    fn long_runs_are_compressed() {
        let px = vec![RED; 10];
        let out = String::from_utf8(encode(10, 1, &image(&px))).unwrap();
        assert!(out.ends_with("#0!10@\x1b\\"), "{out:?}");
        let px = vec![RED; 3];
        let out = String::from_utf8(encode(3, 1, &image(&px))).unwrap();
        assert!(out.ends_with("#0@@@\x1b\\"), "{out:?}");
    }

    #[test]
    fn trailing_empty_columns_are_dropped() {
        let px = [RED, WHITE, WHITE, WHITE, WHITE, WHITE];
        let out = String::from_utf8(encode(6, 1, &image(&px))).unwrap();
        assert!(out.contains("#0@$#1?!5@"), "{out:?}");
    }

    #[test]
    fn many_colours_are_reduced_to_256() {
        let (w, h) = (64u32, 64u32);
        let rgb: Vec<u8> = (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| [(x * 4) as u8, (y * 4) as u8, 200]))
            .collect();
        assert!(exact_palette(&rgb).is_none());
        let (palette, indices) = quantize(&rgb);
        assert!(palette.len() <= 256);
        assert_eq!(indices.len(), (w * h) as usize);
        // Each pixel maps to a colour within one 16-level bin of its own.
        for (p, &i) in rgb.as_chunks::<3>().0.iter().zip(&indices) {
            let c = palette[i as usize];
            for k in 0..3 {
                assert!((p[k] as i32 - c[k] as i32).abs() < 32, "{p:?} -> {c:?}");
            }
        }
        let out = encode(w, h, &rgb);
        assert!(out.starts_with(b"\x1bP0;1;0q\"1;1;64;64#0;2;"));
        assert!(out.ends_with(b"\x1b\\"));
    }

    #[test]
    fn output_is_printable_between_introducer_and_terminator() {
        let rgb: Vec<u8> = (0..30 * 13 * 3).map(|i| (i * 37 % 256) as u8).collect();
        let out = encode(30, 13, &rgb);
        let body = &out[2..out.len() - 2];
        assert!(body.iter().all(|&b| (0x20..0x7f).contains(&b)));
        // Three bands: 13 rows = 6 + 6 + 1.
        assert_eq!(body.iter().filter(|&&b| b == b'-').count(), 2);
    }
}
