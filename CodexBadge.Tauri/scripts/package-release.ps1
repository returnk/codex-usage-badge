#requires -Version 7.0
<#
.SYNOPSIS
Check release inputs, or build/package signed Windows x64 assets. Never uploads.
.EXAMPLE
pwsh -File scripts/package-release.ps1
.EXAMPLE
pwsh -File scripts/package-release.ps1 -Mode Package -UpdaterNotes '本次更新摘要。'
.EXAMPLE
pwsh -File scripts/package-release.ps1 -Mode Package -SkipBuild -TargetDirectory 'D:\build-cache' -UpdaterNotes '本次更新摘要。'
#>
[CmdletBinding()]
param(
    [ValidateSet('Check', 'Package')][string]$Mode = 'Check',
    [switch]$SkipBuild,
    [string]$UpdaterNotes,
    [string]$TargetDirectory = $env:CARGO_TARGET_DIR,
    [string]$OutputDirectory,
    [string]$KeyPath = "$env:USERPROFILE/.tauri/codexbadge-updater.key",
    [string]$CliDirectory
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$repository = 'returnk/codex-usage-badge'
$config = Get-Content -LiteralPath "$projectRoot/src-tauri/tauri.conf.json" -Raw | ConvertFrom-Json -AsHashtable
$package = Get-Content -LiteralPath "$projectRoot/package.json" -Raw | ConvertFrom-Json -AsHashtable
$lock = Get-Content -LiteralPath "$projectRoot/package-lock.json" -Raw | ConvertFrom-Json -AsHashtable
$version = $config.version
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'Release version must be x.y.z.' }
$cargo = Get-Content -LiteralPath "$projectRoot/src-tauri/Cargo.toml" -Raw
$cargoLock = Get-Content -LiteralPath "$projectRoot/src-tauri/Cargo.lock" -Raw
$cargoVersion = [regex]::Match($cargo, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
$lockedVersion = [regex]::Match($cargoLock, '(?m)^name = "codex-badge-tauri"\r?\nversion = "([^"]+)"').Groups[1].Value
if (@($package.version, $lock.version, $lock.packages[''].version, $cargoVersion, $lockedVersion).Where({ $_ -ne $version }).Count) {
    throw 'Version mismatch: tauri.conf.json, package.json, package-lock.json, Cargo.toml and Cargo.lock must agree.'
}
if (!$TargetDirectory) { $TargetDirectory = "$projectRoot/src-tauri/target" }
if (!$OutputDirectory) { $OutputDirectory = "$projectRoot/artifacts/release-v$version/packages" }
if (!$CliDirectory) { $CliDirectory = "$projectRoot/node_modules/@tauri-apps/cli" }
# Resolve relative paths against the app, regardless of the caller's directory.
$TargetDirectory = [IO.Path]::GetFullPath($TargetDirectory, $projectRoot)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory, $projectRoot)
$CliDirectory = [IO.Path]::GetFullPath($CliDirectory, $projectRoot)
$KeyPath = [IO.Path]::GetFullPath($KeyPath, $projectRoot)
if ($Mode -eq 'Check') {
    [pscustomobject]@{
        Repository = "https://github.com/$repository"; Version = $version; Tag = "v$version"
        TargetDirectory = $TargetDirectory; OutputDirectory = $OutputDirectory
        CliAvailable = (Test-Path -LiteralPath "$CliDirectory/package.json")
        SigningKeyAvailable = (Test-Path -LiteralPath $KeyPath)
    }
    return
}
if ([string]::IsNullOrWhiteSpace($UpdaterNotes)) { throw 'Package requires -UpdaterNotes with the reviewed public update summary.' }
if ((Test-Path -LiteralPath $OutputDirectory) -and (Get-ChildItem -LiteralPath $OutputDirectory -Force | Select-Object -First 1)) {
    throw 'Output directory must be empty; reuse existing packages directly or select a new directory.'
}
if (!$SkipBuild) {
    if (!(Test-Path -LiteralPath "$CliDirectory/package.json")) { throw 'Tauri CLI is missing. Run npm ci once, or supply -CliDirectory for an existing installation.' }
    if (!(Test-Path -LiteralPath $KeyPath)) { throw 'Signing key is missing. Do not generate a replacement key for existing users.' }
    $publicKey = (Get-Content -LiteralPath ($KeyPath + '.pub') | Select-Object -Last 1).Trim()
    if ($publicKey -ne $config.plugins.updater.pubkey) { throw 'Signing key does not match the configured public key.' }
    $previousKey = $env:TAURI_SIGNING_PRIVATE_KEY
    $previousTarget = $env:CARGO_TARGET_DIR
    Push-Location -LiteralPath $projectRoot
    try {
        $env:CARGO_TARGET_DIR = $TargetDirectory
        $env:TAURI_SIGNING_PRIVATE_KEY = $KeyPath
        # Keep an explicitly empty password inside Node; never prompt or print key data.
        node -e 'if(!process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD)process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD="";require(process.argv[1]).run(["build","--bundles","nsis","--","--locked"]).catch(e=>{console.error(e);process.exit(1)});' $CliDirectory
        if ($LASTEXITCODE -ne 0) { throw 'Signed release build failed.' }
    } finally {
        $env:TAURI_SIGNING_PRIVATE_KEY = $previousKey
        $env:CARGO_TARGET_DIR = $previousTarget
        Pop-Location
    }
}
$installer = "$TargetDirectory/release/bundle/nsis/$($config.productName)_${version}_x64-setup.exe"
$executable = "$TargetDirectory/release/codex-badge-tauri.exe"
foreach ($path in @($installer, ($installer + '.sig'), $executable)) {
    if (!(Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing build artifact: $path" }
}
$fileVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo($executable)
if ("$($fileVersion.FileMajorPart).$($fileVersion.FileMinorPart).$($fileVersion.FileBuildPart)" -ne $version) {
    throw 'Executable version does not match release metadata.'
}
$signature = (Get-Content -LiteralPath ($installer + '.sig') -Raw).Trim()
if (!$signature) { throw 'Updater signature is empty.' }
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$installerName = "Codex.Badge_${version}_x64-setup.exe"
Copy-Item -LiteralPath $installer -Destination "$OutputDirectory/$installerName"
Copy-Item -LiteralPath ($installer + '.sig') -Destination "$OutputDirectory/$installerName.sig"
$portable = Join-Path ([IO.Path]::GetTempPath()) ("codexbadge-portable-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $portable | Out-Null
try {
    Copy-Item -LiteralPath $executable -Destination $portable
    Copy-Item -LiteralPath "$projectRoot/../LICENSE" -Destination $portable
    @"
Codex Badge v$version / Windows 10-11 x64

Extract all files, then run codex-badge-tauri.exe.
Requires WebView2 Runtime, a signed-in Codex Windows desktop app, and Codex CLI.
No .NET runtime is needed. Exit the old copy through its tray menu first.
Disable launch at login before moving this folder, then enable it again.
Portable copies update by downloading and extracting the next ZIP.

Source and releases: https://github.com/$repository
Local settings and diagnostics: %LOCALAPPDATA%\CodexBadge
Independent community project; not an official OpenAI product.
Updater signatures are separate from Windows Authenticode signing.
"@ | Set-Content -LiteralPath "$portable/README.txt" -Encoding utf8NoBOM
    Compress-Archive -LiteralPath "$portable/codex-badge-tauri.exe", "$portable/LICENSE", "$portable/README.txt" -DestinationPath "$OutputDirectory/CodexBadge-v$version-windows-x64-portable.zip"
} finally {
    # Only remove the fresh, uniquely named staging directory created above.
    $resolvedPortable = [IO.Path]::GetFullPath($portable)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if (!$resolvedPortable.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path $resolvedPortable -Leaf) -notlike 'codexbadge-portable-*') { throw 'Unexpected staging path.' }
    Remove-Item -LiteralPath $resolvedPortable -Recurse -Force
}
$manifest = @{
    version = $version; notes = $UpdaterNotes
    pub_date = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = @{ 'windows-x86_64' = @{
        signature = $signature
        url = "https://github.com/$repository/releases/download/v$version/$installerName"
    } }
}
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath "$OutputDirectory/latest.json" -Encoding utf8NoBOM
Get-ChildItem -LiteralPath $OutputDirectory -File | Sort-Object Name | ForEach-Object {
    '{0}  {1}' -f (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $_.Name
} | Set-Content -LiteralPath "$OutputDirectory/SHA256SUMS.txt" -Encoding ascii
Get-ChildItem -LiteralPath $OutputDirectory -File | Select-Object Name, Length
