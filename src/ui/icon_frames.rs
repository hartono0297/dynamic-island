//! Guards `assets/icon.ico`: it keeps its nine sizes and every frame decodes
//! (through WIC, the same decoder family the shell uses) to the same pixels.
//! The hashes were taken from the v3.5.0 icon, whose small frames were
//! uncompressed DIBs; the PNG-compressed frames of the size-trimmed icon
//! must decode to exactly those pixels. See `docs/memory/exe-size.md`.

use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::core::HSTRING;

/// (size, FNV-1a 64 of the decoded 32 bpp BGRA pixels, top row first).
const GOLDEN: [(u32, u64); 9] = [
    (16, 429793334160563556),
    (20, 13104606106289928688),
    (24, 9229188787522462981),
    (32, 16875175387177808563),
    (40, 17148844232472308440),
    (48, 3380694787198623212),
    (64, 4054360817366831056),
    (128, 9444251138123922695),
    (256, 13771041625169929025),
];

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

/// Every frame of the icon file as (width, height, hash of its BGRA pixels).
fn frames(path: &str) -> Vec<(u32, u32, u64)> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let wic: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .expect("WIC factory");
        let decoder = wic
            .CreateDecoderFromFilename(
                &HSTRING::from(path),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .expect("decode icon");
        let count = decoder.GetFrameCount().expect("frame count");
        let mut out = Vec::new();
        for i in 0..count {
            let frame = decoder.GetFrame(i).expect("frame");
            let (mut w, mut h) = (0u32, 0u32);
            frame.GetSize(&mut w, &mut h).expect("size");
            let converter = wic.CreateFormatConverter().expect("converter");
            converter
                .Initialize(
                    &frame,
                    &GUID_WICPixelFormat32bppBGRA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .expect("convert");
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            converter
                .CopyPixels(std::ptr::null(), w * 4, &mut pixels)
                .expect("pixels");
            out.push((w, h, fnv1a(&pixels)));
        }
        // WIC lists the frames largest first.
        out.sort_by_key(|f| f.0);
        out
    }
}

#[test]
fn icon_frames_decode_to_the_same_pixels() {
    let path = std::env::var("ICON_PATH").unwrap_or_else(|_| {
        format!("{}/assets/icon.ico", env!("CARGO_MANIFEST_DIR"))
    });
    let got = frames(&path);
    if std::env::var("ICON_DUMP").is_ok() {
        for (w, h, hash) in &got {
            println!("ICONFRAME {w} {h} {hash}");
        }
    }
    assert_eq!(got.len(), GOLDEN.len(), "frame count");
    for ((w, h, hash), (size, golden)) in got.iter().zip(GOLDEN) {
        assert_eq!((*w, *h), (size, size), "frame size");
        assert_eq!(*hash, golden, "{size} px frame decodes to other pixels");
    }
}
