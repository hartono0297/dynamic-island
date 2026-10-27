---
tags: [gotcha, cli, explorer, com, win32]
date: 2026-10-27
---

# Reading File Explorer's folders (COM)

`src/sources/explorer.rs`, used by [[cli-explorer-folder]].

- **Own thread, own apartment.** Each read is a short-lived thread that
  calls `CoInitializeEx(COINIT_APARTMENTTHREADED)`, drops every COM object,
  then `CoUninitialize`. Shell COM never runs on the UI thread or in paint;
  the result comes back as `Event::ExplorerFolders` through the event
  channel and `WM_APP_EVENT`, like the CLI's sink.
- **The path.** `IShellWindows::Item(VARIANT VT_I4 index)` → `IDispatch` →
  `IWebBrowserApp` (for `HWND()`) → `IServiceProvider::QueryService(
  SID_STopLevelBrowser)` → `IShellBrowser::QueryActiveShellView` →
  `IFolderView::GetFolder::<IPersistFolder2>` → `GetCurFolder` (a PIDL, freed
  with `CoTaskMemFree`) → `SHGetPathFromIDListW`. `LocationURL` is not
  enough: it is empty or a `shell:` moniker for many locations.
- **Virtual locations** (Home, This PC, Recycle Bin, Gallery) make
  `SHGetPathFromIDListW` fail: that is how they are dropped.
- **Only Explorer.** `IShellWindows` also lists other shell browsers; the
  top-level window class must be `CabinetWClass`.
- **Z-order.** `IShellWindows` order is not activation order. `EnumWindows`
  walks top-level windows topmost first, which is last-active first; items
  are sorted by their HWND's place in that list.
- **Windows 11 tabs** are separate items sharing one top-level HWND. The
  tab on show is the one whose `IShellBrowser::GetWindow` (its
  `ShellTabWindowClass`) is visible; it sorts first within its window.
- **windows 0.62 features**: `Win32_System_Ole` and `Win32_System_Variant`
  (for `IShellWindows::Item`, `IShellBrowser`, `IOleWindow`, `VARIANT`) and
  `Win32_UI_Shell_Common` (PIDLs). `VARIANT` has no `From<i32>`: set
  `Anonymous.Anonymous.vt = VT_I4` and `.Anonymous.lVal` by hand.
- Not unit-tested (needs live Explorer windows); keep it thin.

Listed in [[index]].
