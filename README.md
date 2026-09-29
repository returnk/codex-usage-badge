<p align="center">
  <img src="docs/images/brand.svg" width="72" alt="余量 · Codex Badge">
</p>

<h1 align="center">余量 · Codex Badge</h1>

<p align="center"><strong>Codex 还剩多少，一眼就知道。</strong></p>
<p align="center">Windows 上的 Codex 额度胶囊 · Codex usage monitor for Windows</p>

<p align="center">
  <a href="https://github.com/returnk/codex-usage-badge/releases/tag/v0.3.0-beta.1"><img src="https://img.shields.io/badge/v0.3.0--beta.1-preview-2879E7" alt="v0.3.0-beta.1 预发布"></a>
  <img src="https://img.shields.io/badge/Windows-10%20%2F%2011-1B2738" alt="Windows 10 / 11 x64">
  <img src="https://img.shields.io/badge/Tauri-2-24C8DB" alt="Tauri 2">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-34C759" alt="MIT license"></a>
</p>

<p align="center">
  <a href="#下载">下载</a> · <a href="#开始使用">开始使用</a> · <a href="#日常操作">日常操作</a> · <a href="https://github.com/returnk/codex-usage-badge/issues">反馈</a> · <a href="docs/README.en.md">English</a>
</p>

把剩余额度留在眼前，把注意力留给工作。**余量**贴合 Codex 侧栏，显示 5 小时剩余百分比；悬停展开本周剩余、重置时间和重置机会，不必切换页面。

<p align="center">
  <img src="docs/images/tauri-preview.svg" width="1100" alt="余量 Tauri 界面示意：毛玻璃、跟随系统浅色、跟随系统深色；显示 5 小时和每周剩余额度、重置时间与重置机会">
  <br>
  <sub>当前界面风格示意 · 示例数据 · 两个主题选项：毛玻璃 / 跟随系统</sub>
</p>

## 下载

**v0.3.0-beta.1** · Windows 10 / 11 x64 · 当前预发布版本

| 安装版 · 推荐 | 便携版 |
| --- | --- |
| [下载安装器](https://github.com/returnk/codex-usage-badge/releases/download/v0.3.0-beta.1/CodexBadge-v0.3.0-beta.1-windows-x64-setup.exe) | [下载 ZIP](https://github.com/returnk/codex-usage-badge/releases/download/v0.3.0-beta.1/CodexBadge-v0.3.0-beta.1-windows-x64-portable.zip) |
| 按提示安装后运行 | 解压后运行 `codex-badge-tauri.exe` |

[更新说明](https://github.com/returnk/codex-usage-badge/releases/tag/v0.3.0-beta.1) · [SHA-256 校验](https://github.com/returnk/codex-usage-badge/releases/download/v0.3.0-beta.1/SHA256SUMS.txt)

无需 .NET。需要 **WebView2 Runtime**、已登录的 Codex Windows 桌面客户端和可用的 Codex CLI；安装器可引导下载 WebView2。程序尚未签名，首次运行可能出现 Windows 安全提示，请确认来自本仓库。

## 开始使用

1. 安装或解压，运行余量。
2. 打开已登录的 Codex，胶囊自动出现。
3. 右键胶囊或托盘图标，可开启**开机启动**和**置顶模式**。

余量在后台运行时，会随 Codex 打开而显示、退出而收起；从托盘选择“退出”后，需要手动重新启动余量。

## 日常操作

| 操作 | 效果 |
| --- | --- |
| 悬停胶囊 | 展开 5 小时、本周剩余和重置时间 |
| 点击“查看” | 查看重置机会的到期时间（有可用详情时显示） |
| 滚动鼠标滚轮 | 切换**毛玻璃 / 跟随系统**；系统主题自动适配明暗 |
| 拖动胶囊 | 调整位置，普通模式和置顶模式分别记忆 |
| 右键胶囊或托盘 | 开机启动、置顶模式、重新定位、退出 |
| 双击胶囊 | 普通模式恢复位置；置顶模式保持位置 |

普通模式随 Codex 最小化隐藏；置顶模式可在 Codex 最小化时继续显示。Codex 退出后，两种模式都会收起。

## 常见问题

**没有看到胶囊？** 确认余量正在运行、Codex 主窗口已经打开。普通模式下，Codex 最小化时不会显示；也可以右键托盘选择“重新定位”。

**显示 `--%`？** 确认 Codex 已登录，且 Codex CLI 可用。找不到 CLI 时，可将环境变量 `CODEX_BADGE_CLI` 设置为 `codex.exe` 的完整路径。

**更新或移动便携版？** 先关闭开机启动，再退出正在运行的版本；运行新版本后按需重新开启。开机启动会记住程序所在路径。

本版仍在测试中，快速来回悬停可能短暂花屏；安装升级、显示缩放及跨屏行为仍在持续验证。[查看发布说明](CodexBadge.Tauri/docs/releases/v0.3.0-beta.1.md)

## 隐私与反馈

通过本机 `codex app-server` 只读获取额度，不读取 `auth.json`、不发送模型请求，不上传登录凭据。设置和诊断日志保存在本机 `%LOCALAPPDATA%\CodexBadge`。

发现问题？[提交反馈](https://github.com/returnk/codex-usage-badge/issues/new/choose)，附上复现步骤、显示缩放和主题即可。截图或日志请先检查隐私，不要上传凭据。

[开发与构建](CodexBadge.Tauri/README.md) · [MIT License](LICENSE)

<sub>第三方开源工具，非 OpenAI 官方产品。</sub>
