# 余量 · Codex Badge — Tauri development

当前产品实现：**Tauri 2 + Rust + 系统 WebView2**。当前版本为 **v0.3.5**。下载与日常使用见[项目首页](../README.md)；本页面向开发者，记录构建、诊断与验证信息。

Tauri 2 + Rust + system WebView2 implementation for Windows 10/11 x64. This directory is independent of the WPF source in `src/CodexBadge`.

## Data and behavior

- When Codex is running, or local automatic reset permission is enabled, starts `codex app-server --stdio`, initializes the protocol, checks `account/read` with `refreshToken:false`, reads `account/rateLimits/read` immediately and every 60 seconds, and refreshes after quota/account notifications. The manual refresh action has been removed; automatic refresh and failure backoff remain.
- Uses only the app-server response. It does not read `auth.json` or use the wham endpoint. Failed reads retry after 15, 30, 60, then 300 seconds. Opening details requests a refresh when the last read failed or is at least 15 seconds old; repeated openings are throttled. Cached data older than 30 minutes is unavailable.
- Maps `usedPercent` to remaining percent for 300-minute and 10080-minute windows. The reset count uses `availableCount`, including when expiry details are absent. Unknown count is shown as unknown, not zero; View is hidden without real unexpired details. Known details must be `available` and unexpired and are sorted by expiration. Expiry of a known row reduces the cached count until the next read. Failed refreshes visibly mark cached data as pending update.
- Defaults to Glass, with a System theme that follows Windows light/dark appearance. The wheel switches themes. Normal mode uses a compact numeric circle; global mode uses a capsule with a small percent unit. Both modes support drag and double-click position reset.
- Saves theme, normal anchor-relative DIP offset, independent global work-area position, startup preference and reminder-view deduplication state in `%LOCALAPPDATA%\CodexBadge\tauri-settings.json`. Capsule and tray share a native three-item menu: topmost, Settings submenu, and Exit. Settings contains launch at login, 临期用卡 (a right-side checkmark), and Check for updates. Expiry Windows balloons and their separate notification switch are removed; the capsule dot and card expiry details remain. Native menus and hints use premultiplied-alpha buffered rendering with UpdateLayeredWindow; no WebView cold creation is needed for tray menus.
- Discovers the new ChatGPT navigation column through visible profile-menu controls and validates cached avatar/column geometry; the old Voice sidebar remains a compatibility path. Normal placement avoids the desktop update action in ready, downloading (including percentage) and installing states, even when its UIA role changes from button to progress bar. Without that action, it centers the 30x30 DIP frame above the avatar with a 10 DIP gap. Global placement uses a 72x34 DIP frame and independent saved position. Window changes remain nonactivating and work-area constrained.
- Badge overlays are nonactivating tool windows; user-opened update and authorization windows can take focus. Overlays are normally owned by Codex; **置顶模式** detaches them for global topmost behavior while Codex is running. Minimize hides normal mode and retains global mode. Closing Codex hides the badge in both modes and destroys its WebViews after a two-second window-recreation grace period. Without automatic-reset permission, app-server stops and only tray and a lightweight Rust watcher remain; with permission the same quota worker remains available without creating badge WebViews; reopening Codex recreates the badge automatically. Badge WebViews are not created at login while Codex is absent. A tray menu is created on demand even while waiting.

当前完整功能统一为 0.3.5。本次发布包来自 `artifacts/release-v0.3.5-ui-polish/packages/`；此前 `release-v0.3.5/`、`release-v0.3.5-unified/` 和 0.3.6/0.3.7 均为历史本地构建，保留原包和记录，不再作为当前交付。

## 临期用卡（v0.3.5）

默认关闭，升级不代表授权。在设置中点击“临期用卡”，菜单立即显示处理中；只读取得当前 ChatGPT 账号后，通过同主题授权卡片确认持续授权，默认焦点为“取消”。保存成功才勾选，随后继续后台检查；取消或关闭不会开启，结束后恢复设置菜单。无需逐次确认，固定在卡到期前 30 分钟内按条件尝试。电脑和 CodexBadge 须运行，不要求 Codex 窗口打开；关机、休眠或退出时不执行，不安装额外服务或计划任务。

账号/计划/响应形态不兼容时重新授权；同邮箱账号的执行账本保守归组，重新授权不会清空原卡阶段限制或未决状态。使用显式 creditId，发送前同步写入幂等记录；本机跨进程文件互斥后重新读取记录和账号/额度/卡。原卡到期前不换下一张，成功但额度仍旧也适用。未知结果只在约 60/300 秒复核原卡原键两次，仍未知或已过期就暂停，需核查官方状态；开关不清除该暂停。

关闭立即阻止本会话新请求。保存关闭失败会明确显示“关闭未保存”，保留勾选以便重试；重启可能恢复旧授权。已发送请求仍接收答复，关闭后不重发 consume；先前超时且无法只读确认的结果标为未决并暂停，不能从卡消失推定为本程序已用卡。

悬停“临期用卡”查看状态、最近结果和时间。Windows 只提交自动用卡成功或需处理失败的结果通知，每事件最多一次；系统可能不显示。软件内及本地记录仍保留结果，不受旧通知开关影响。完整消费/无窗口后台/退出唤醒/实际升级仍待用户实机验收，模拟不等于真实用卡成功。

## Build

Signed release packaging and upload: [release workflow](docs/release.md). Select checks for the changed code; the commands below are available checks, not a mandatory sequence for every task. Full automated checks run in CI.

Install Rust MSVC, Visual Studio C++ Build Tools, Node.js, and system WebView2. From this directory:

```powershell
npm ci
node --test tests/*.test.cjs
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

## Local cached data

Local cached records can include opaque account fingerprints, quota/window values and reset-credit details. No credentials are stored. The private automatic-reset journal (`expiring-reset.json`) also contains the authorized account identity/plan, exact card IDs, request keys, phase guards and last result. Do not upload this journal. Diagnostics exclude account plaintext, card IDs and request keys. All data remains in the local CodexBadge directory.

## Desktop acceptance

v0.3.2 was checked on the current 125% desktop: normal/global capsules, left/right submenus, the Explorer tray icon, hover hints, reset credits, closing and focus. Four additional renderer regressions include an actual Win32 atomic-frame/no-activation check. Other DPI settings have geometry and coverage checks only; full multi-monitor, upgrade, IME and long-running lifecycle acceptance remains open.

The app checks owner visibility when disabling topmost. Both modes double-click to reset their own position. Native menus now submit complete frames atomically and share smooth alpha edges. Continuous menu sampling found no root-card jumps, hidden frames or black frames; the reported transient black window was not directly reproduced. `--diagnose-no-hover-detail` and `--diagnose-default-cursor` remain opt-in diagnostics. Cold-wait checks do not prove real client exit/reopen resource release.

Close the WPF `CodexBadge.exe` before visual acceptance. Check capsule edge, centering and visible size at 100%, 125%, 150%, and 175% DPI; initial appearance; ordinary/maximized/restored Codex; sidebar resizing; cross-monitor moves; drag to every screen edge and after a monitor is removed; double-click; repeated tray right-clicks; hover and reset-credit flyout closing after moving outside all three badge windows; Glass/System and OS light/dark changes; topmost on/off over another app and while Codex minimizes or closes; taskbar/Alt+Tab/focus/ownership; and performance during maximize and idle. Automated checks and one launch cannot establish these desktop behaviors.
