//! The Devices page's data and wording, with no Win32 in it: what a listed
//! flash drive, external disk or memory card is, how a drive is classified
//! from what Windows reports, the sentence for an eject Windows refused, the
//! sizes and subtitles the cards show, what changed between two scans and
//! the two notices. The scanner and the eject are `devices_win.rs`; the page
//! state is `app_devices.rs`.

use crate::i18n::{self, Key, Lang, text};
use crate::model::{Notice, NoticeKind};

/// What kind of drive a card is (its icon and its fallback name).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// A flash drive.
    Usb,
    /// An external disk (a USB or Thunderbolt enclosure, any fixed medium).
    Hdd,
    /// A memory card, in a reader.
    Sd,
}

/// How the device is connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    Usb2,
    Usb3,
    /// USB, speed unknown.
    Usb,
    CardReader,
    Thunderbolt,
    Other,
}

/// One volume with a drive letter on a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partition {
    pub letter: char,
    pub label: String,
    pub fs: String,
    /// Bytes; 0 while the capacity has not been read.
    pub total: u64,
    /// Free bytes, `None` while the capacity has not been read.
    pub free: Option<u64>,
}

/// A removable device as the page lists it: everything behind one ejectable
/// device node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// The instance ID of the ejectable device node: stable while it stays
    /// plugged in, and what Eject locates again.
    pub key: String,
    /// The vendor and product, else the volume's label; empty when the device
    /// gives neither, and then `display_name` names its kind when drawn.
    pub name: String,
    /// Vendor and product as the device reports them, for the details.
    pub model: String,
    pub kind: DeviceKind,
    pub bus: Bus,
    pub partitions: Vec<Partition>,
}

/// Why Windows refused an eject (`PNP_VETO_TYPE`, 0..=13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VetoKind {
    Unknown,
    LegacyDevice,
    PendingClose,
    WindowsApp,
    WindowsService,
    OutstandingOpen,
    Device,
    Driver,
    IllegalDeviceRequest,
    InsufficientPower,
    NonDisableable,
    LegacyDriver,
    InsufficientRights,
    /// Not a refusal: the device was gone already.
    AlreadyRemoved,
}

impl VetoKind {
    /// From a `PNP_VETO_TYPE` value; anything unknown is `Unknown`.
    pub fn from_raw(raw: i32) -> VetoKind {
        match raw {
            1 => VetoKind::LegacyDevice,
            2 => VetoKind::PendingClose,
            3 => VetoKind::WindowsApp,
            4 => VetoKind::WindowsService,
            5 => VetoKind::OutstandingOpen,
            6 => VetoKind::Device,
            7 => VetoKind::Driver,
            8 => VetoKind::IllegalDeviceRequest,
            9 => VetoKind::InsufficientPower,
            10 => VetoKind::NonDisableable,
            11 => VetoKind::LegacyDriver,
            12 => VetoKind::InsufficientRights,
            13 => VetoKind::AlreadyRemoved,
            _ => VetoKind::Unknown,
        }
    }
}

/// An eject Windows refused, with the name it gave (often empty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Veto {
    pub kind: VetoKind,
    pub name: String,
}

/// What the scanner thread reports (`Event::Devices`).
#[derive(Clone, Debug, PartialEq)]
pub enum DevicesEvent {
    /// A scan finished: the devices now plugged in, for request `seq`.
    Listed { seq: u64, devices: Vec<Device> },
    /// An eject finished: per device its key, its name when the worker knew
    /// one (never the key) and the outcome. A fresh `Listed` follows.
    Ejected {
        results: Vec<(String, Option<String>, Result<Ejected, Veto>)>,
    },
}

/// How an eject that did not fail ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ejected {
    /// Windows removed the device on request: safe to unplug.
    Safe,
    /// The device was gone already (unplugged, or its node no longer
    /// locates): the card goes, but nothing was ejected, so no "safe" notice.
    Gone,
}

// ---- Classifying ----

/// `STORAGE_BUS_TYPE` values the classification needs.
pub const BUS_TYPE_USB: u32 = 0x7;
pub const BUS_TYPE_SD: u32 = 0xC;
pub const BUS_TYPE_MMC: u32 = 0xD;

/// Whether a product string reads like a card reader: "card", "reader" or
/// "mmc" anywhere, or "sd" as a word of its own ("SD/MMC", "SDXC", not
/// "SanDisk"), in any case.
fn reads_like_card_reader(product: &str) -> bool {
    let lower = product.to_lowercase();
    if lower.contains("card") || lower.contains("reader") || lower.contains("mmc") {
        return true;
    }
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.starts_with("sd") || w == "microsd")
}

/// A card for a memory card (SD or MMC bus, or a USB reader named like one),
/// a flash drive for removable USB, a disk for the rest (fixed media in an
/// enclosure, NVMe or SATA behind a bridge).
pub fn classify(bus_type_raw: u32, removable_media: bool, product: &str) -> DeviceKind {
    match bus_type_raw {
        BUS_TYPE_SD | BUS_TYPE_MMC => DeviceKind::Sd,
        BUS_TYPE_USB if removable_media && reads_like_card_reader(product) => DeviceKind::Sd,
        BUS_TYPE_USB if removable_media => DeviceKind::Usb,
        _ => DeviceKind::Hdd,
    }
}

/// The name a device is listed under: its vendor and product, the vendor
/// not twice when the product already starts with it ("SanDisk" and
/// "SanDisk Ultra" read "SanDisk Ultra"). Empty when the device names
/// neither.
pub fn vendor_product_name(vendor: &str, product: &str) -> String {
    let (v, p) = (vendor.trim(), product.trim());
    if v.is_empty() {
        return p.to_owned();
    }
    if p.is_empty() {
        return v.to_owned();
    }
    let repeated =
        p.len() >= v.len() && p.is_char_boundary(v.len()) && p[..v.len()].eq_ignore_ascii_case(v);
    if repeated {
        p.to_owned()
    } else {
        format!("{v} {p}")
    }
}

// ---- Eject refusals ----

/// What Windows answered to an eject request: `CR_SUCCESS` with no veto is
/// `Safe`, a device that was gone already (veto `AlreadyRemoved`, whatever
/// the call returned) is `Gone`; anything else is a refusal with the kind and
/// the name Windows gave.
pub fn eject_outcome(succeeded: bool, veto_raw: i32, name: &str) -> Result<Ejected, Veto> {
    let kind = VetoKind::from_raw(veto_raw);
    match (succeeded, kind) {
        (_, VetoKind::AlreadyRemoved) => Ok(Ejected::Gone),
        (true, VetoKind::Unknown) => Ok(Ejected::Safe),
        _ => Err(Veto {
            kind,
            name: name.to_owned(),
        }),
    }
}

/// The program a veto names: the last path component without ".exe" (any
/// case), a name that is all capitals or all lowercase capitalised
/// ("C:\...\EXCEL.EXE" is "Excel"). `None` when nothing usable is left.
pub fn app_name(raw: &str) -> Option<String> {
    let last = raw.rsplit(['\\', '/']).next().unwrap_or(raw).trim();
    let stem = match last.len().checked_sub(4) {
        Some(at) if last.is_char_boundary(at) && last[at..].eq_ignore_ascii_case(".exe") => {
            &last[..at]
        }
        _ => last,
    }
    .trim();
    if stem.is_empty() {
        return None;
    }
    let letters = || stem.chars().filter(|c| c.is_alphabetic());
    let shouty = letters().all(char::is_uppercase) || letters().all(char::is_lowercase);
    if !shouty || letters().next().is_none() {
        return Some(stem.to_owned());
    }
    let mut chars = stem.chars();
    let first: String = chars
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect();
    Some(first + &chars.as_str().to_lowercase())
}

/// The title and the detail of the line a refused eject shows on its card.
/// Worked out when the card is drawn, so a language change re-renders it.
pub fn veto_message(lang: Lang, veto: &Veto) -> (String, String) {
    let title = text(lang, Key::VetoTitle).to_owned();
    let name = veto.name.trim();
    let detail = match veto.kind {
        // The only kind whose name is a program (`PNP_VETO_TYPE` remarks).
        VetoKind::WindowsApp => match app_name(name) {
            Some(app) if !name.is_empty() => i18n::veto_in_use_by(lang, &app),
            _ => text(lang, Key::VetoInUse).to_owned(),
        },
        // Their name is a device instance path: never shown.
        VetoKind::OutstandingOpen | VetoKind::PendingClose => text(lang, Key::VetoInUse).to_owned(),
        VetoKind::WindowsService if !name.is_empty() => i18n::veto_service(lang, name),
        VetoKind::WindowsService => text(lang, Key::VetoService).to_owned(),
        VetoKind::Device => text(lang, Key::VetoDevice).to_owned(),
        VetoKind::Driver | VetoKind::LegacyDriver | VetoKind::LegacyDevice => {
            text(lang, Key::VetoDriver).to_owned()
        }
        VetoKind::NonDisableable => text(lang, Key::VetoNotAllowed).to_owned(),
        VetoKind::InsufficientRights => text(lang, Key::VetoRights).to_owned(),
        VetoKind::InsufficientPower => text(lang, Key::VetoPower).to_owned(),
        // `AlreadyRemoved` is a success and never stored; the rest is
        // Windows saying no without saying why.
        VetoKind::Unknown | VetoKind::IllegalDeviceRequest | VetoKind::AlreadyRemoved => {
            text(lang, Key::VetoRefused).to_owned()
        }
    };
    (title, detail)
}

// ---- Sizes, subtitles and titles ----

const MIB: f64 = 1024.0 * 1024.0;
const GIB: f64 = MIB * 1024.0;

/// A size in binary units: "512 MB" under 1 GB, "18,4 GB" ("18.4 GB") under
/// 100 GB, "612 GB" up to 999 GB, "1,8 TB" from 1000 GB. Compared as shown,
/// so 99.96 GB reads "100 GB" and 999.96 GB reads "1,0 TB".
pub fn device_size(lang: Lang, bytes: u64) -> String {
    let mb = (bytes as f64 / MIB).round();
    if mb < 1024.0 {
        return format!("{mb:.0} MB");
    }
    let gb = bytes as f64 / GIB;
    if (gb * 10.0).round() < 1000.0 {
        return format!("{} GB", i18n::one_decimal(lang, gb));
    }
    if gb.round() < 1000.0 {
        return format!("{:.0} GB", gb);
    }
    format!("{} TB", i18n::one_decimal(lang, gb / 1024.0))
}

/// "USB 3", "USB 2", "USB" for a flash drive or disk on USB.
fn usb_speed(bus: Bus) -> &'static str {
    match bus {
        Bus::Usb3 => "USB 3",
        Bus::Usb2 => "USB 2",
        _ => "USB",
    }
}

/// The file system the partitions share, `None` when none is known or they
/// differ.
fn shared_fs(device: &Device) -> Option<&str> {
    let first = device.partitions.first()?.fs.as_str();
    (!first.is_empty() && device.partitions.iter().all(|p| p.fs == first)).then_some(first)
}

/// The line under a card's title: "Flashdisk USB 3 · exFAT",
/// "Disk eksternal USB · 2 partisi NTFS", "Pembaca kartu · FAT32".
pub fn subtitle(lang: Lang, device: &Device) -> String {
    let fs = shared_fs(device);
    let with_fs = |head: String| match fs {
        Some(fs) => format!("{head} · {fs}"),
        None => head,
    };
    match device.kind {
        DeviceKind::Usb => {
            let speed = usb_speed(device.bus);
            let head = match lang {
                Lang::Id => format!("Flashdisk {speed}"),
                // "USB 3" is "USB flash drive 3": the number follows.
                Lang::En => match speed.strip_prefix("USB ") {
                    Some(n) => format!("USB flash drive {n}"),
                    None => "USB flash drive".to_owned(),
                },
            };
            with_fs(head)
        }
        DeviceKind::Hdd => {
            let via = match device.bus {
                Bus::Thunderbolt => "Thunderbolt",
                Bus::Usb | Bus::Usb2 | Bus::Usb3 => "USB",
                Bus::CardReader | Bus::Other => "",
            };
            let head = match (lang, via) {
                (Lang::Id, "") => "Disk eksternal".to_owned(),
                (Lang::Id, via) => format!("Disk eksternal {via}"),
                (Lang::En, "") => "External disk".to_owned(),
                (Lang::En, via) => format!("External {via} disk"),
            };
            let n = device.partitions.len();
            if n > 1 {
                let counted = match (lang, fs) {
                    (Lang::Id, Some(fs)) => format!("{n} partisi {fs}"),
                    (Lang::Id, None) => format!("{n} partisi"),
                    (Lang::En, Some(fs)) => format!("{n} {fs} partitions"),
                    (Lang::En, None) => format!("{n} partitions"),
                };
                format!("{head} · {counted}")
            } else {
                with_fs(head)
            }
        }
        DeviceKind::Sd => with_fs(text(lang, Key::DeviceCardReader).to_owned()),
    }
}

/// The device's name, or its kind ("Flashdisk") when it has none: resolved
/// when drawn, so a language change re-renders it.
pub fn display_name(lang: Lang, device: &Device) -> String {
    if device.name.is_empty() {
        kind_name(lang, device.kind).to_owned()
    } else {
        device.name.clone()
    }
}

/// "SanDisk Ultra · E:", or "WD Elements · F:  G:" for several letters.
pub fn title_line(lang: Lang, device: &Device) -> String {
    let name = display_name(lang, device);
    let letters: Vec<String> = device
        .partitions
        .iter()
        .map(|p| format!("{}:", p.letter))
        .collect();
    if letters.is_empty() {
        name
    } else {
        format!("{name} · {}", letters.join("  "))
    }
}

/// The connection as the details say it.
pub fn bus_label(lang: Lang, bus: Bus) -> String {
    match bus {
        Bus::Usb3 => "USB 3".into(),
        Bus::Usb2 => "USB 2".into(),
        Bus::Usb => "USB".into(),
        Bus::CardReader => text(lang, Key::DeviceCardReader).into(),
        Bus::Thunderbolt => "Thunderbolt".into(),
        Bus::Other => text(lang, Key::DeviceBusOther).into(),
    }
}

/// The card's fallback name when the device reports neither a vendor nor a
/// product and its volume has no label.
pub fn kind_name(lang: Lang, kind: DeviceKind) -> &'static str {
    text(
        lang,
        match kind {
            DeviceKind::Usb => Key::DeviceKindUsb,
            DeviceKind::Hdd => Key::DeviceKindHdd,
            DeviceKind::Sd => Key::DeviceKindSd,
        },
    )
}

/// Used and total bytes over every partition, `None` before any capacity
/// was read.
pub fn used_total(device: &Device) -> Option<(u64, u64)> {
    let mut known = false;
    let (mut used, mut total) = (0u64, 0u64);
    for p in &device.partitions {
        if let Some(free) = p.free {
            known = true;
            total += p.total;
            used += p.total.saturating_sub(free);
        }
    }
    (known && total > 0).then_some((used, total))
}

/// "18,4 GB / 57,3 GB" under a card's bar.
pub fn used_total_text(lang: Lang, used: u64, total: u64) -> String {
    format!("{} / {}", device_size(lang, used), device_size(lang, total))
}

/// The file systems of the device in the details, each once ("NTFS",
/// "NTFS, exFAT"); empty when none is known.
pub fn fs_summary(device: &Device) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for p in &device.partitions {
        if !p.fs.is_empty() && !seen.contains(&p.fs.as_str()) {
            seen.push(&p.fs);
        }
    }
    seen.join(", ")
}

/// One partition in the details: "F: · NTFS · 612 GB / 931 GB · Data".
pub fn partition_line(lang: Lang, p: &Partition) -> String {
    let mut parts = vec![format!("{}:", p.letter)];
    if !p.fs.is_empty() {
        parts.push(p.fs.clone());
    }
    match p.free {
        Some(free) => parts.push(used_total_text(lang, p.total.saturating_sub(free), p.total)),
        None if p.total > 0 => parts.push(device_size(lang, p.total)),
        None => {}
    }
    if !p.label.is_empty() {
        parts.push(p.label.clone());
    }
    parts.join(" · ")
}

// ---- What changed ----

/// The difference between two scans, matched by `key`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Change {
    pub arrived: Vec<Device>,
    pub removed: Vec<String>,
}

pub fn diff(old: &[Device], new: &[Device]) -> Change {
    Change {
        arrived: new
            .iter()
            .filter(|n| !old.iter().any(|o| o.key == n.key))
            .cloned()
            .collect(),
        removed: old
            .iter()
            .filter(|o| !new.iter().any(|n| n.key == o.key))
            .map(|o| o.key.clone())
            .collect(),
    }
}

// ---- Notices ----

/// "SanDisk Ultra tersambung" with "E: · 38,9 GB kosong": the first
/// partition (with its free space when it was read), else just the letters.
pub fn plugged_notice(lang: Lang, device: &Device) -> Notice {
    let first = device
        .partitions
        .iter()
        .find(|p| p.letter.is_ascii_alphabetic());
    let body = match first {
        Some(p) => match p.free {
            Some(free) => i18n::device_free(lang, p.letter, &device_size(lang, free)),
            None => device
                .partitions
                .iter()
                .map(|p| format!("{}:", p.letter))
                .collect::<Vec<_>>()
                .join(", "),
        },
        None => String::new(),
    };
    Notice {
        key: format!("device-plugged-{}", device.key),
        source: text(lang, Key::PageDevices).to_owned(),
        title: i18n::device_plugged(lang, &display_name(lang, device)),
        body,
        kind: NoticeKind::DevicePlugged {
            letter: first.map_or('\0', |p| p.letter),
        },
    }
}

/// The longest list of names the safe-to-remove notice writes out.
const SAFE_NAMES_MAX: usize = 40;

/// "Aman dicabut" with the names, or how many when they would not fit.
pub fn safe_notice(lang: Lang, names: &[String]) -> Notice {
    let joined = names.join(", ");
    let body = if joined.chars().count() > SAFE_NAMES_MAX {
        i18n::device_count(lang, names.len())
    } else {
        joined
    };
    Notice {
        key: "device-safe".into(),
        source: text(lang, Key::PageDevices).to_owned(),
        title: text(lang, Key::DeviceSafe).to_owned(),
        body,
        kind: NoticeKind::DeviceSafe,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    fn part(letter: char, fs: &str, total: u64, free: Option<u64>) -> Partition {
        Partition {
            letter,
            label: String::new(),
            fs: fs.into(),
            total,
            free,
        }
    }

    fn device(key: &str, name: &str, kind: DeviceKind, bus: Bus, parts: Vec<Partition>) -> Device {
        Device {
            key: key.into(),
            name: name.into(),
            model: name.into(),
            kind,
            bus,
            partitions: parts,
        }
    }

    fn sandisk() -> Device {
        device(
            "USB\\VID_0781\\A",
            "SanDisk Ultra",
            DeviceKind::Usb,
            Bus::Usb3,
            vec![part(
                'E',
                "exFAT",
                57 * GB + GB * 3 / 10,
                Some(38 * GB + GB * 9 / 10),
            )],
        )
    }

    fn veto(kind: VetoKind, name: &str) -> Veto {
        Veto {
            kind,
            name: name.into(),
        }
    }

    #[test]
    fn classification_table() {
        let (usb, sd, mmc) = (BUS_TYPE_USB, BUS_TYPE_SD, BUS_TYPE_MMC);
        assert_eq!(classify(sd, false, "anything"), DeviceKind::Sd);
        assert_eq!(classify(mmc, true, "anything"), DeviceKind::Sd);
        // USB, removable: a reader by its name, else a flash drive.
        assert_eq!(classify(usb, true, "Generic Card Reader"), DeviceKind::Sd);
        assert_eq!(classify(usb, true, "MULTI READER"), DeviceKind::Sd);
        assert_eq!(classify(usb, true, "SD/MMC"), DeviceKind::Sd);
        assert_eq!(classify(usb, true, "Mass SDXC"), DeviceKind::Sd);
        assert_eq!(classify(usb, true, "mmc"), DeviceKind::Sd);
        assert_eq!(classify(usb, true, "Ultra USB 3.0"), DeviceKind::Usb);
        assert_eq!(classify(usb, true, "SanDisk Cruzer"), DeviceKind::Usb);
        assert_eq!(classify(usb, true, "DataTraveler"), DeviceKind::Usb);
        assert_eq!(classify(usb, true, ""), DeviceKind::Usb);
        // Fixed media, or another bus: a disk.
        assert_eq!(classify(usb, false, "Elements"), DeviceKind::Hdd);
        assert_eq!(classify(usb, false, "Card Reader"), DeviceKind::Hdd);
        assert_eq!(classify(0x11, false, "NVMe"), DeviceKind::Hdd); // NVMe
        assert_eq!(classify(0xB, false, "SATA"), DeviceKind::Hdd); // SATA
        assert_eq!(classify(0x11, true, "x"), DeviceKind::Hdd);
    }

    #[test]
    fn app_names_are_the_last_component_without_exe_and_capitalised() {
        assert_eq!(
            app_name(r"C:\Program Files\Office\EXCEL.EXE").as_deref(),
            Some("Excel")
        );
        assert_eq!(app_name("explorer.exe").as_deref(), Some("Explorer"));
        assert_eq!(app_name("WINWORD.exe").as_deref(), Some("Winword"));
        assert_eq!(app_name("C:/x/Code.Exe").as_deref(), Some("Code"));
        // Mixed case stays as it is.
        assert_eq!(app_name("OneDrive.exe").as_deref(), Some("OneDrive"));
        assert_eq!(
            app_name("Excel (Laporan.xlsx)").as_deref(),
            Some("Excel (Laporan.xlsx)")
        );
        assert_eq!(app_name(""), None);
        assert_eq!(app_name("   "), None);
        assert_eq!(app_name(r"C:\x\.exe"), None);
        assert_eq!(app_name(r"C:\x\"), None);
    }

    #[test]
    fn every_veto_kind_has_a_message_in_both_languages() {
        use VetoKind::*;
        let id = |k, n: &str| veto_message(Lang::Id, &veto(k, n));
        let en = |k, n: &str| veto_message(Lang::En, &veto(k, n));
        for k in [
            Unknown,
            LegacyDevice,
            PendingClose,
            WindowsApp,
            WindowsService,
            OutstandingOpen,
            Device,
            Driver,
            IllegalDeviceRequest,
            InsufficientPower,
            NonDisableable,
            LegacyDriver,
            InsufficientRights,
            AlreadyRemoved,
        ] {
            for n in ["", r"C:\Apps\TOOL.EXE"] {
                let (ti, di) = id(k, n);
                let (te, de) = en(k, n);
                assert_eq!(ti, "Belum bisa dilepas.", "{k:?}");
                assert_eq!(te, "Can't remove it yet.", "{k:?}");
                assert!(!di.is_empty() && !de.is_empty(), "{k:?}");
            }
        }
        // An app, with and without a usable name.
        let k = WindowsApp;
        assert_eq!(id(k, r"C:\Office\EXCEL.EXE").1, "Masih dipakai: Excel");
        assert_eq!(en(k, r"C:\Office\EXCEL.EXE").1, "Still in use: Excel");
        assert_eq!(id(k, "").1, "Masih dipakai aplikasi lain.");
        assert_eq!(en(k, "  ").1, "Still in use by another app.");
        assert_eq!(id(k, r"C:\x\.exe").1, "Masih dipakai aplikasi lain.");
        // A pending close names a device instance path: never shown.
        for n in [
            "",
            r"USBSTOR\DISK&VEN_SANDISK&PROD_ULTRA&REV_1.00\4C530001230101&0",
            r"C:\Office\EXCEL.EXE",
        ] {
            assert_eq!(id(PendingClose, n).1, "Masih dipakai aplikasi lain.");
            assert_eq!(en(PendingClose, n).1, "Still in use by another app.");
        }
        // The design's own example.
        assert_eq!(
            id(WindowsApp, "Excel (Laporan.xlsx)").1,
            "Masih dipakai: Excel (Laporan.xlsx)"
        );
        // An open handle's name is a device path: never shown.
        assert_eq!(
            id(OutstandingOpen, r"\Device\HarddiskVolume7\a.docx").1,
            "Masih dipakai aplikasi lain."
        );
        assert_eq!(
            en(OutstandingOpen, r"\Device\HarddiskVolume7").1,
            "Still in use by another app."
        );
        assert_eq!(
            id(WindowsService, "SysMain").1,
            "Masih dipakai layanan Windows: SysMain"
        );
        assert_eq!(
            en(WindowsService, "SysMain").1,
            "In use by a Windows service: SysMain"
        );
        assert_eq!(id(WindowsService, "").1, "Masih dipakai layanan Windows.");
        assert_eq!(en(WindowsService, "").1, "In use by a Windows service.");
        assert_eq!(id(Device, "").1, "Perangkat lain masih memakainya.");
        assert_eq!(en(Device, "").1, "Another device is still using it.");
        for k in [Driver, LegacyDriver, LegacyDevice] {
            assert_eq!(id(k, "").1, "Driver-nya menolak dilepas.");
            assert_eq!(en(k, "").1, "Its driver refused.");
        }
        assert_eq!(
            id(NonDisableable, "").1,
            "Windows tidak mengizinkan perangkat ini dilepas."
        );
        assert_eq!(
            en(NonDisableable, "").1,
            "Windows does not allow removing this device."
        );
        assert_eq!(id(InsufficientRights, "").1, "Butuh izin administrator.");
        assert_eq!(en(InsufficientRights, "").1, "Needs administrator rights.");
        assert_eq!(id(InsufficientPower, "").1, "Dayanya tidak cukup.");
        assert_eq!(en(InsufficientPower, "").1, "Not enough power.");
        for k in [Unknown, IllegalDeviceRequest] {
            assert_eq!(id(k, "").1, "Windows menolak melepasnya.");
            assert_eq!(en(k, "").1, "Windows refused to remove it.");
        }
    }

    #[test]
    fn the_listed_name_is_vendor_and_product_without_a_repeated_vendor() {
        assert_eq!(vendor_product_name("SanDisk", "Ultra"), "SanDisk Ultra");
        assert_eq!(
            vendor_product_name("SanDisk ", " SanDisk Ultra "),
            "SanDisk Ultra"
        );
        assert_eq!(vendor_product_name("WD", "wd Elements"), "wd Elements");
        assert_eq!(
            vendor_product_name("Generic", "STORAGE DEVICE"),
            "Generic STORAGE DEVICE"
        );
        assert_eq!(vendor_product_name("", " Ultra "), "Ultra");
        assert_eq!(vendor_product_name("SanDisk", ""), "SanDisk");
        assert_eq!(vendor_product_name(" ", " "), "");
    }

    #[test]
    fn an_eject_succeeds_without_a_veto_or_when_already_gone() {
        assert_eq!(eject_outcome(true, 0, ""), Ok(Ejected::Safe));
        // Already removed is gone, not a safe eject, whatever the call said.
        assert_eq!(eject_outcome(true, 13, ""), Ok(Ejected::Gone));
        assert_eq!(eject_outcome(false, 13, ""), Ok(Ejected::Gone));
        // Refused: the kind and the name Windows gave.
        assert_eq!(
            eject_outcome(false, 3, r"C:\x\EXCEL.EXE"),
            Err(Veto {
                kind: VetoKind::WindowsApp,
                name: r"C:\x\EXCEL.EXE".into()
            })
        );
        // Failed with no reason given.
        assert_eq!(
            eject_outcome(false, 0, ""),
            Err(Veto {
                kind: VetoKind::Unknown,
                name: String::new()
            })
        );
        // Success with a veto type that is not "unknown" is not trusted.
        assert!(eject_outcome(true, 5, "").is_err());
    }

    #[test]
    fn a_pnp_veto_value_maps_to_its_kind() {
        let kinds = [
            VetoKind::Unknown,
            VetoKind::LegacyDevice,
            VetoKind::PendingClose,
            VetoKind::WindowsApp,
            VetoKind::WindowsService,
            VetoKind::OutstandingOpen,
            VetoKind::Device,
            VetoKind::Driver,
            VetoKind::IllegalDeviceRequest,
            VetoKind::InsufficientPower,
            VetoKind::NonDisableable,
            VetoKind::LegacyDriver,
            VetoKind::InsufficientRights,
            VetoKind::AlreadyRemoved,
        ];
        for (raw, kind) in kinds.iter().enumerate() {
            assert_eq!(VetoKind::from_raw(raw as i32), *kind, "{raw}");
        }
        assert_eq!(VetoKind::from_raw(99), VetoKind::Unknown);
        assert_eq!(VetoKind::from_raw(-1), VetoKind::Unknown);
    }

    #[test]
    fn sizes_in_binary_units_compared_as_shown() {
        let id = |b| device_size(Lang::Id, b);
        let en = |b| device_size(Lang::En, b);
        let gb = |x: f64| (x * GB as f64) as u64;
        assert_eq!(id(512 * 1024 * 1024), "512 MB");
        assert_eq!(id(0), "0 MB");
        // Just under 1 GB rounds up to 1024 MB: "1,0 GB" instead.
        assert_eq!(id(GB - 1), "1,0 GB");
        assert_eq!(id(gb(18.4)), "18,4 GB");
        assert_eq!(id(gb(57.3)), "57,3 GB");
        assert_eq!(en(gb(18.4)), "18.4 GB");
        assert_eq!(id(gb(99.94)), "99,9 GB");
        assert_eq!(id(gb(99.96)), "100 GB", "as shown, not 100,0");
        assert_eq!(id(gb(100.0)), "100 GB");
        assert_eq!(id(gb(612.0)), "612 GB");
        assert_eq!(id(gb(931.0)), "931 GB");
        assert_eq!(en(gb(612.4)), "612 GB");
        assert_eq!(id(gb(999.4)), "999 GB");
        assert_eq!(id(gb(999.96)), "1,0 TB", "never 1000 GB");
        assert_eq!(id(1000 * GB), "1,0 TB");
        assert_eq!(id(gb(1.8 * 1024.0)), "1,8 TB");
        assert_eq!(en(gb(1.8 * 1024.0)), "1.8 TB");
        assert_eq!(id(gb(4.0 * 1024.0)), "4,0 TB");
    }

    #[test]
    fn subtitles_by_kind_bus_and_partitions() {
        let usb = sandisk();
        assert_eq!(subtitle(Lang::Id, &usb), "Flashdisk USB 3 · exFAT");
        assert_eq!(subtitle(Lang::En, &usb), "USB flash drive 3 · exFAT");
        let mut usb2 = usb.clone();
        usb2.bus = Bus::Usb2;
        assert_eq!(subtitle(Lang::Id, &usb2), "Flashdisk USB 2 · exFAT");
        usb2.bus = Bus::Usb;
        assert_eq!(subtitle(Lang::Id, &usb2), "Flashdisk USB · exFAT");
        assert_eq!(subtitle(Lang::En, &usb2), "USB flash drive · exFAT");
        usb2.partitions[0].fs = String::new();
        assert_eq!(subtitle(Lang::Id, &usb2), "Flashdisk USB");

        let two = device(
            "k",
            "WD Elements",
            DeviceKind::Hdd,
            Bus::Usb3,
            vec![
                part('F', "NTFS", 600 * GB, Some(10 * GB)),
                part('G', "NTFS", 331 * GB, Some(10 * GB)),
            ],
        );
        assert_eq!(
            subtitle(Lang::Id, &two),
            "Disk eksternal USB · 2 partisi NTFS"
        );
        assert_eq!(
            subtitle(Lang::En, &two),
            "External USB disk · 2 NTFS partitions"
        );
        let one = Device {
            partitions: vec![part('F', "NTFS", GB, None)],
            ..two.clone()
        };
        assert_eq!(subtitle(Lang::Id, &one), "Disk eksternal USB · NTFS");
        let tb = Device {
            bus: Bus::Thunderbolt,
            partitions: vec![],
            ..two.clone()
        };
        assert_eq!(subtitle(Lang::Id, &tb), "Disk eksternal Thunderbolt");
        assert_eq!(subtitle(Lang::En, &tb), "External Thunderbolt disk");
        let mixed = Device {
            partitions: vec![part('F', "NTFS", GB, None), part('G', "exFAT", GB, None)],
            ..two
        };
        assert_eq!(subtitle(Lang::Id, &mixed), "Disk eksternal USB · 2 partisi");

        let card = device(
            "c",
            "Kartu SD",
            DeviceKind::Sd,
            Bus::CardReader,
            vec![part('H', "FAT32", 29 * GB, None)],
        );
        assert_eq!(subtitle(Lang::Id, &card), "Pembaca kartu · FAT32");
        assert_eq!(subtitle(Lang::En, &card), "Card reader · FAT32");
    }

    #[test]
    fn the_title_line_lists_the_letters() {
        assert_eq!(title_line(Lang::Id, &sandisk()), "SanDisk Ultra · E:");
        let wd = device(
            "k",
            "WD Elements",
            DeviceKind::Hdd,
            Bus::Usb,
            vec![part('F', "NTFS", GB, None), part('G', "NTFS", GB, None)],
        );
        assert_eq!(title_line(Lang::Id, &wd), "WD Elements · F:  G:");
        let none = Device {
            partitions: vec![],
            ..wd
        };
        assert_eq!(title_line(Lang::Id, &none), "WD Elements");
    }

    #[test]
    fn a_nameless_device_is_named_by_its_kind_in_the_language_drawn() {
        let mut bare = sandisk();
        bare.name = String::new();
        let (id, en) = (
            kind_name(Lang::Id, bare.kind),
            kind_name(Lang::En, bare.kind),
        );
        assert_eq!(display_name(Lang::Id, &bare), id);
        assert_eq!(display_name(Lang::En, &bare), en);
        assert_eq!(title_line(Lang::En, &bare), format!("{en} · E:"));
        assert!(plugged_notice(Lang::En, &bare).title.starts_with(en));
        assert_eq!(display_name(Lang::En, &sandisk()), "SanDisk Ultra");
    }

    #[test]
    fn used_and_total_span_every_partition() {
        let wd = device(
            "k",
            "WD",
            DeviceKind::Hdd,
            Bus::Usb,
            vec![
                part('F', "NTFS", 600 * GB, Some(100 * GB)),
                part('G', "NTFS", 331 * GB, Some(219 * GB)),
            ],
        );
        assert_eq!(used_total(&wd), Some((612 * GB, 931 * GB)));
        assert_eq!(
            used_total_text(Lang::Id, 612 * GB, 931 * GB),
            "612 GB / 931 GB"
        );
        let unknown = Device {
            partitions: vec![part('F', "NTFS", 0, None)],
            ..wd
        };
        assert_eq!(used_total(&unknown), None);
        // A partition line in the details.
        let p = Partition {
            label: "Data".into(),
            ..part('F', "NTFS", 931 * GB, Some(319 * GB))
        };
        assert_eq!(
            partition_line(Lang::Id, &p),
            "F: · NTFS · 612 GB / 931 GB · Data"
        );
        let bare = part('H', "", 29 * GB, None);
        assert_eq!(partition_line(Lang::En, &bare), "H: · 29.0 GB");
        let mixed = Device {
            partitions: vec![
                part('F', "NTFS", GB, None),
                part('G', "exFAT", GB, None),
                part('H', "NTFS", GB, None),
            ],
            ..sandisk()
        };
        assert_eq!(fs_summary(&mixed), "NTFS, exFAT");
        assert_eq!(fs_summary(&sandisk()), "exFAT");
        assert_eq!(
            fs_summary(&Device {
                partitions: vec![],
                ..sandisk()
            }),
            ""
        );
    }

    #[test]
    fn diff_finds_arrivals_removals_both_and_nothing() {
        let a = sandisk();
        let b = device(
            "B",
            "WD",
            DeviceKind::Hdd,
            Bus::Usb,
            vec![part('F', "NTFS", GB, None)],
        );
        let c = device("C", "Kartu", DeviceKind::Sd, Bus::CardReader, vec![]);
        let same = diff(&[a.clone(), b.clone()], &[b.clone(), a.clone()]);
        assert_eq!(same, Change::default());
        let arrival = diff(std::slice::from_ref(&a), &[a.clone(), b.clone()]);
        assert_eq!(arrival.arrived, vec![b.clone()]);
        assert!(arrival.removed.is_empty());
        let removal = diff(&[a.clone(), b.clone()], std::slice::from_ref(&b));
        assert!(removal.arrived.is_empty());
        assert_eq!(removal.removed, vec![a.key.clone()]);
        let both = diff(&[a.clone(), b.clone()], &[b.clone(), c.clone()]);
        assert_eq!(both.arrived, vec![c]);
        assert_eq!(both.removed, vec![a.key.clone()]);
        // Capacity read later does not make a device arrive again.
        let mut read = a.clone();
        read.partitions[0].free = None;
        assert_eq!(diff(&[a], &[read]), Change::default());
        assert_eq!(diff(&[], &[]), Change::default());
    }

    #[test]
    fn the_plugged_notice_names_the_drive_and_its_free_space() {
        let n = plugged_notice(Lang::Id, &sandisk());
        assert_eq!(n.title, "SanDisk Ultra tersambung");
        assert_eq!(n.body, "E: · 38,9 GB kosong");
        assert_eq!(n.kind, NoticeKind::DevicePlugged { letter: 'E' });
        assert_eq!(n.key, "device-plugged-USB\\VID_0781\\A");
        assert_eq!(n.source, "Perangkat");
        let en = plugged_notice(Lang::En, &sandisk());
        assert_eq!(en.title, "SanDisk Ultra connected");
        assert_eq!(en.body, "E: · 38.9 GB free");
        // Capacity not read: just the letters.
        let mut bare = sandisk();
        bare.partitions[0].free = None;
        bare.partitions.push(part('F', "exFAT", 0, None));
        let n = plugged_notice(Lang::Id, &bare);
        assert_eq!(n.body, "E:, F:");
        assert_eq!(n.kind, NoticeKind::DevicePlugged { letter: 'E' });
    }

    #[test]
    fn the_safe_notice_lists_names_or_counts_them() {
        let one = safe_notice(Lang::Id, &["SanDisk Ultra".to_owned()]);
        assert_eq!(one.title, "Aman dicabut");
        assert_eq!(one.body, "SanDisk Ultra");
        assert_eq!(one.key, "device-safe");
        assert_eq!(one.kind, NoticeKind::DeviceSafe);
        let two = safe_notice(Lang::En, &["SanDisk Ultra".to_owned(), "WD".to_owned()]);
        assert_eq!(two.title, "Safe to remove");
        assert_eq!(two.body, "SanDisk Ultra, WD");
        let long: Vec<String> = ["SanDisk Ultra Fit", "WD Elements Portable", "Kartu SD"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(safe_notice(Lang::Id, &long).body, "3 perangkat");
        assert_eq!(safe_notice(Lang::En, &long).body, "3 devices");
        // Exactly at the limit still lists.
        let edge = vec!["a".repeat(SAFE_NAMES_MAX)];
        assert_eq!(safe_notice(Lang::Id, &edge).body, edge[0]);
        let over = vec!["a".repeat(SAFE_NAMES_MAX + 1)];
        assert_eq!(safe_notice(Lang::Id, &over).body, "1 perangkat");
    }
}
