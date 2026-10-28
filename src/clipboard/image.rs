//! Clipboard bitmaps (`CF_DIB`): reading the header, a small thumbnail
//! sampled straight from the pixels (no full decode), and a DIB built back
//! from a thumbnail. Pure byte work, tested without the clipboard.

/// The largest thumbnail kept: the card's 92 px strip at 2× is enough.
pub const THUMB_MAX_W: u32 = 184;
pub const THUMB_MAX_H: u32 = 128;

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;

/// A small top-down BGRA picture, opaque.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumb {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

impl Thumb {
    pub fn bytes(&self) -> usize {
        self.bgra.len()
    }
}

/// What the header of a DIB says, for the formats read (24 or 32 bpp,
/// uncompressed or standard bit fields).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DibInfo {
    pub w: u32,
    pub h: u32,
    pub top_down: bool,
    pub bpp: u32,
    /// Where the pixels start in the packed DIB.
    pub offset: usize,
    pub stride: usize,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn i32_at(b: &[u8], i: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// The header of a packed DIB (`BITMAPINFOHEADER`, V4 or V5 and its
/// pixels); `None` for anything this does not read or a buffer too short.
pub fn dib_info(b: &[u8]) -> Option<DibInfo> {
    let size = u32_at(b, 0)? as usize;
    if !(40..=124).contains(&size) {
        return None;
    }
    let w = i32_at(b, 4)?;
    let h = i32_at(b, 8)?;
    let bpp = u32::from(u16_at(b, 14)?);
    let compression = u32_at(b, 16)?;
    if w <= 0 || h == 0 || !(bpp == 24 || bpp == 32) {
        return None;
    }
    let mut offset = size;
    match compression {
        BI_RGB => {}
        BI_BITFIELDS if bpp == 32 => {
            // The masks follow a plain header, or sit inside a V4/V5 one.
            let masks = (u32_at(b, 40)?, u32_at(b, 44)?, u32_at(b, 48)?);
            if masks != (0x00FF_0000, 0x0000_FF00, 0x0000_00FF) {
                return None;
            }
            if size == 40 {
                offset += 12;
            }
        }
        _ => return None,
    }
    let colors = u32_at(b, 32)? as usize;
    offset += colors.min(256) * 4;
    let (w, top_down) = (w as u32, h < 0);
    let h = h.unsigned_abs();
    let stride = (w as usize * bpp as usize).div_ceil(32) * 4;
    let end = offset.checked_add(stride.checked_mul(h as usize)?)?;
    if end > b.len() {
        return None;
    }
    Some(DibInfo {
        w,
        h,
        top_down,
        bpp,
        offset,
        stride,
    })
}

/// The thumbnail's size for a `w` × `h` image: scaled down to fit
/// `THUMB_MAX_W` × `THUMB_MAX_H`, never up.
pub fn thumb_size(w: u32, h: u32) -> (u32, u32) {
    fit_size(w, h, THUMB_MAX_W, THUMB_MAX_H)
}

/// `w` × `h` scaled down to fit `max_w` × `max_h`, never up.
fn fit_size(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let k = (max_w as f32 / w as f32)
        .min(max_h as f32 / h as f32)
        .min(1.0);
    let s = |v: u32| ((v as f32 * k).round() as u32).max(1);
    (s(w), s(h))
}

/// A thumbnail of the DIB: each pixel the average of four samples of the
/// area it covers, alpha forced opaque (clipboard bitmaps rarely carry a
/// real alpha channel).
pub fn thumbnail(b: &[u8], info: &DibInfo) -> Thumb {
    thumbnail_fit(b, info, THUMB_MAX_W, THUMB_MAX_H)
}

/// `thumbnail` at most `max_w` × `max_h` (the screenshot preview's larger
/// one, made from the full DIB).
pub fn thumbnail_fit(b: &[u8], info: &DibInfo, max_w: u32, max_h: u32) -> Thumb {
    let (tw, th) = fit_size(info.w, info.h, max_w, max_h);
    let bytes_pp = (info.bpp / 8) as usize;
    let pixel = |x: u32, y: u32| -> [u32; 3] {
        let row = if info.top_down { y } else { info.h - 1 - y };
        let i = info.offset + row as usize * info.stride + x as usize * bytes_pp;
        [u32::from(b[i]), u32::from(b[i + 1]), u32::from(b[i + 2])]
    };
    let mut bgra = Vec::with_capacity((tw * th * 4) as usize);
    let (sx, sy) = (info.w as f32 / tw as f32, info.h as f32 / th as f32);
    for y in 0..th {
        for x in 0..tw {
            let mut sum = [0u32; 3];
            for (fx, fy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let px = (((x as f32 + fx) * sx) as u32).min(info.w - 1);
                let py = (((y as f32 + fy) * sy) as u32).min(info.h - 1);
                let p = pixel(px, py);
                for c in 0..3 {
                    sum[c] += p[c];
                }
            }
            bgra.extend([
                (sum[0] / 4) as u8,
                (sum[1] / 4) as u8,
                (sum[2] / 4) as u8,
                255,
            ]);
        }
    }
    Thumb { w: tw, h: th, bgra }
}

/// A packed 32 bpp bottom-up DIB of `t`, for `CF_DIB`.
pub fn dib_from_thumb(t: &Thumb) -> Vec<u8> {
    let mut out = Vec::with_capacity(40 + t.bgra.len());
    out.extend(40u32.to_le_bytes());
    out.extend((t.w as i32).to_le_bytes());
    out.extend((t.h as i32).to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend(BI_RGB.to_le_bytes());
    out.extend((t.bgra.len() as u32).to_le_bytes());
    out.extend([0u8; 16]);
    let row = t.w as usize * 4;
    for y in (0..t.h as usize).rev() {
        out.extend(&t.bgra[y * row..(y + 1) * row]);
    }
    out
}

/// A fingerprint of every byte of an image (its length, then 8 bytes at a
/// time, multiply-rotate mixing): equal images give equal values, and a
/// one-pixel change gives another, for de-duplication. A 33 MB 4K
/// screenshot takes a few milliseconds.
pub fn fingerprint(b: &[u8]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mix = |h: u64, word: u64| (h.rotate_left(23) ^ word).wrapping_mul(K);
    let mut h = mix(0xcbf2_9ce4_8422_2325, b.len() as u64);
    let (words, rest) = b.as_chunks::<8>();
    for w in words {
        h = mix(h, u64::from_le_bytes(*w));
    }
    let mut tail = [0u8; 8];
    tail[..rest.len()].copy_from_slice(rest);
    mix(h, u64::from_le_bytes(tail))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A `w` × `h` 24 bpp bottom-up DIB: blue left half, red right half.
    pub fn dib24(w: u32, h: u32) -> Vec<u8> {
        let stride = (w as usize * 24).div_ceil(32) * 4;
        let mut out = Vec::new();
        out.extend(40u32.to_le_bytes());
        out.extend((w as i32).to_le_bytes());
        out.extend((h as i32).to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(24u16.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(((stride * h as usize) as u32).to_le_bytes());
        out.extend([0u8; 16]);
        for _ in 0..h {
            let mut row = Vec::new();
            for x in 0..w {
                if x < w / 2 {
                    row.extend([255, 0, 0]);
                } else {
                    row.extend([0, 0, 255]);
                }
            }
            row.resize(stride, 0);
            out.extend(row);
        }
        out
    }

    #[test]
    fn reads_the_header_and_rejects_junk() {
        let d = dib24(1280, 720);
        let info = dib_info(&d).unwrap();
        assert_eq!(
            (info.w, info.h, info.bpp, info.top_down),
            (1280, 720, 24, false)
        );
        assert_eq!(info.offset, 40);
        assert_eq!(dib_info(&d[..100]), None);
        assert_eq!(dib_info(b"garbage"), None);
        let mut eight = d.clone();
        eight[14] = 8;
        assert_eq!(dib_info(&eight), None);
    }

    #[test]
    fn thumbnail_fits_and_keeps_the_picture() {
        let d = dib24(1280, 720);
        let info = dib_info(&d).unwrap();
        let t = thumbnail(&d, &info);
        assert_eq!((t.w, t.h), (184, 104));
        assert_eq!(t.bgra.len(), 184 * 104 * 4);
        // Left is blue, right is red (BGRA order), all opaque.
        assert_eq!(&t.bgra[..4], &[255, 0, 0, 255]);
        let last = t.bgra.len() - 4;
        assert_eq!(&t.bgra[last..], &[0, 0, 255, 255]);
        // The preview's larger one, from the same pixels.
        let big = thumbnail_fit(&d, &info, 460, 272);
        assert_eq!((big.w, big.h), (460, 259));
        assert_eq!(&big.bgra[..4], &[255, 0, 0, 255]);
        // Never scaled up.
        assert_eq!(thumb_size(40, 30), (40, 30));
        assert_eq!(thumb_size(100, 1000), (13, 128));
    }

    #[test]
    fn a_thumbnail_dib_reads_back() {
        let t = Thumb {
            w: 3,
            h: 2,
            bgra: (0..24).map(|i| i as u8).collect(),
        };
        let d = dib_from_thumb(&t);
        let info = dib_info(&d).unwrap();
        assert_eq!((info.w, info.h, info.bpp), (3, 2, 32));
        // Bottom-up: the first stored row is the thumbnail's last.
        assert_eq!(&d[40..52], &t.bgra[12..24]);
        assert_eq!(thumbnail(&d, &info).w, 3);
    }

    #[test]
    fn fingerprints_tell_images_apart() {
        let a = dib24(64, 32);
        let mut b = a.clone();
        assert_eq!(fingerprint(&a), fingerprint(&b));
        b[40] = 1;
        assert_ne!(fingerprint(&a), fingerprint(&b));
        assert_ne!(fingerprint(&a), fingerprint(&dib24(64, 33)));
        // A one-pixel change anywhere (a typo fixed, a caret moved) is a new
        // image, not the old card moving to the front.
        for at in [100, 1_001, 4_000, a.len() - 1] {
            let mut c = a.clone();
            c[at] ^= 0x10;
            assert_ne!(fingerprint(&a), fingerprint(&c), "byte {at}");
        }
    }
}
