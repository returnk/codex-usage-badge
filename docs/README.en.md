<p align="center"><img src="images/brand.svg" width="72" alt="Codex Badge"></p>

<h1 align="center">Codex Badge · 余量</h1>

<p align="center"><strong>Your Codex quota, at a glance.</strong></p>
<p align="center">A Windows sidebar badge for Codex usage limits, reset times, and reset credits.</p>

<p align="center"><a href="../README.md">简体中文</a> · <a href="#download">Download</a> · <a href="#get-started">Get started</a> · <a href="https://github.com/returnk/codex-usage-badge/issues">Feedback</a></p>

Keep your remaining quota in view without leaving your work. The badge sits alongside Codex; hover to see your 5-hour and weekly remaining limits, reset times, and available reset credits.

<p align="center">
  <img src="images/tauri-preview.svg" width="1100" alt="Tauri UI illustration: Glass, System light, and System dark; sample remaining quota and reset times">
  <br><sub>UI illustration with sample data · Two theme choices: Glass / System</sub>
</p>

## Download

**v0.3.5** · Windows 10 / 11 x64 · Release

[**Installer (recommended)**](https://github.com/returnk/codex-usage-badge/releases/download/v0.3.5/Codex.Badge_0.3.5_x64-setup.exe) · [Portable ZIP](https://github.com/returnk/codex-usage-badge/releases/download/v0.3.5/CodexBadge-v0.3.5-windows-x64-portable.zip) · [Release notes](https://github.com/returnk/codex-usage-badge/releases/tag/v0.3.5) · [SHA-256](https://github.com/returnk/codex-usage-badge/releases/download/v0.3.5/SHA256SUMS.txt)

Requires WebView2 Runtime, the signed-in Codex Windows desktop app, and a working Codex CLI. The installer can download WebView2 if needed. No .NET runtime required. The app is unsigned; Windows may show a security warning. Download only from this repository.

## Get started

1. Install, or extract the ZIP and run `codex-badge-tauri.exe`.
2. Open Codex. The badge appears automatically.
3. Right-click the badge or tray icon to enable launch at login or always-on-top mode.

Leave Codex Badge running in the background to follow Codex opening and closing. Choosing **退出** (Exit) stops the watcher until you launch the badge again.

## Everyday controls

| Action | Result |
| --- | --- |
| Hover | Show remaining quota and reset times |
| Click **查看** | Show reset-credit expiry details, when available |
| Mouse wheel | Switch **Glass / System**; System follows Windows light/dark appearance |
| Drag | Move the badge; each display mode remembers its own position |
| Right-click | Always-on-top, Settings submenu, or Exit; Settings contains launch at login, expiring-card automation, and Check for updates |
| Double-click | Restore the default position for the current mode |

Normal mode hides when Codex is minimized. Always-on-top mode remains visible while Codex is minimized. Both modes hide when Codex exits. The current app interface is in Chinese.

## Help

- **Badge missing?** Check that Codex Badge is running and the ChatGPT/Codex window is open. v0.3.3 adapts anchor discovery to the new ChatGPT layout. Double-click the badge to restore its default position.
- **`--%`?** Check your Codex sign-in and CLI. If the CLI cannot be found, set `CODEX_BADGE_CLI` to the full path of `codex.exe`.
- **Updating or moving the portable app?** Disable launch at login, exit the running copy, then launch the new copy and re-enable it if needed. When upgrading from v0.2.x, configure your theme and position again.

This update adds opt-in expiring-card automation, improves rounded edges and account-consent UI, and avoids overlap with client update controls. Automated checks and user testing do not establish multi-PC, mixed-DPI, or full install/upgrade acceptance. See the [release notes](../CodexBadge.Tauri/docs/releases/v0.3.5.md).

## Privacy and feedback

Reads quota through the local `codex app-server`; official redemption requests are sent only after explicit account authorization for expiring-card automation. It does not read `auth.json`, send model requests, or upload login credentials. Settings and diagnostics stay in `%LOCALAPPDATA%\CodexBadge`.

[Report an issue](https://github.com/returnk/codex-usage-badge/issues/new/choose) with steps to reproduce, display scaling, and theme. Remove personal information and credentials from screenshots and logs.

[Build from source](../CodexBadge.Tauri/README.md) · [MIT License](../LICENSE)

<sub>Independent open-source tool. Not an official OpenAI product.</sub>

## v0.3.5 updates

Expiring-card automation is off by default. After explicit per-account consent, Codex Badge can attempt eligible free-card redemption within 30 minutes of expiry, including future cards, until disabled. Keep the computer and Codex Badge running; the Codex window need not stay open. Shutdown, sleep, or exiting the app stops execution. Success is not guaranteed.

Existing quota is not added to the reset quota, reset times may change, and successful redemption cannot be undone. Do not enable automation for the same account on multiple devices or tools. Disabling stops new requests; an already-sent request cannot be guaranteed canceled. Uncertain results pause automation for review.

Themed consent, immediate loading feedback, compact tooltips, capsule-edge rendering, quota recovery handling, and client-update avoidance are improved. Installed copies support user-initiated signed updates; portable copies use the ZIP. No automatic update checks, downloads, or installation. Updater signatures are separate from Windows Authenticode signing.

If Codex Badge helps your workflow, a Star is appreciated.
