<p align="center">
  <img src="src/CodexBadge/Assets/CodexBadge.png" width="88" alt="Codex Badge icon">
</p>

<h1 align="center">Codex Usage Badge for Windows</h1>

<p align="center">在 Codex 窗口中直接查看 5 小时和每周剩余额度。</p>

<p align="center">
  <a href="https://github.com/returnk/codex-usage-badge/releases/latest"><img src="https://img.shields.io/github/v/release/returnk/codex-usage-badge?label=release" alt="Latest release"></a>
  <a href="https://github.com/returnk/codex-usage-badge/actions/workflows/ci.yml"><img src="https://github.com/returnk/codex-usage-badge/actions/workflows/ci.yml/badge.svg" alt="Build status"></a>
  <img src="https://img.shields.io/badge/Windows-10%20%7C%2011-3A83F7" alt="Windows 10 and 11">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/returnk/codex-usage-badge" alt="MIT license"></a>
</p>

Codex 桌面客户端的额度信息不够直观，查看剩余额度和重置时间容易打断当前操作。**Codex Badge** 会跟随 Codex 窗口，在原额度位置直接展示 5 小时剩余百分比；鼠标悬停即可查看重置时间、本周剩余额度和重置机会。

> 目前仅提供 Windows 10/11 x64 版本，暂不支持 macOS 与 Linux。本项目是非官方开源工具，与 OpenAI 无隶属关系，也未获得 OpenAI 背书。

<p align="center">
  <img src="docs/images/overview.png" width="900" alt="Codex Badge 胶囊与悬停额度详情卡">
  <br>
  <sub>界面预览 · 示例数据</sub>
</p>

## 下载

### Tauri 重构测试版：余量 · Codex Badge

[**v0.3.0-beta.1 测试版下载与已知问题**](https://github.com/returnk/codex-usage-badge/releases/tag/v0.3.0-beta.1) · Windows 10/11 x64 · 提供安装器与便携 ZIP，需要 Microsoft Edge WebView2 Runtime，不需要 .NET。

新实现位于 [CodexBadge.Tauri](CodexBadge.Tauri/README.md)。只有两个可选主题：**毛玻璃 / 跟随系统明暗**。已实现随 Codex 启停、独立置顶位置、两种模式右键菜单和重置机会详情；真实退出/重开资源释放、快速悬停花屏和安装升级仍需更多实测。本版不是稳定版，也不宣称原生 Windows 亚克力效果或低于旧版的运行内存。

旧版与新版不能同时运行；请先关闭旧版开机启动，再从旧版托盘退出，然后运行新版。两者设置文件独立，本版不自动导入旧 WPF 设置。新版监听程序必须运行，才能在 Codex 打开时自动显示；从托盘退出后不会自动跟随。

### 旧 WPF 稳定版 v0.2.0

[**下载 CodexBadge-win-x64-framework-dependent.exe**](https://github.com/returnk/codex-usage-badge/releases/latest/download/CodexBadge-win-x64-framework-dependent.exe) · 约 0.30 MiB · 需要 [.NET 10 Desktop Runtime x64](https://dotnet.microsoft.com/download/dotnet/10.0)

[查看全部版本与 SHA-256 校验文件](https://github.com/returnk/codex-usage-badge/releases/latest)

## 30 秒开始使用

以下使用说明、功能与截图对应旧 WPF 稳定版；Tauri 测试版请阅读上方链接。

1. 安装 [.NET 10 Desktop Runtime x64](https://dotnet.microsoft.com/download/dotnet/10.0)。
2. 下载并运行 `CodexBadge-win-x64-framework-dependent.exe`。
3. 打开已登录的 Codex Windows 桌面客户端；胶囊会自动出现在底部，软件通过系统托盘管理。

这是未签名的便携程序，Windows 首次运行时可能显示未知发布者提示。你可以在 Release 页面下载 `SHA256SUMS.txt` 核对文件完整性。

## 功能

- 自动跟随 Codex 窗口、侧栏宽度和常见 DPI；Codex 最小化或关闭时自动隐藏
- 显示 5 小时额度，悬停查看完整详情
- 5 小时进度条按额度显示绿色、黄色或橙红色
- 鼠标滚轮切换蓝色、浅色、深色和隐私模式；隐私模式仅在悬停时显示百分比
- 拖动胶囊微调位置，双击恢复默认位置且保留当前主题
- 当前用户开机启动，无需管理员权限
- 单文件、便携运行

<p align="center">
  <img src="docs/images/themes.png" width="900" alt="Codex Badge 蓝色、浅色和深色主题">
  <br>
  <sub>Codex Blue · Frost Light · Graphite Dark</sub>
</p>

设置保存在 `%LOCALAPPDATA%\CodexBadge\settings.json`。托盘菜单可控制开机启动、重新定位或退出。

## 常见问题

**胶囊没有显示**

确认 Codex 主窗口已经打开且没有最小化。仍未显示时，可从托盘菜单选择“重新定位”。

**胶囊显示 `--%` 或暂时无法读取额度**

确认 Codex 已登录，且本机 Codex CLI 可以正常运行。如果无法自动找到 CLI，可将环境变量 `CODEX_BADGE_CLI` 设置为 `codex.exe` 的完整路径。

**程序无法启动**

安装 [.NET 10 Desktop Runtime x64](https://dotnet.microsoft.com/download/dotnet/10.0)，注意需要 Desktop Runtime x64，而不是仅安装普通 .NET Runtime。

## 隐私

Codex Badge 通过本机 `codex app-server` 读取额度。它不读取 `auth.json`，不调用内部 `wham` 接口，也不会自行上传你的登录凭据。所有设置只保存在本机 `%LOCALAPPDATA%\CodexBadge`。

## 从源码构建

需要 .NET 10 SDK：

```powershell
dotnet run --project .\tests\CodexBadge.Core.Tests\CodexBadge.Core.Tests.csproj
.\publish.ps1
```

构建结果和 `SHA256SUMS.txt` 保存在 `artifacts/`，该目录不会提交到源码仓库。

## License

[MIT](LICENSE)
