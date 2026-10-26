//! The Studio typefaces (Fraunces Italic, DM Sans, DM Mono), embedded in the
//! exe as RCDATA resources and loaded from memory: for DirectWrite as a
//! custom font collection, for the remaining GDI windows as process-private
//! fonts. When a face is missing (no resource, a load error), every caller
//! falls back to the Segoe / Cascadia families the app used before.

use std::sync::OnceLock;

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_FEATURE, DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE, DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_TEXT_RANGE, IDWriteFactory, IDWriteFactory5, IDWriteFontCollection,
    IDWriteFontCollection1, IDWriteInMemoryFontFileLoader, IDWriteTextFormat, IDWriteTextLayout,
    IDWriteTypography,
};
use windows::Win32::Graphics::Gdi::{
    AddFontMemResourceEx, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_QUALITY,
    DeleteObject, FW_NORMAL, GetDC, GetTextFaceW, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, ReleaseDC,
    SelectObject,
};
use windows::Win32::System::LibraryLoader::{
    FindResourceW, LoadResource, LockResource, SizeofResource,
};
use windows::core::{BOOL, HSTRING, IUnknown, Interface, PCWSTR, Result, w};

pub const FRAUNCES: &str = "Fraunces";
pub const DM_SANS: &str = "DM Sans";
pub const DM_MONO: &str = "DM Mono";

/// RCDATA ids in `app.rc`: Fraunces Italic 400/500, DM Sans 400/500/600,
/// DM Mono 400/500.
pub const FONT_RESOURCE_IDS: [u16; 7] = [201, 202, 203, 204, 205, 206, 207];

/// `RT_RCDATA` (`MAKEINTRESOURCE(10)`).
const RT_RCDATA: PCWSTR = PCWSTR(10 as _);

const TEXT_FALLBACK: &[&str] = &["Segoe UI Variable Text", "Segoe UI Variable", "Segoe UI"];
const DISPLAY_FALLBACK: &[&str] = &["Segoe UI Variable Display", "Segoe UI Variable", "Segoe UI"];
const MONO_FALLBACK: &[&str] = &["Cascadia Mono", "Consolas"];

/// What a piece of text is set in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Body text and captions: DM Sans.
    Text,
    /// Headings and the big clock: Fraunces Italic (16 px and up only).
    Display,
    /// Code: DM Mono.
    Mono,
}

/// Which embedded families resolved in the custom collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Loaded {
    pub fraunces: bool,
    pub dm_sans: bool,
    pub dm_mono: bool,
}

/// The family a role is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub family: &'static str,
    /// Look the family up in the embedded collection (else the system one).
    pub embedded: bool,
    pub italic: bool,
}

/// The embedded family for `role` when it loaded, else the first of the
/// fallback chain that `installed` reports (the chain's last name when none
/// is, leaving the rest to DirectWrite's own font fallback).
pub fn choose(role: Role, loaded: Loaded, installed: impl Fn(&str) -> bool) -> Choice {
    let (family, ok, fallback, italic) = match role {
        Role::Text => (DM_SANS, loaded.dm_sans, TEXT_FALLBACK, false),
        Role::Display => (FRAUNCES, loaded.fraunces, DISPLAY_FALLBACK, true),
        Role::Mono => (DM_MONO, loaded.dm_mono, MONO_FALLBACK, false),
    };
    if ok {
        return Choice {
            family,
            embedded: true,
            italic,
        };
    }
    let last = fallback[fallback.len() - 1];
    Choice {
        family: fallback
            .iter()
            .copied()
            .find(|f| installed(f))
            .unwrap_or(last),
        embedded: false,
        italic,
    }
}

// ---- Resources ----

/// The bytes of RCDATA resource `id` in the exe, `None` when it is absent.
fn resource(id: u16) -> Option<&'static [u8]> {
    unsafe {
        let info = FindResourceW(None, PCWSTR(id as usize as *const u16), RT_RCDATA);
        if info.is_invalid() {
            return None;
        }
        let size = SizeofResource(None, info);
        let handle = LoadResource(None, info).ok()?;
        let data = LockResource(handle);
        if data.is_null() || size == 0 {
            return None;
        }
        // The exe's resources stay mapped for the life of the process.
        Some(std::slice::from_raw_parts(data.cast::<u8>(), size as usize))
    }
}

/// The resources among `ids` that exist; missing ones are skipped.
fn resources(ids: &[u16]) -> Vec<&'static [u8]> {
    ids.iter().filter_map(|&id| resource(id)).collect()
}

/// The embedded font files, read once.
fn blobs() -> &'static [&'static [u8]] {
    static BLOBS: OnceLock<Vec<&'static [u8]>> = OnceLock::new();
    BLOBS.get_or_init(|| resources(&FONT_RESOURCE_IDS))
}

// ---- DirectWrite ----

/// DirectWrite objects made on the shared factory, kept for the whole process.
/// The shared factory and the immutable collections it hands out are
/// thread-safe; in practice they are only used from the UI thread.
struct Shared<T>(T);
// SAFETY: see above; the wrapped objects are never mutated after creation.
unsafe impl<T> Send for Shared<T> {}
unsafe impl<T> Sync for Shared<T> {}

/// The in-memory loader, registered once on the shared factory and never
/// unregistered: text formats made from its fonts may live until exit.
fn loader(factory: &IDWriteFactory5) -> Option<&'static IDWriteInMemoryFontFileLoader> {
    static LOADER: OnceLock<Shared<Option<IDWriteInMemoryFontFileLoader>>> = OnceLock::new();
    let loader = LOADER.get_or_init(|| unsafe {
        let made = factory
            .CreateInMemoryFontFileLoader()
            .ok()
            .filter(|loader| factory.RegisterFontFileLoader(loader).is_ok());
        Shared(made)
    });
    loader.0.as_ref()
}

/// A collection of the fonts in `files` (raw OpenType bytes, copied by
/// DirectWrite), `None` when not one of them could be added.
fn build_collection(dwrite: &IDWriteFactory, files: &[&[u8]]) -> Option<IDWriteFontCollection1> {
    let factory: IDWriteFactory5 = dwrite.cast().ok()?;
    let loader = loader(&factory)?;
    unsafe {
        let builder = factory.CreateFontSetBuilder().ok()?;
        let mut added = 0;
        for file in files {
            let Ok(len) = u32::try_from(file.len()) else {
                continue;
            };
            let Ok(reference) = loader.CreateInMemoryFontFileReference(
                dwrite,
                file.as_ptr().cast(),
                len,
                None::<&IUnknown>,
            ) else {
                continue;
            };
            // DWRITE_E_FILEFORMAT for anything that is not an OpenType file.
            if builder.AddFontFile(&reference).is_ok() {
                added += 1;
            }
        }
        if added == 0 {
            return None;
        }
        let set = builder.CreateFontSet().ok()?;
        factory.CreateFontCollectionFromFontSet(&set).ok()
    }
}

fn has_family(collection: &IDWriteFontCollection, name: &str) -> bool {
    let (mut index, mut exists) = (0u32, BOOL(0));
    let found = unsafe { collection.FindFamilyName(&HSTRING::from(name), &mut index, &mut exists) };
    found.is_ok() && exists.as_bool()
}

/// The embedded collection and which families resolved in it.
pub struct Embedded {
    collection: Option<IDWriteFontCollection>,
    pub loaded: Loaded,
}

impl Embedded {
    fn build(dwrite: &IDWriteFactory, files: &[&[u8]]) -> Self {
        let collection = build_collection(dwrite, files).and_then(|c| c.cast().ok());
        let has = |name| collection.as_ref().is_some_and(|c| has_family(c, name));
        let loaded = Loaded {
            fraunces: has(FRAUNCES),
            dm_sans: has(DM_SANS),
            dm_mono: has(DM_MONO),
        };
        Self { collection, loaded }
    }

    pub fn collection(&self) -> Option<&IDWriteFontCollection> {
        self.collection.as_ref()
    }
}

/// The embedded fonts on the shared factory `dwrite`, loaded on first use.
pub fn embedded(dwrite: &IDWriteFactory) -> &'static Embedded {
    static EMBEDDED: OnceLock<Shared<Embedded>> = OnceLock::new();
    &EMBEDDED
        .get_or_init(|| Shared(Embedded::build(dwrite, blobs())))
        .0
}

/// `name` is in the system font collection.
pub fn system_has(dwrite: &IDWriteFactory, name: &str) -> bool {
    unsafe {
        let mut collection = None;
        if dwrite
            .GetSystemFontCollection(&mut collection, false)
            .is_err()
        {
            return false;
        }
        collection.is_some_and(|c| has_family(&c, name))
    }
}

/// A family ready for `CreateTextFormat`.
pub struct Face {
    pub family: HSTRING,
    /// `None`: the system collection.
    pub collection: Option<IDWriteFontCollection>,
    pub style: DWRITE_FONT_STYLE,
}

impl Face {
    /// The face `role` is drawn in, embedded or fallback.
    pub fn for_role(dwrite: &IDWriteFactory, role: Role) -> Face {
        let fonts = embedded(dwrite);
        let choice = choose(role, fonts.loaded, |name| system_has(dwrite, name));
        Face {
            family: HSTRING::from(choice.family),
            collection: if choice.embedded {
                fonts.collection().cloned()
            } else {
                None
            },
            style: if choice.italic {
                DWRITE_FONT_STYLE_ITALIC
            } else {
                DWRITE_FONT_STYLE_NORMAL
            },
        }
    }

    /// A system family (icon fonts).
    pub fn system(family: PCWSTR) -> Face {
        Face {
            family: HSTRING::from_wide(unsafe { family.as_wide() }),
            collection: None,
            style: DWRITE_FONT_STYLE_NORMAL,
        }
    }

    pub fn format(
        &self,
        dwrite: &IDWriteFactory,
        weight: DWRITE_FONT_WEIGHT,
        size: f32,
    ) -> Result<IDWriteTextFormat> {
        unsafe {
            dwrite.CreateTextFormat(
                &self.family,
                self.collection.as_ref(),
                weight,
                self.style,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-us"),
            )
        }
    }
}

/// Tabular (fixed-width) figures, so times and counters do not jitter.
/// Made once per renderer and shared by every layout.
pub fn tabular_figures(dwrite: &IDWriteFactory) -> Result<IDWriteTypography> {
    unsafe {
        let typography = dwrite.CreateTypography()?;
        typography.AddFontFeature(DWRITE_FONT_FEATURE {
            nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
            parameter: 1,
        })?;
        Ok(typography)
    }
}

/// `text` laid out in a `width` × `height` box with `format` (its alignment,
/// wrapping and trimming) and `typography` over the whole string. The same
/// box `DrawText` would use, but able to carry OpenType features.
pub fn text_layout(
    dwrite: &IDWriteFactory,
    text: &[u16],
    format: &IDWriteTextFormat,
    width: f32,
    height: f32,
    typography: &IDWriteTypography,
) -> Result<IDWriteTextLayout> {
    unsafe {
        let layout = dwrite.CreateTextLayout(text, format, width.max(0.0), height.max(0.0))?;
        layout.SetTypography(
            typography,
            DWRITE_TEXT_RANGE {
                startPosition: 0,
                length: text.len() as u32,
            },
        )?;
        Ok(layout)
    }
}

// ---- GDI ----

/// Which embedded families GDI can use after `register_gdi`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GdiFonts {
    pub dm_sans: bool,
    pub dm_mono: bool,
}

static GDI: OnceLock<GdiFonts> = OnceLock::new();

/// Adds the embedded fonts to this process's GDI font table. Call once at
/// startup, before any window is created; later calls return the first result.
pub fn register_gdi() -> GdiFonts {
    *GDI.get_or_init(|| {
        let files = blobs();
        let mut installed = 0u32;
        for file in files {
            let Ok(len) = u32::try_from(file.len()) else {
                continue;
            };
            let mut count = 0u32;
            // GDI writes the font count through this pointer (the binding
            // types it const). The handle is never released: the fonts stay
            // for the process.
            let count_out = (&raw mut count).cast_const();
            let handle =
                unsafe { AddFontMemResourceEx(file.as_ptr().cast(), len, None, count_out) };
            if handle.is_invalid() {
                eprintln!("dynamic-island: cannot register an embedded font with GDI");
            } else {
                installed += count;
            }
        }
        if installed == 0 {
            return GdiFonts::default();
        }
        let fonts = GdiFonts {
            dm_sans: gdi_has(DM_SANS),
            dm_mono: gdi_has(DM_MONO),
        };
        if !fonts.dm_sans || !fonts.dm_mono {
            eprintln!("dynamic-island: embedded fonts incomplete for GDI: {fonts:?}");
        }
        fonts
    })
}

/// What `register_gdi` found (nothing before it ran).
pub fn gdi() -> GdiFonts {
    GDI.get().copied().unwrap_or_default()
}

/// The GDI face for UI text: DM Sans when registered, else Segoe UI.
pub fn gdi_text_face() -> PCWSTR {
    if gdi().dm_sans {
        w!("DM Sans")
    } else {
        w!("Segoe UI")
    }
}

/// GDI resolves `name` to that face rather than substituting another.
fn gdi_has(name: &str) -> bool {
    unsafe {
        let font = CreateFontW(
            -13,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY,
            0,
            &HSTRING::from(name),
        );
        if font.is_invalid() {
            return false;
        }
        let got = gdi_face(font);
        let _ = DeleteObject(HGDIOBJ(font.0));
        got.eq_ignore_ascii_case(name)
    }
}

/// The face GDI actually picked for `font`.
pub fn gdi_face(font: HFONT) -> String {
    unsafe {
        let dc = GetDC(None);
        let old = SelectObject(dc, HGDIOBJ(font.0));
        let mut name = [0u16; 64];
        let n = GetTextFaceW(dc, Some(&mut name)).max(1) as usize - 1;
        SelectObject(dc, old);
        ReleaseDC(None, dc);
        String::from_utf16_lossy(&name[..n.min(name.len())])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::DirectWrite::{
        DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_WEIGHT_NORMAL, DWriteCreateFactory,
    };

    fn factory() -> IDWriteFactory {
        unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).expect("shared factory") }
    }

    #[test]
    fn fallbacks_when_nothing_loaded() {
        let none = Loaded::default();
        let all = |_: &str| true;
        let text = choose(Role::Text, none, all);
        assert_eq!(text.family, "Segoe UI Variable Text");
        assert!(!text.embedded && !text.italic);
        let display = choose(Role::Display, none, all);
        assert_eq!(display.family, "Segoe UI Variable Display");
        assert!(!display.embedded && display.italic);
        assert_eq!(choose(Role::Mono, none, all).family, "Cascadia Mono");
        // Older systems: the chain moves on, ending on its last name.
        let only_segoe = |f: &str| f == "Segoe UI";
        assert_eq!(choose(Role::Text, none, only_segoe).family, "Segoe UI");
        assert_eq!(choose(Role::Display, none, only_segoe).family, "Segoe UI");
        assert_eq!(choose(Role::Mono, none, |_| false).family, "Consolas");
    }

    #[test]
    fn embedded_names_when_loaded() {
        let loaded = Loaded {
            fraunces: true,
            dm_sans: true,
            dm_mono: true,
        };
        let pick = |role| choose(role, loaded, |_| true);
        assert_eq!(
            pick(Role::Text),
            Choice {
                family: DM_SANS,
                embedded: true,
                italic: false
            }
        );
        assert_eq!(
            pick(Role::Display),
            Choice {
                family: FRAUNCES,
                embedded: true,
                italic: true
            }
        );
        assert_eq!(pick(Role::Mono).family, DM_MONO);
        // One family missing falls back alone.
        let partial = Loaded {
            dm_sans: false,
            ..loaded
        };
        let text = choose(Role::Text, partial, |_| true);
        assert!(!text.embedded);
        assert!(choose(Role::Display, partial, |_| true).embedded);
    }

    #[test]
    fn garbage_bytes_build_nothing() {
        let dwrite = factory();
        let junk = [0x5Au8; 512];
        assert!(build_collection(&dwrite, &[&junk, &[]]).is_none());
        let fonts = Embedded::build(&dwrite, &[&junk]);
        assert_eq!(fonts.loaded, Loaded::default());
        assert!(fonts.collection().is_none());
        assert!(build_collection(&dwrite, &[]).is_none());
    }

    #[test]
    fn system_font_bytes_resolve_in_the_custom_collection() {
        let path = std::path::Path::new(r"C:\Windows\Fonts\consola.ttf");
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("skipped: {} not readable", path.display());
            return;
        };
        let dwrite = factory();
        // Built twice: the loader is registered once and reused.
        for _ in 0..2 {
            let collection = build_collection(&dwrite, &[&bytes]).expect("collection");
            let collection: IDWriteFontCollection = collection.cast().expect("base interface");
            assert!(has_family(&collection, "Consolas"));
            assert!(!has_family(&collection, DM_SANS));
            let face = Face {
                family: HSTRING::from("Consolas"),
                collection: Some(collection),
                style: DWRITE_FONT_STYLE_NORMAL,
            };
            let format = face
                .format(&dwrite, DWRITE_FONT_WEIGHT_NORMAL, 11.0)
                .expect("format");
            let wide: Vec<u16> = "12:34".encode_utf16().collect();
            let typography = tabular_figures(&dwrite).expect("typography");
            text_layout(&dwrite, &wide, &format, 100.0, 20.0, &typography).expect("layout");
        }
    }

    #[test]
    fn missing_resource_ids_are_skipped() {
        assert!(resource(65000).is_none());
        assert!(resources(&[65000, 65001, 1]).is_empty());
        // Whatever this binary embeds, reading the real ids never panics.
        assert!(resources(&FONT_RESOURCE_IDS).len() <= FONT_RESOURCE_IDS.len());
    }
}
