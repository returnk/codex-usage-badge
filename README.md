# Codex Usage Badge for Windows

**Codex Badge** 是一个适用于 Windows 10/11 的极简 Codex 额度胶囊。

> 目前仅提供 Windows 10/11 x64 版本，暂不支持 macOS 与 Linux。

Codex 桌面客户端的额度信息不够直观，查看剩余额度和重置时间容易打断当前操作。Codex Badge 会跟随 Codex 窗口，在原额度位置直接展示 5 小时剩余百分比；鼠标悬停即可查看重置时间、本周剩余额度和重置机会。

> 非官方开源工具，与 OpenAI 无隶属关系，也未获得 OpenAI 背书。

## 功能

- 自动跟随 Codex 窗口；Codex 最小化或关闭时自动隐藏
- 显示 5 小时额度，悬停查看完整详情
- 5 小时进度条按额度显示绿色、黄色或橙红色
- 鼠标滚轮切换蓝色、浅色和深色主题
- 拖动胶囊微调位置，双击恢复默认位置且保留当前主题
- 当前用户开机启动，无需管理员权限
- 单文件、便携运行

## 下载

GitHub Release 提供两个 Windows x64 版本：

| 版本 | 大小 | 运行要求 |
| --- | ---: | --- |
| `CodexBadge-win-x64-framework-dependent.exe` | 约 0.30 MiB | 需要 [.NET 10 Desktop Runtime x64](https://dotnet.microsoft.com/download/dotnet/10.0) |
| `CodexBadge-win-x64-self-contained.exe` | 约 165 MiB | 无需预装 .NET |

多数用户可优先选择体积较小的 Framework-dependent 版本。如果电脑没有 .NET 10 Desktop Runtime，可使用 Self-contained 版本。

## 使用

1. 下载并运行 `CodexBadge.exe`。
2. 打开 Codex 桌面客户端，胶囊会自动出现在底部额度位置。
3. 悬停查看额度详情；滚轮切换主题；拖动可微调位置。
4. 双击胶囊恢复默认位置。托盘菜单可控制开机启动、重新定位或退出。

设置保存在 `%LOCALAPPDATA%\CodexBadge\settings.json`。

Codex Badge 通过本机 `codex app-server` 读取额度，不会读取或上传你的登录凭据。如果无法自动找到 Codex CLI，可将环境变量 `CODEX_BADGE_CLI` 设置为 `codex.exe` 的完整路径。

## 从源码构建

需要 .NET 10 SDK：

```powershell
dotnet run --project .\tests\CodexBadge.Core.Tests\CodexBadge.Core.Tests.csproj
.\publish.ps1
```

构建结果保存在 `artifacts/`，该目录不会提交到源码仓库。

## 支持范围

- Windows 10/11 x64
- Codex Windows 桌面客户端
- 暂无 macOS 或 Linux 版本
- 未签名便携程序，Windows 首次运行时可能显示未知发布者提示

## License

[MIT](LICENSE)
