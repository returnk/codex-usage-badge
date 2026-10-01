# 余量 · Codex Badge — Tauri development

当前产品实现：**Tauri 2 + Rust + 系统 WebView2**。当前公开版本为 **v0.3.1**。下载与日常使用见[项目首页](../README.md)；本页面向开发者，记录构建、诊断与验证信息。

Tauri 2 + Rust + system WebView2 implementation for Windows 10/11 x64. This directory is independent of the WPF source in `src/CodexBadge`.

## Data and behavior

- When Codex is running, starts `codex app-server --stdio`, initializes the protocol, checks `account/read` with `refreshToken:false`, reads `account/rateLimits/read` immediately and every 60 seconds, and refreshes after quota/account notifications. The manual refresh action has been removed; automatic refresh and failure backoff remain.
- Uses only the app-server response. It does not read `auth.json` or use the wham endpoint. Failed reads retry after 15, 30, 60, then 300 seconds. Cached data older than 30 minutes is unavailable.
- Maps `usedPercent` to remaining percent for 300-minute and 10080-minute windows. The reset count uses `availableCount`, including when expiry details are absent. Unknown count is shown as unknown, not zero; View is hidden without real unexpired details. Known details must be `available` and unexpired and are sorted by expiration. Expiry of a known row reduces the cached count until the next read. Failed refreshes visibly mark cached data as pending update.
- Defaults to Glass, with a System theme that follows Windows light/dark appearance. The wheel switches themes. Normal mode uses a compact numeric circle; global mode uses a capsule with a small percent unit. Both modes support drag and double-click position reset.
- Saves theme, normal anchor-relative DIP offset, independent global work-area position, startup preference, notifications and reminder deduplication state in `%LOCALAPPDATA%\CodexBadge\tauri-settings.json`. Capsule and tray share a native three-item menu: topmost, Settings submenu, and Exit. Settings contains launch at login and optional Windows notifications. Native menus and hints use premultiplied-alpha buffered rendering with UpdateLayeredWindow; no WebView cold creation is needed for tray menus.
- Discovers the new ChatGPT navigation column through visible profile-menu controls and validates cached avatar/column geometry; the old Voice sidebar remains a compatibility path. Normal placement centers the 30x30 DIP frame above the avatar with a 10 DIP gap. Global placement uses a 72x34 DIP frame and independent saved position. Window changes remain nonactivating and work-area constrained.
- All windows are nonactivating tool windows. Normally owned by Codex; **置顶模式** detaches them for global topmost behavior while Codex is running. Minimize hides normal mode and retains global mode. Closing Codex now hides the badge in both modes, stops app-server and destroys WebViews after a two-second window-recreation grace period. Only tray and a lightweight Rust watcher remain; reopening Codex recreates the badge automatically. WebViews are not created at login while Codex is absent. A tray menu is created on demand even while waiting.

## Build

Install Rust MSVC, Visual Studio C++ Build Tools, Node.js, and system WebView2. From this directory:

```powershell
npm ci
node --test tests/frontend.test.cjs
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
```

The Release EXE is `src-tauri/target/release/codex-badge-tauri.exe`; the NSIS installer is under `src-tauri/target/release/bundle/nsis/`. The installer uses the system WebView2 runtime or its download bootstrapper, never the full offline runtime.

Local short-duration measurements found about 612 MiB of summed working sets with Codex open versus 262 MiB for the older WPF reference; waiting mode was about 15 MiB. Shared pages can be counted more than once, and the measurements are not a controlled long-term comparison. Small packaging does not mean small active memory.

## Targeted diagnostics

The app records window/tray events, UIA discovery duration, lifecycle and sanitized RPC diagnostics in `%LOCALAPPDATA%\CodexBadge\tauri-diagnostics.log`. RPC entries contain method, category, numeric error code and duration; account/response values, stderr contents, raw error messages and tokens are omitted. The log rotates at about 1 MB. `--diagnose-no-native-region` compares the same render without GDI clipping. `--diagnose-no-codex` exercises the idle watcher without closing your actual Codex app. Both flags are process-only and do not change saved settings.

Dragging skips unchanged positions and pauses UIA probing; the native region updates when physical dimensions change rather than on every release. The log records drag start/end and requested/actual HWND positions.

After a missing tray menu or a failed topmost switch, note the approximate clock time and inspect the nearby `tray_right`, `menu_request`, `menu_raise`, `menu_observe`, and `topmost_*` entries. A tray menu is temporarily topmost and unowned so it is visible above the foreground app; its checkmark comes from persisted state. The menu now gives the pointer time to travel from the tray icon into the popup; after entering, it closes about 300 ms after leaving.

For a new failure, record the time, display scaling, Glass/System choice, whether topmost is enabled, and the shortest steps to reproduce. The nearby log entries identify quota RPC stages, native mode requests, menu visibility and drag coordinates. Historic quota failures cannot be assigned a cause when their original errors were discarded.

## Desktop acceptance

v0.3.1 was checked on the current 125% desktop: normal/global capsules, left/right submenus, the Explorer tray icon, hover hints, reset credits, closing and focus. Four additional renderer regressions include an actual Win32 atomic-frame/no-activation check. Other DPI settings have geometry and coverage checks only; full multi-monitor, upgrade, IME and long-running lifecycle acceptance remains open.

The app checks owner visibility when disabling topmost. Both modes double-click to reset their own position. Native menus now submit complete frames atomically and share smooth alpha edges. Continuous menu sampling found no root-card jumps, hidden frames or black frames; the reported transient black window was not directly reproduced. `--diagnose-no-hover-detail` and `--diagnose-default-cursor` remain opt-in diagnostics. Cold-wait checks do not prove real client exit/reopen resource release.

Close the WPF `CodexBadge.exe` before visual acceptance. Check capsule edge, centering and visible size at 100%, 125%, 150%, and 175% DPI; initial appearance; ordinary/maximized/restored Codex; sidebar resizing; cross-monitor moves; drag to every screen edge and after a monitor is removed; double-click; repeated tray right-clicks; hover and reset-credit flyout closing after moving outside all three badge windows; Glass/System and OS light/dark changes; topmost on/off over another app and while Codex minimizes or closes; taskbar/Alt+Tab/focus/ownership; and performance during maximize and idle. Automated checks and one launch cannot establish these desktop behaviors.
