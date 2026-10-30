//! The Devices page's Windows side: which flash drives, external disks and
//! memory cards are plugged in, and the eject. Both run on one parked worker
//! thread (`Devices`), never on the UI thread: opening volumes and asking
//! the disk for its size can block, and `CM_Request_Device_EjectW` waits for
//! Windows to finish. The thread blocks on its channel, so it costs nothing
//! while nothing is plugged, unplugged or asked.
//!
//! A drive is listed when it has a drive letter and sits under a device node
//! the system calls removable (`DN_REMOVABLE`): that node is what Eject
//! requests removal of, and its instance ID is the device's key. Internal
//! disks have no removable ancestor and never show; one behind a hot-plug
//! SATA or PCIe port does, so the disk holding Windows or a page file, and a
//! SATA, ATA, SAS or RAID disk with nothing external on its chain, are left
//! out too (`pinned_disks`, `internal_disk`). Every handle is closed
//! as soon as it has answered (`Handle`): one kept open would veto our own
//! eject. Everything that can be decided without Windows (the names, the
//! sizes, the sentences, what changed) is `devices.rs`.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem::{size_of, size_of_val};
use std::sync::mpsc::{self, Receiver, Sender};

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_DEVNODE_STATUS_FLAGS, CM_DRP_ADDRESS, CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
    CM_Get_DevNode_Registry_PropertyW, CM_Get_DevNode_Status, CM_Get_Device_IDW,
    CM_Get_Device_Interface_ListW, CM_Get_Parent, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW,
    CM_PROB, CM_Request_Device_EjectW, CR_SUCCESS, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    DN_REMOVABLE, HDEVINFO, PNP_VETO_TYPE, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::Devices::Usb::{
    GUID_DEVINTERFACE_USB_HUB, IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2,
    USB_NODE_CONNECTION_INFORMATION_EX_V2, USB_PROTOCOLS,
};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_NO_MEDIA_IN_DRIVE, ERROR_NOT_READY,
    ERROR_SHARING_VIOLATION, GetLastError, HANDLE, WIN32_ERROR,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW,
    GetDriveTypeW, GetFileAttributesW, GetLogicalDrives, GetVolumeInformationW,
    INVALID_FILE_ATTRIBUTES, OPEN_EXISTING,
};
use windows::Win32::System::Diagnostics::Debug::{
    SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX, SetThreadErrorMode,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY,
    PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR, STORAGE_DEVICE_NUMBER,
    STORAGE_PROPERTY_QUERY, StorageDeviceProperty,
};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows::core::{HSTRING, PCWSTR};

use crate::devices::{
    self, BUS_TYPE_USB, Bus, Device, DeviceKind, DevicesEvent, Partition, Veto, classify,
    eject_outcome, vendor_product_name,
};
use crate::i18n;
use crate::model::Event;

/// How much of a scan reads each volume's size and free space
/// (`GetDiskFreeSpaceExW`, which can wake a sleeping disk).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capacity {
    /// None: the figures of the last list are kept for the drives still
    /// there (the startup scan).
    Keep,
    /// Only for a device that was not in the last list: the one just plugged
    /// in, for its notice (also the scan after an eject, which may be the
    /// first to see a drive plugged in while it ran).
    NewOnly,
    /// Every volume: the page opened.
    All,
}

pub enum Req {
    Scan { seq: u64, capacity: Capacity },
    Eject { keys: Vec<String> },
}

/// A batch waiting in the queue, with consecutive scans merged into one: the
/// newest number and the most that any of them asked to read.
fn merge(batch: Vec<Req>) -> Vec<Req> {
    let mut out: Vec<Req> = Vec::new();
    for req in batch {
        match (out.last_mut(), req) {
            (
                Some(Req::Scan { seq, capacity }),
                Req::Scan {
                    seq: next,
                    capacity: more,
                },
            ) => {
                *seq = (*seq).max(next);
                *capacity = (*capacity).max(more);
            }
            (_, req) => out.push(req),
        }
    }
    out
}

/// The thread's handle. Dropping it ends the thread.
pub struct Devices {
    requests: Sender<Req>,
}

impl Devices {
    /// `None` when the thread could not start.
    pub fn spawn(tx: Sender<Event>, wake: impl Fn() + Send + 'static) -> Option<Self> {
        let (requests, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("devices".into())
            .spawn(move || serve(&rx, &tx, wake));
        match spawned {
            Ok(_) => Some(Self { requests }),
            Err(error) => {
                eprintln!("vale: no devices thread: {error}");
                None
            }
        }
    }

    /// False when the thread is gone.
    pub fn request(&self, req: Req) -> bool {
        self.requests.send(req).is_ok()
    }
}

fn serve(rx: &Receiver<Req>, tx: &Sender<Event>, wake: impl Fn()) {
    // A removable drive with no media (an empty card slot) must answer "not
    // ready" to the scan, never raise Windows' "There is no disk in the
    // drive" dialog, which would block this thread until someone closed it.
    unsafe {
        let _ = SetThreadErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX, None);
    }
    // What the last scan found: capacity is read for what is not in it, and
    // kept for what is.
    let mut last: Vec<Device> = Vec::new();
    let mut seq = 0u64;
    let send = |event: DevicesEvent| {
        let sent = tx.send(Event::Devices(event)).is_ok();
        if sent {
            wake();
        }
        sent
    };
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        for req in merge(batch) {
            match req {
                Req::Scan {
                    seq: asked,
                    capacity,
                } => {
                    seq = seq.max(asked);
                    last = scan(capacity, &last);
                    if !send(DevicesEvent::Listed {
                        seq: asked,
                        devices: last.clone(),
                    }) {
                        return;
                    }
                }
                Req::Eject { keys } => {
                    let results = keys
                        .into_iter()
                        .map(|key| {
                            // Named now, for the notice raised right after.
                            let name = last.iter().find(|d| d.key == key).map_or_else(
                                || key.clone(),
                                |d| devices::display_name(i18n::lang(), d),
                            );
                            let outcome = eject(&key);
                            (key, name, outcome)
                        })
                        .collect();
                    if !send(DevicesEvent::Ejected { results }) {
                        return;
                    }
                    // What is left, under the number of the newest scan
                    // asked for, so it is never taken for a stale one. A
                    // drive plugged in meanwhile has its size read here: the
                    // debounced scan after it would find it known already.
                    last = scan(Capacity::NewOnly, &last);
                    if !send(DevicesEvent::Listed {
                        seq,
                        devices: last.clone(),
                    }) {
                        return;
                    }
                }
            }
        }
    }
}

// ---- Handles and IOCTLs ----

/// An open handle, closed when it goes out of scope.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Opens `path` with `access` (0 asks for nothing but the device's own
/// answers) and full sharing, so it never blocks anyone else.
fn open(path: &HSTRING, access: u32) -> Option<Handle> {
    unsafe {
        CreateFileW(
            path,
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .ok()
    .map(Handle)
}

/// `DeviceIoControl`; the bytes returned, `None` when it failed.
fn ioctl(
    handle: &Handle,
    code: u32,
    input: *const c_void,
    in_len: usize,
    out: *mut c_void,
    out_len: usize,
) -> Option<u32> {
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            handle.0,
            code,
            (!input.is_null()).then_some(input),
            in_len as u32,
            Some(out),
            out_len as u32,
            Some(&mut returned),
            None,
        )
    }
    .ok()?;
    Some(returned)
}

/// `FILE_DEVICE_DISK`: what a volume on a disk reports as its device type.
const FILE_DEVICE_DISK: u32 = 7;

fn device_number(handle: &Handle) -> Option<STORAGE_DEVICE_NUMBER> {
    let mut number = STORAGE_DEVICE_NUMBER::default();
    ioctl(
        handle,
        IOCTL_STORAGE_GET_DEVICE_NUMBER,
        std::ptr::null(),
        0,
        (&mut number as *mut STORAGE_DEVICE_NUMBER).cast(),
        size_of::<STORAGE_DEVICE_NUMBER>(),
    )?;
    Some(number)
}

/// What the storage driver says about a disk.
struct Descriptor {
    bus_type: u32,
    removable: bool,
    vendor: String,
    product: String,
}

/// The NUL-ended text at `offset` in `bytes`; empty for offset 0 (none).
fn text_at(bytes: &[u8], offset: u32) -> String {
    let start = offset as usize;
    if start == 0 || start >= bytes.len() {
        return String::new();
    }
    let tail = &bytes[start..];
    let end = tail.iter().position(|b| *b == 0).unwrap_or(tail.len());
    String::from_utf8_lossy(&tail[..end]).trim().to_owned()
}

fn descriptor(handle: &Handle) -> Option<Descriptor> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    // 8-aligned, so the descriptor can be read in place.
    let mut buf = [0u64; 128];
    let got = ioctl(
        handle,
        IOCTL_STORAGE_QUERY_PROPERTY,
        (&query as *const STORAGE_PROPERTY_QUERY).cast(),
        size_of::<STORAGE_PROPERTY_QUERY>(),
        buf.as_mut_ptr().cast(),
        size_of_val(&buf),
    )? as usize;
    if got < size_of::<STORAGE_DEVICE_DESCRIPTOR>() || got > size_of_val(&buf) {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), got) };
    let d: STORAGE_DEVICE_DESCRIPTOR = unsafe { std::ptr::read(buf.as_ptr().cast()) };
    Some(Descriptor {
        bus_type: d.BusType.0 as u32,
        removable: d.RemovableMedia,
        vendor: text_at(bytes, d.VendorIdOffset),
        product: text_at(bytes, d.ProductIdOffset),
    })
}

// ---- Volumes ----

struct Volume {
    letter: char,
    number: u32,
    desc: Descriptor,
}

const DRIVE_UNKNOWN: u32 = 0;
const DRIVE_NO_ROOT_DIR: u32 = 1;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;

/// Text up to the first NUL.
fn wide_text(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// The label and file system of the volume at `root` ("E:\"); `Err(())` when
/// it holds no media (an empty card slot).
fn volume_info(root: &HSTRING) -> Result<(String, String), ()> {
    let (mut label, mut fs) = ([0u16; 261], [0u16; 64]);
    match unsafe { GetVolumeInformationW(root, Some(&mut label), None, None, None, Some(&mut fs)) }
    {
        Ok(()) => Ok((wide_text(&label), wide_text(&fs))),
        Err(e)
            if e.code() == ERROR_NOT_READY.to_hresult()
                || e.code() == ERROR_NO_MEDIA_IN_DRIVE.to_hresult() =>
        {
            Err(())
        }
        // Unreadable for another reason (a locked or unformatted volume):
        // still listed, so it can be ejected.
        Err(_) => Ok((String::new(), String::new())),
    }
}

/// Every local volume on a disk, in letter order, with what the storage
/// driver says about the disk (answered without touching the disk's
/// contents, so an internal disk is never woken). Network drives, optical
/// drives and letters with nothing behind them are left out. The volume's own
/// label and file system are read only for the ones that get listed.
fn volumes() -> Vec<Volume> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|bit| mask & (1 << bit) != 0)
        .filter_map(|bit| {
            let letter = char::from(b'A' + bit);
            let root = HSTRING::from(format!("{letter}:\\"));
            let kind = unsafe { GetDriveTypeW(&root) };
            if matches!(
                kind,
                DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR | DRIVE_REMOTE | DRIVE_CDROM
            ) {
                return None;
            }
            let handle = open(&HSTRING::from(format!("\\\\.\\{letter}:")), 0)?;
            let number = device_number(&handle)?;
            let desc = descriptor(&handle);
            drop(handle);
            if number.DeviceType != FILE_DEVICE_DISK {
                return None;
            }
            Some(Volume {
                letter,
                number: number.DeviceNumber,
                desc: desc?,
            })
        })
        .collect()
}

/// Total and free bytes of the volume at `letter` (free to the caller).
fn read_capacity(letter: char) -> Option<(u64, u64)> {
    let root = HSTRING::from(format!("{letter}:\\"));
    let (mut free, mut total) = (0u64, 0u64);
    unsafe { GetDiskFreeSpaceExW(&root, Some(&mut free), Some(&mut total), None) }.ok()?;
    (total > 0).then_some((total, free))
}

// ---- Device nodes ----

/// A destroyed-on-drop device information set.
struct InfoSet(HDEVINFO);

impl Drop for InfoSet {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

/// The text up to the first NUL of the UTF-16 at `ptr`.
fn read_wide(ptr: *const u16) -> String {
    let mut len = 0;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// The device node of each present disk, by its disk number: the disk
/// interfaces are walked, each opened for its number only.
fn disk_nodes() -> HashMap<u32, u32> {
    let mut nodes = HashMap::new();
    let Ok(set) = (unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVINTERFACE_DISK),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }) else {
        return nodes;
    };
    let set = InfoSet(set);
    for index in 0.. {
        let mut data = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        if unsafe {
            SetupDiEnumDeviceInterfaces(set.0, None, &GUID_DEVINTERFACE_DISK, index, &mut data)
        }
        .is_err()
        {
            break;
        }
        // The first call only says how big the detail is.
        let mut needed = 0u32;
        let _ = unsafe {
            SetupDiGetDeviceInterfaceDetailW(set.0, &data, None, 0, Some(&mut needed), None)
        };
        if (needed as usize) < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() {
            continue;
        }
        // 8-aligned storage for the variable-sized struct.
        let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
        let detail = buf.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        unsafe { (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 };
        let mut info = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        let got = unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &data,
                Some(detail),
                needed,
                None,
                Some(&mut info),
            )
        };
        if got.is_err() {
            continue;
        }
        let path = read_wide(unsafe { std::ptr::addr_of!((*detail).DevicePath) }.cast());
        let Some(handle) = open(&HSTRING::from(path), 0) else {
            continue;
        };
        if let Some(number) = device_number(&handle) {
            nodes.insert(number.DeviceNumber, info.DevInst);
        }
    }
    nodes
}

/// The instance ID of device node `inst`.
fn device_id(inst: u32) -> Option<String> {
    let mut buf = [0u16; 512];
    if unsafe { CM_Get_Device_IDW(inst, &mut buf, 0) } != CR_SUCCESS {
        return None;
    }
    // An empty ID names no device (looked up, it would be the tree's root).
    Some(wide_text(&buf)).filter(|id| !id.is_empty())
}

fn is_removable(inst: u32) -> bool {
    let (mut status, mut problem) = (CM_DEVNODE_STATUS_FLAGS(0), CM_PROB(0));
    let got = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, inst, 0) };
    got == CR_SUCCESS && status.0 & DN_REMOVABLE.0 != 0
}

/// The disk's node and every ancestor up to the root: (node, instance ID).
fn chain(disk: u32) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let mut at = disk;
    // Deep enough for any real tree, short enough never to loop.
    for _ in 0..32 {
        let Some(id) = device_id(at) else { break };
        out.push((at, id));
        let mut parent = 0u32;
        if unsafe { CM_Get_Parent(&mut parent, at, 0) } != CR_SUCCESS || parent == at {
            break;
        }
        at = parent;
    }
    out
}

/// What a disk resolved to: the node Eject asks for, and how it is
/// connected.
struct Resolved {
    key: String,
    ids: Vec<String>,
    usb: Option<Bus>,
}

/// The first removable node from the disk up (the disk itself included) is
/// the one to eject; a disk without one is internal. `usb_wanted`: probe the
/// USB speed too.
fn resolve(disk: u32, usb_wanted: bool) -> Option<Resolved> {
    let nodes = chain(disk);
    let at = nodes.iter().position(|(inst, _)| is_removable(*inst))?;
    let ids: Vec<String> = nodes.iter().map(|(_, id)| id.to_uppercase()).collect();
    let usb = if usb_wanted { usb_speed(&nodes) } else { None };
    Some(Resolved {
        key: nodes[at].1.clone(),
        ids,
        usb,
    })
}

/// `GENERIC_WRITE`, which the hub's control interface wants.
const GENERIC_WRITE: u32 = 0x4000_0000;

/// USB 3 or USB 2 for the USB device on the chain, asked of the hub it is
/// plugged into (`IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2` on the
/// hub's interface, the port from the device's address). `None` when any
/// step does not work: the page then says plain "USB".
fn usb_speed(nodes: &[(u32, String)]) -> Option<Bus> {
    let at = nodes
        .iter()
        .position(|(_, id)| id.to_uppercase().starts_with("USB\\VID_"))?;
    let (device, _) = nodes[at];
    let (_, hub_id) = nodes.get(at + 1)?;
    let mut port = 0u32;
    let mut len = size_of::<u32>() as u32;
    if unsafe {
        CM_Get_DevNode_Registry_PropertyW(
            device,
            CM_DRP_ADDRESS,
            None,
            Some((&mut port as *mut u32).cast()),
            &mut len,
            0,
        )
    } != CR_SUCCESS
        || port == 0
    {
        return None;
    }
    let mut paths = [0u16; 512];
    if unsafe {
        CM_Get_Device_Interface_ListW(
            &GUID_DEVINTERFACE_USB_HUB,
            &HSTRING::from(hub_id.as_str()),
            &mut paths,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    } != CR_SUCCESS
    {
        return None;
    }
    let path = wide_text(&paths);
    if path.is_empty() {
        return None;
    }
    let hub = open(&HSTRING::from(path), GENERIC_WRITE)?;
    let mut info = USB_NODE_CONNECTION_INFORMATION_EX_V2 {
        ConnectionIndex: port,
        Length: size_of::<USB_NODE_CONNECTION_INFORMATION_EX_V2>() as u32,
        SupportedUsbProtocols: USB_PROTOCOLS { ul: 0b100 },
        ..Default::default()
    };
    let info_ptr: *mut c_void = (&mut info as *mut USB_NODE_CONNECTION_INFORMATION_EX_V2).cast();
    ioctl(
        &hub,
        IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2,
        info_ptr,
        size_of::<USB_NODE_CONNECTION_INFORMATION_EX_V2>(),
        info_ptr,
        size_of::<USB_NODE_CONNECTION_INFORMATION_EX_V2>(),
    )?;
    // Bit 0: the device operates at SuperSpeed or higher.
    let flags = info.Flags;
    Some(if unsafe { flags.ul } & 1 != 0 {
        Bus::Usb3
    } else {
        Bus::Usb2
    })
}

// ---- Scanning ----

/// How a device is connected: a card reader by its kind, Thunderbolt or USB4
/// by an ancestor's ID, USB by the speed probe (plain USB when it did not
/// work), else unknown.
fn bus_of(kind: DeviceKind, bus_type: u32, resolved: &Resolved) -> Bus {
    if kind == DeviceKind::Sd {
        return Bus::CardReader;
    }
    if resolved
        .ids
        .iter()
        .any(|id| id.contains("THUNDERBOLT") || id.contains("USB4"))
    {
        return Bus::Thunderbolt;
    }
    if bus_type == BUS_TYPE_USB {
        return resolved.usb.unwrap_or(Bus::Usb);
    }
    Bus::Other
}

/// `STORAGE_BUS_TYPE` values of disks built into the machine: ATA, RAID, SAS
/// and SATA. NVMe (0x11) is not among them: a Thunderbolt 3 enclosure's chain
/// is plain PCI nodes with no ID that says so, and it would vanish.
const INTERNAL_BUSES: [u32; 4] = [0x3, 0x8, 0xA, 0xB];

/// A disk on an internal bus with nothing external (USB, Thunderbolt, USB4,
/// an SD host) on its chain: a hot-plug SATA port flags it removable (why
/// Windows' own tray offers to eject it), but it is still inside the machine.
/// `ids` are upper case.
fn internal_disk(bus_type: u32, ids: &[String]) -> bool {
    INTERNAL_BUSES.contains(&bus_type)
        && !ids.iter().any(|id| {
            id.starts_with("USB")
                || id.starts_with("SD\\")
                || id.starts_with("SDBUS\\")
                || id.contains("THUNDERBOLT")
                || id.contains("USB4")
        })
}

/// The disk holding the Windows directory, by its disk number.
fn system_disk(vols: &[Volume]) -> Option<u32> {
    let mut buf = [0u16; 261];
    let len = unsafe { GetWindowsDirectoryW(Some(&mut buf)) } as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    let letter = char::from_u32(u32::from(buf[0]))?.to_ascii_uppercase();
    vols.iter().find(|v| v.letter == letter).map(|v| v.number)
}

/// Whether the volume at `letter` holds a page file (at its root, where
/// Windows always puts one).
fn holds_page_file(letter: char) -> bool {
    let path = HSTRING::from(format!("{letter}:\\pagefile.sys"));
    let attributes = unsafe { GetFileAttributesW(&path) };
    // Read at once: nothing in between may reset the thread's last error.
    let error = unsafe { GetLastError() };
    page_file_present(attributes, error)
}

/// What `GetFileAttributesW` on `X:\pagefile.sys` says about the page file.
/// A page file in use is refused with a sharing violation (checked live:
/// `C:\pagefile.sys` gives `INVALID_FILE_ATTRIBUTES` and error 32 while
/// Windows pages to it), and access denied also means the entry is there.
/// Any other failure (not found, path not found, not ready) means absent.
fn page_file_present(attributes: u32, error: WIN32_ERROR) -> bool {
    attributes != INVALID_FILE_ATTRIBUTES
        || error == ERROR_SHARING_VIOLATION
        || error == ERROR_ACCESS_DENIED
}

/// The drives plugged in now. Capacity is read as `capacity` says; what is
/// not read is carried over from `last` for the same device and letter.
fn scan(capacity: Capacity, last: &[Device]) -> Vec<Device> {
    let vols = volumes();
    if vols.is_empty() {
        return Vec::new();
    }
    let nodes = disk_nodes();
    // Disks never listed, whatever their nodes say: the one Windows runs
    // from, and any holding a page file (found below).
    let mut pinned: HashSet<u32> = system_disk(&vols).into_iter().collect();
    // Per disk: where to eject and how it is connected, worked out once.
    let mut resolved: HashMap<u32, Option<Resolved>> = HashMap::new();
    // The volumes to list, with their label and file system. A page file on
    // any volume pins its whole disk, so the list is built after all of them.
    let mut listed: Vec<(Volume, String, String)> = Vec::new();
    for vol in vols {
        if pinned.contains(&vol.number) {
            continue;
        }
        let entry = resolved.entry(vol.number).or_insert_with(|| {
            let node = *nodes.get(&vol.number)?;
            resolve(node, vol.desc.bus_type == BUS_TYPE_USB)
                .filter(|found| !internal_disk(vol.desc.bus_type, &found.ids))
        });
        if entry.is_none() {
            continue;
        }
        // Only a candidate is asked for its label: a card slot with no card
        // says "not ready" and is left out.
        let root = HSTRING::from(format!("{}:\\", vol.letter));
        let Ok((label, fs)) = volume_info(&root) else {
            continue;
        };
        if holds_page_file(vol.letter) {
            pinned.insert(vol.number);
            continue;
        }
        listed.push((vol, label, fs));
    }
    let mut devices: Vec<Device> = Vec::new();
    for (vol, label, fs) in listed {
        if pinned.contains(&vol.number) {
            continue;
        }
        let Some(Some(found)) = resolved.get(&vol.number) else {
            continue;
        };
        let known = last.iter().find(|d| d.key == found.key);
        let read = match capacity {
            Capacity::All => true,
            Capacity::NewOnly => known.is_none(),
            Capacity::Keep => false,
        };
        let (total, free) = if read {
            read_capacity(vol.letter).map_or((0, None), |(t, f)| (t, Some(f)))
        } else {
            known
                .and_then(|d| d.partitions.iter().find(|p| p.letter == vol.letter))
                .map_or((0, None), |p| (p.total, p.free))
        };
        let partition = Partition {
            letter: vol.letter,
            label: label.clone(),
            fs,
            total,
            free,
        };
        if let Some(device) = devices.iter_mut().find(|d| d.key == found.key) {
            device.partitions.push(partition);
            continue;
        }
        let kind = classify(vol.desc.bus_type, vol.desc.removable, &vol.desc.product);
        let model = vendor_product_name(&vol.desc.vendor, &vol.desc.product);
        // Empty when there is neither: the kind is named when drawn.
        let name = if model.is_empty() {
            label
        } else {
            model.clone()
        };
        devices.push(Device {
            key: found.key.clone(),
            name,
            model,
            kind,
            bus: bus_of(kind, vol.desc.bus_type, found),
            partitions: vec![partition],
        });
    }
    devices
}

// ---- Eject ----

/// Asks Windows to remove the device node `key` (its instance ID), looked up
/// again now: a node number kept from a scan may belong to another device by
/// the time of the click. Passing a veto type and a name buffer also keeps
/// Windows from showing its own dialog. A device that is no longer there
/// counts as removed.
///
/// Two guards keep a wrong key from reaching Windows' eject. An empty ID
/// would be looked up as the *root* of the device tree, so it is refused
/// before any call; and only a node that is still flagged removable is ever
/// asked (the root and every internal node are not).
fn eject(key: &str) -> Result<(), Veto> {
    let refused = || Veto {
        kind: devices::VetoKind::Unknown,
        name: String::new(),
    };
    if key.trim().is_empty() {
        return Err(refused());
    }
    let mut inst = 0u32;
    let located =
        unsafe { CM_Locate_DevNodeW(&mut inst, &HSTRING::from(key), CM_LOCATE_DEVNODE_NORMAL) };
    if located != CR_SUCCESS {
        // CR_NO_SUCH_DEVNODE and CR_NO_SUCH_VALUE: gone already.
        return Ok(());
    }
    if !is_removable(inst) {
        return Err(refused());
    }
    let mut veto = PNP_VETO_TYPE(0);
    let mut name = [0u16; 260];
    let result = unsafe { CM_Request_Device_EjectW(inst, Some(&mut veto), Some(&mut name), 0) };
    eject_outcome(result == CR_SUCCESS, veto.0, &wide_text(&name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS};

    #[test]
    fn a_page_file_in_use_counts_as_present_though_its_attributes_are_refused() {
        // A live page file: Windows refuses the call.
        assert!(page_file_present(
            INVALID_FILE_ATTRIBUTES,
            ERROR_SHARING_VIOLATION
        ));
        assert!(page_file_present(
            INVALID_FILE_ATTRIBUTES,
            ERROR_ACCESS_DENIED
        ));
        // A page file not in use reads normally (hidden, system).
        assert!(page_file_present(0x6, ERROR_SUCCESS));
        // No page file on that volume.
        assert!(!page_file_present(
            INVALID_FILE_ATTRIBUTES,
            ERROR_FILE_NOT_FOUND
        ));
        assert!(!page_file_present(
            INVALID_FILE_ATTRIBUTES,
            ERROR_PATH_NOT_FOUND
        ));
        assert!(!page_file_present(INVALID_FILE_ATTRIBUTES, ERROR_NOT_READY));
    }

    fn scan_req(seq: u64, capacity: Capacity) -> Req {
        Req::Scan { seq, capacity }
    }

    #[test]
    fn queued_scans_merge_into_one_with_the_newest_number_and_the_widest_read() {
        let out = merge(vec![
            scan_req(3, Capacity::Keep),
            scan_req(5, Capacity::All),
            scan_req(4, Capacity::NewOnly),
        ]);
        assert_eq!(out.len(), 1);
        assert!(matches!(
            out[0],
            Req::Scan {
                seq: 5,
                capacity: Capacity::All
            }
        ));
    }

    #[test]
    fn an_eject_between_scans_keeps_their_order() {
        let out = merge(vec![
            scan_req(1, Capacity::Keep),
            Req::Eject {
                keys: vec!["A".into()],
            },
            scan_req(2, Capacity::NewOnly),
            scan_req(3, Capacity::Keep),
        ]);
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], Req::Scan { seq: 1, .. }));
        assert!(matches!(out[1], Req::Eject { .. }));
        assert!(matches!(
            out[2],
            Req::Scan {
                seq: 3,
                capacity: Capacity::NewOnly
            }
        ));
    }

    /// A device that is not there is gone already: nothing is asked of
    /// Windows beyond looking the made-up ID up, so this ejects nothing real.
    #[test]
    fn ejecting_a_device_that_is_not_there_counts_as_removed() {
        assert_eq!(eject("USB\\VID_0000&PID_0000\\NO-SUCH-DEVICE-0000"), Ok(()));
    }

    /// An empty ID is the root of the device tree to `CM_Locate_DevNodeW`:
    /// it is refused before Windows is asked anything.
    #[test]
    fn an_empty_key_is_refused_and_never_looked_up() {
        let refused = Err(Veto {
            kind: devices::VetoKind::Unknown,
            name: String::new(),
        });
        assert_eq!(eject(""), refused);
        assert_eq!(eject("   "), refused);
    }

    /// Looks at this machine's drives (read-only: handles opened for no
    /// access and closed again, sizes read) and prints them. Run by hand:
    /// `cargo test live_scan -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads this machine's drives"]
    fn live_scan_lists_only_removable_drives() {
        // How far each local volume gets: its disk, the disk's device node
        // and whether anything above it is removable.
        let nodes = disk_nodes();
        for v in volumes() {
            let node = nodes.get(&v.number).copied();
            let chain = node.map(chain).unwrap_or_default();
            let removable = chain.iter().position(|(inst, _)| is_removable(*inst));
            println!(
                "{}: disk {} bus {} node {:?} chain {} removable at {:?}",
                v.letter,
                v.number,
                v.desc.bus_type,
                node,
                chain.len(),
                removable
            );
        }
        let found = scan(Capacity::All, &[]);
        for d in &found {
            println!("{d:#?}");
            assert!(!d.key.is_empty() && !d.partitions.is_empty());
        }
        // A second scan that keeps the sizes finds the same drives.
        let again = scan(Capacity::Keep, &found);
        assert_eq!(found, again);
        println!("{} removable drive(s)", found.len());
    }

    #[test]
    fn a_hot_plug_sata_disk_with_nothing_external_above_it_is_internal() {
        let ids = |list: &[&str]| -> Vec<String> { list.iter().map(|s| (*s).to_owned()).collect() };
        let (sata, nvme) = (0xB, 0x11);
        let inside = ids(&[
            "SCSI\\DISK&VEN_WDC",
            "PCIIDE\\IDECHANNEL",
            "PCI\\VEN_8086&DEV_A352",
        ]);
        assert!(internal_disk(sata, &inside));
        assert!(internal_disk(0x3, &inside));
        // Behind USB (an eSATA-to-USB bridge), Thunderbolt or USB4: external.
        assert!(!internal_disk(
            sata,
            &ids(&["USBSTOR\\DISK", "USB\\VID_152D"])
        ));
        assert!(!internal_disk(
            sata,
            &ids(&["SCSI\\DISK", "PCI\\THUNDERBOLT_NHI"])
        ));
        assert!(!internal_disk(sata, &ids(&["SCSI\\DISK", "USB4\\ROUTER"])));
        // USB and SD buses are never internal; NVMe is left to the node flag.
        assert!(!internal_disk(BUS_TYPE_USB, &inside));
        assert!(!internal_disk(devices::BUS_TYPE_SD, &inside));
        assert!(!internal_disk(nvme, &inside));
    }

    #[test]
    fn capacity_orders_from_nothing_to_everything() {
        assert!(Capacity::Keep < Capacity::NewOnly);
        assert!(Capacity::NewOnly < Capacity::All);
    }

    #[test]
    fn text_is_read_up_to_its_nul_and_trimmed() {
        let bytes = b"\0\0SanDisk \0Ultra\0";
        assert_eq!(text_at(bytes, 2), "SanDisk");
        assert_eq!(text_at(bytes, 0), "");
        assert_eq!(text_at(bytes, 99), "");
        assert_eq!(wide_text(&[b'E' as u16, b':' as u16, 0, b'x' as u16]), "E:");
        assert_eq!(wide_text(&[]), "");
    }

    #[test]
    fn the_bus_comes_from_the_kind_the_ancestors_and_the_probe() {
        let node = |usb: Option<Bus>, ids: &[&str]| Resolved {
            key: "k".into(),
            ids: ids.iter().map(|s| (*s).to_owned()).collect(),
            usb,
        };
        let plain = node(None, &["USBSTOR\\DISK", "USB\\VID_1"]);
        assert_eq!(
            bus_of(DeviceKind::Sd, BUS_TYPE_USB, &plain),
            Bus::CardReader
        );
        assert_eq!(bus_of(DeviceKind::Usb, BUS_TYPE_USB, &plain), Bus::Usb);
        let fast = node(Some(Bus::Usb3), &["USB\\VID_1"]);
        assert_eq!(bus_of(DeviceKind::Hdd, BUS_TYPE_USB, &fast), Bus::Usb3);
        let tb = node(None, &["SCSI\\DISK", "PCI\\THUNDERBOLT_NHI"]);
        assert_eq!(bus_of(DeviceKind::Hdd, 0x11, &tb), Bus::Thunderbolt);
        let usb4 = node(None, &["USB4\\ROUTER"]);
        assert_eq!(bus_of(DeviceKind::Hdd, 0x11, &usb4), Bus::Thunderbolt);
        assert_eq!(bus_of(DeviceKind::Hdd, 0x11, &plain), Bus::Other);
    }
}
