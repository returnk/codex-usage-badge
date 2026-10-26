# Tauri 发布入口

仓库：<https://github.com/returnk/codex-usage-badge>

CI：<https://github.com/returnk/codex-usage-badge/actions>

安装版更新入口：<https://github.com/returnk/codex-usage-badge/releases/latest/download/latest.json>

以下从 `CodexBadge.Tauri/` 执行，使用 PowerShell 7。根目录 `publish.ps1` 属于旧 WPF 实现。这里只说明命令，运行预检或打包均不授权推送和公开发布。

## 1. 确定范围，复用已有结果

- 先看 `git status --short`，识别待发布提交及用户已有改动。版本号同步到 `package.json`、`package-lock.json`（两处）、`src-tauri/tauri.conf.json`、`Cargo.toml` 和 `Cargo.lock`。
- 更新开发说明、首页下载版本和 `docs/releases/v<版本>.md`。公开说明以已实现、已验证的用户变更为准。
- 本地只验证本次改动及直接影响路径；记录对应提交、命令和结果，代码/依赖/环境未发生相关变化就复用。完整自动检查看 CI，不再在本地重复一轮。
- 发布入口本身的修改运行 `pwsh -NoProfile -File tests/release-script.test.ps1` 即可，不触发产品构建。实际交互验收与自动检查分开记录。

## 2. 预检、构建或复用已验收构建

```powershell
# 默认只读：核对版本，显示路径和本机依赖是否可用。
pwsh -NoProfile -File scripts/package-release.ps1

# 需要新构建时执行一次。依赖缺失才 npm ci；沿用现有更新签名密钥。
pwsh -NoProfile -File scripts/package-release.ps1 -Mode Package -UpdaterNotes '审核后的公开更新摘要'

# 已有同一提交的签名构建和验收证据时，跳过构建，只整理产物。
pwsh -NoProfile -File scripts/package-release.ps1 -Mode Package -SkipBuild -TargetDirectory 'D:\已有构建目录' -UpdaterNotes '审核后的公开更新摘要'
```

默认读取 `CARGO_TARGET_DIR`，否则使用 `src-tauri/target/`。本机现有缓存为 `D:\ChatGPT\CodexFloat\构建缓存\main-tauri-target`，需要复用时显式传 `-TargetDirectory`。密钥默认在 `$env:USERPROFILE/.tauri/codexbadge-updater.key`；可用 `-KeyPath` 指定原密钥。依赖在另一现有工作区时，可用 `-CliDirectory` 指向其 `node_modules/@tauri-apps/cli`，不必重复安装。

默认产物位于 `artifacts/release-v<版本>/packages/`：安装器、更新签名、便携 ZIP、`latest.json`、`SHA256SUMS.txt`，共五个文件。ZIP 只含 EXE、LICENSE、README.txt。脚本拒绝非空输出目录；已有合格产物直接上传，确需重新整理时传新的 `-OutputDirectory`，不要覆盖已验收包。

预检不证明二进制或签名有效；`-SkipBuild` 的版本检查也不证明构建来自当前提交。只有来源明确且已验收的构建才能复用，不能因为版本相同就认定有效。

## 3. 提交、上传草稿、确认发布

仅在用户已授权对应操作时执行。使用当前工作区，保护其他分支和工作区。先按文件路径暂存本次发布内容并用中文提交，避免 `git add .` 带入无关资料。

```powershell
$repo = 'returnk/codex-usage-badge'
$version = (Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
$tag = "v$version"
$packages = "artifacts/release-$tag/packages" # 自定义输出目录时同步此路径。
$notes = "docs/releases/$tag.md"

# 提交完成后创建标签；HEAD 必须是最终待发布提交。
git tag -a $tag -m "发布 $tag"
# 必须能快进到远端 main；失败先查原因，不强推。
git push --atomic origin HEAD:refs/heads/main "refs/tags/$tag"

# 每行成功后再执行下一行；任一外部命令失败就停止。
gh release create $tag --repo $repo --verify-tag --draft --title "Codex Badge $tag" --notes-file $notes
$assets = (Get-ChildItem -LiteralPath $packages -File).FullName
gh release upload $tag @assets --repo $repo
gh run list --repo $repo --commit (git rev-parse HEAD) --workflow ci.yml --limit 1
```

上传后核对五个资产名称/大小，使用 `gh api "repos/$repo/releases/tags/$tag"` 中的资产 SHA-256 digest 与本地文件哈希比较；不要再整批下载草稿和正式包各一次。如果 digest 缺失或有不一致，只下载对应资产排查。核对 portable ZIP 内容、版本和签名的已有结果仍有效时直接复用。

确认该提交 CI 成功、资产完整后，才执行公开发布：

```powershell
gh release edit $tag --repo $repo --draft=false --latest
```

发布后只确认一次公开 Release 页面和上述 `latest.json` 更新入口的版本、安装器 URL、签名与本地产物一致。已有验收结果无需再跑。必要检查成功、没有待解决问题即结束；只有新改动、失败或明确风险才追加检查。
