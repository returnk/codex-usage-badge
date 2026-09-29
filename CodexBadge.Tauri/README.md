# Codex Badge for Windows (Tauri 2)

中文展示名：**余量 · Codex Badge**。当前公开版本为 **v0.3.0-beta.1**，属于 Tauri 重构测试版，不替代旧 WPF 稳定版。下载、迁移和已知问题见 [发布说明](docs/releases/v0.3.0-beta.1.md)。

Tauri 2 + Rust + system WebView2 implementation for Windows 10/11 x64. This directory is independent of the WPF source in `src/CodexBadge`.

## Data and behavior

- When Codex is running, starts `codex app-server --stdio`, initializes the protocol, checks `account/read` with `refreshToken:false`, reads `account/rateLimits/read` immediately and every 60 seconds, and refreshes after quota/account notifications. The manual refresh action has been removed; automatic refresh and failure backoff remain.
- Uses only the app-server response. It does not read `auth.json` or use the wham endpoint. Failed reads retry after 15, 30, 60, then 300 seconds. Cached data older than 30 minutes is unavailable.
- Maps `usedPercent` to remaining percent for 300-minute and 10080-minute windows. The reset count uses `availableCount`, including when expiry details are absent. Unknown count is shown as unknown, not zero; View is hidden without real unexpired details. Known details must be `available` and unexpired and are sorted by expiration. Expiry of a known row reduces the cached count until the next read. Failed refreshes visibly mark cached data as pending update.
- Defaults to **Glass**. The wheel cycles only **Glass / System**, in either direction. System reacts to OS light/dark appearance; its light palette is a warm peach/blue-gray gradient and its dark palette is graphite. Theme names never replace the percentage. Glass has a soft opaque capsule intended to hide the underlying voice text, plus mostly opaque detail surfaces to preserve readability; some official voice text may still remain visible in the current layout. CSS backdrop blur is an enhancement, not verified Windows system acrylic. Earlier Tauri light/dark/privacy settings migrate to System and transparent settings to Glass; WPF settings are not imported. Normal-mode double-click and **重新定位** reset only position; global-mode double-click does not reset. The percentage uses a separate, smaller percent sign; the detail divider is replaced by spacing.
- Saves theme, normal-mode anchor-relative DIP offset, independent global-mode monitor/work-area DIP position, startup preference and **置顶模式** in `%LOCALAPPDATA%\CodexBadge\tauri-settings.json`. Tray and capsule right-click in both modes reuse a four-item custom menu: **开机启动**, **置顶模式**, **重新定位**, **退出**. Browser context menus are disabled in WebView2 and JavaScript. A capsule-origin menu remains open while the pointer rests on the capsule, menu, or narrow shared-edge corridor; after departure it closes about 300 ms later. Tray-origin menus allow 1.2 seconds before first entry. Pending/visible menus suppress detail hover; after closing, leave the capsule before re-entering to open details. Pending display requests expire after 10 seconds if creation/showing cannot complete; displayed menus have no such deadline. Close commands are tied to the opening request. Tauri shares the WPF single-instance mutex.
- Finds named `语音`/`Voice` controls using a UIA name condition and caches the sidebar container. Missing-anchor discovery backs off from 2 to 30 seconds; dragging pauses UIA reads. Initial display waits for two stable physical-pixel readings. Visible capsule size is 67×26 DIP inside a 71×30 DIP transparent HWND. The native region leaves space for the CSS antialiased edge. Native focus-border rendering is disabled; moves use `SWP_NOCOPYBITS`. Window mutations run on the UI thread with at most one queued follow frame. Cross-DPI movement settles monitor changes before setting physical client size; drag offsets use visible origins and the Codex anchor DPI. Work-area clamping keeps the whole capsule on screen.
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

This is a candidate, not an accepted stable replacement. It includes explicit initial focus=false, first-render readiness, validated iconic-host retention, independent global coordinates, popup avoidance and lifecycle generation checks. IME/composition causality, repeated native tray operations, four-DPI/multi-monitor behavior and long performance checks remain acceptance gates.

The beta checks live owner visibility before disabling topmost. Global double-click does not reset position; normal double-click does. Fast-hover visual corruption is not resolved: process-local `--diagnose-no-hover-detail` and `--diagnose-default-cursor` isolate detail triggering versus cursor changes, one at a time, without altering saved settings or native clipping. The default build uses neither flag. Cold-wait process checks do not establish real Codex exit/reopen resource release.

Close the WPF `CodexBadge.exe` before visual acceptance. Check capsule edge, centering and visible size at 100%, 125%, 150%, and 175% DPI; initial appearance; ordinary/maximized/restored Codex; sidebar resizing; cross-monitor moves; drag to every screen edge and after a monitor is removed; double-click; repeated tray right-clicks; hover and reset-credit flyout closing after moving outside all three badge windows; Glass/System and OS light/dark changes; topmost on/off over another app and while Codex minimizes or closes; taskbar/Alt+Tab/focus/ownership; and performance during maximize and idle. Automated checks and one launch cannot establish these desktop behaviors.
