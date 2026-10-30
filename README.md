<p align="center">
  <img src="docs/images/brand.svg" width="72" alt="余量 · Codex Badge">
</p>

<h1 align="center">余量 · Codex Badge</h1>

<p align="center"><strong>Codex 还剩多少，一眼就知道。</strong></p>
<p align="center">Windows 上的 Codex 额度胶囊 · Codex usage monitor for Windows</p>

<p align="center">
  <a href="https://github.com/returnk/codex-usage-badge/releases/tag/v0.3.5"><img src="https://img.shields.io/badge/v0.3.5-release-2879E7" alt="v0.3.5 正式发布"></a>
  <img src="https://img.shields.io/badge/Windows-10%20%2F%2011-1B2738" alt="Windows 10 / 11 x64">
  <img src="https://img.shields.io/badge/Tauri-2-24C8DB" alt="Tauri 2">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-34C759" alt="MIT license"></a>
</p>

<p align="center">
  <a href="#下载">下载</a> · <a href="#开始使用">开始使用</a> · <a href="#日常操作">日常操作</a> · <a href="https://github.com/returnk/codex-usage-badge/issues">反馈</a> · <a href="docs/README.en.md">English</a>
</p>

把剩余额度留在眼前，把注意力留给工作。**余量**贴合 Codex 侧栏，根据接口返回优先显示 5 小时剩余百分比，只有周窗口时显示本周剩余；悬停展开本周剩余、重置时间和重置机会，不必切换页面。

<p align="center">
  <img src="docs/images/tauri-preview.svg" width="1100" alt="余量 Tauri 界面示意：毛玻璃、跟随系统浅色、跟随系统深色；显示 5 小时和每周剩余额度、重置时间与重置机会">
  <br>
  <sub>当前界面风格示意 · 示例数据 · 两个主题选项：毛玻璃 / 跟随系统</sub>
</p>

## 下载

本版新增需账号明确授权的“临期用卡”，改进圆边、悬停提示和授权界面，并修复更新控件避让。[查看版本说明](CodexBadge.Tauri/docs/releases/v0.3.5.md)。

**v0.3.5** · Windows 10 / 11 x64 · 正式发布版本

| 安装版 · 推荐 | 便携版 |
| --- | --- |
| [下载安装器](https://github.com/returnk/codex-usage-badge/releases/download/v0.3.5/Codex.Badge_0.3.5_x64-setup.exe) | [下载 ZIP](https://github.com/returnk/codex-usage-badge/releases/download/v0.3.5/CodexBadge-v0.3.5-windows-x64-portable.zip) |
| 按提示安装后运行 | 解压后运行 `codex-badge-tauri.exe` |

[更新说明](https://github.com/returnk/codex-usage-badge/releases/tag/v0.3.5) · [SHA-256 校验](https://github.com/returnk/codex-usage-badge/releases/download/v0.3.5/SHA256SUMS.txt)

无需 .NET。需要 **WebView2 Runtime**、已登录的 Codex Windows 桌面客户端和可用的 Codex CLI；安装器可引导下载 WebView2。程序尚未签名，首次运行可能出现 Windows 安全提示，请确认来自本仓库。

## 开始使用

1. 安装或解压，运行余量。
2. 打开已登录的 Codex，胶囊自动出现。
3. 右键胶囊或托盘图标开启**置顶模式**；在**设置 ›**中选择开机启动、临期用卡或检查更新。

余量在后台运行时，会随 Codex 打开而显示、退出而收起；从托盘选择“退出”后，需要手动重新启动余量。

## 日常操作

| 操作 | 效果 |
| --- | --- |
| 悬停胶囊 | 展开 5 小时、本周剩余和重置时间 |
| 点击“查看” | 查看重置机会的到期时间（有可用详情时显示） |
| 滚动鼠标滚轮 | 切换**毛玻璃 / 跟随系统**；系统主题自动适配明暗 |
| 拖动胶囊 | 调整位置，普通模式和置顶模式分别记忆 |
| 右键胶囊或托盘 | 置顶模式、设置 ›、退出；设置内包含开机启动、临期用卡和检查更新 |
| 双击胶囊 | 恢复当前模式的默认位置 |

普通模式随 Codex 最小化隐藏；置顶模式可在 Codex 最小化时继续显示。Codex 退出后，两种模式都会收起。

## 常见问题

**没有看到胶囊？** 确认余量正在运行、ChatGPT/Codex 主窗口已经打开。普通模式下，客户端最小化时不会显示。v0.3.3 已适配新版 ChatGPT 的布局变化；胶囊位置不合适时可双击恢复默认位置。

**显示 `--%`？** 确认 Codex 已登录，且 Codex CLI 可用。找不到 CLI 时，可将环境变量 `CODEX_BADGE_CLI` 设置为 `codex.exe` 的完整路径。

**更新或移动便携版？** 先关闭开机启动，再退出正在运行的版本；运行新版本后按需重新开启。开机启动会记住程序所在路径。从 v0.2.x 升级需重新设置主题与位置。

临期用卡默认关闭。明确授权当前账号后，电脑和 CodexBadge 运行期间，会在免费卡到期前 30 分钟内按官方条件自动尝试，无需打开 Codex 窗口。关机、休眠或退出后不执行；不保证兑换成功。自动检查和用户日常测试不等于多机、混合 DPI 或完整安装升级验收。[查看发布说明](CodexBadge.Tauri/docs/releases/v0.3.5.md)

## 隐私与反馈

通过本机 `codex app-server` 读取额度；仅在明确授权临期用卡后，按条件提交官方兑换请求。不读取 `auth.json`、不发送模型请求，不上传登录凭据。设置和诊断日志保存在本机 `%LOCALAPPDATA%\CodexBadge`。

发现问题？[提交反馈](https://github.com/returnk/codex-usage-badge/issues/new/choose)，附上复现步骤、显示缩放和主题即可。截图或日志请先检查隐私，不要上传凭据。

[开发与构建](CodexBadge.Tauri/README.md) · [MIT License](LICENSE)

<sub>第三方开源工具，非 OpenAI 官方产品。</sub>

## v0.3.5 更新

新增默认关闭、按账号持续授权的临期用卡；授权界面与现有主题统一，读取账号及保存时提供反馈。修复圆形胶囊的圆边裁切，悬停提示按文字收紧宽度，并改进额度恢复识别和更新控件避让。

自动用卡成功后，已有额度不会与重置后的额度叠加，重置时间可能改变，成功无法撤销。请勿在多台设备或工具同时为同一账号开启。关闭阻止新请求，但已发送请求不能保证撤销；结果不明确时暂停并保留记录。

安装版支持主动检查和一键更新；便携版通过发布页下载 ZIP 手动替换。不会自动检查、下载或安装更新。更新签名不等于 Windows Authenticode 代码签名。

[English overview](docs/README.en.md) · 如果余量对你有帮助，欢迎给项目一个 Star。
