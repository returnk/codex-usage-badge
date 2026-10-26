# 项目入口与验证范围

- GitHub：<https://github.com/returnk/codex-usage-badge>；当前产品在 `CodexBadge.Tauri/`，根目录 `publish.ps1` 只用于旧 WPF 实现。
- Tauri 发布统一入口：[发布流程](CodexBadge.Tauri/docs/release.md)。先复用这里的命令和已有结果，不在产物目录临时重写发布脚本。
- 修改前检查 Git 状态，保留已有改动。讨论中的建议不视为修改或发布授权。
- 本地验证修改及直接影响的路径。已有结果在对应代码、依赖和环境未发生相关变化时继续有效；发生新改动、失败或明确覆盖缺口才补测或扩大范围。
- 版本、说明和发布脚本改动只检查对应内容；发布脚本使用独立临时目录演练，不重新构建产品或触发发布。必要检查通过且没有待解决问题即停止。
- 完整自动检查由 `.github/workflows/ci.yml` 执行；不因“准备发布”重复本地全套检查。自动检查、模拟和用户实际验收分别说明。
- Git 提交说明使用简体中文。公开说明只写面向用户的变更，保留未公开交互的发现体验。

## 常用命令

以下命令均从 `CodexBadge.Tauri/` 执行，按本次改动选择：

- 前端/安装脚本：`node --test tests/<相关文件>.test.cjs`。
- Rust：`cargo test --locked --manifest-path src-tauri/Cargo.toml <相关测试过滤词>`。
- Rust 格式：`cargo fmt --check --manifest-path src-tauri/Cargo.toml`，仅改 Rust 时使用。
- 发布入口演练：`pwsh -NoProfile -File tests/release-script.test.ps1`。
- 发布参数预检：`pwsh -NoProfile -File scripts/package-release.ps1`；默认不写文件、不构建、不联网。
