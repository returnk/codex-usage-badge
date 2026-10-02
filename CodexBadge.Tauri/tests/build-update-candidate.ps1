param(
    [string]$KeyPath = "$env:USERPROFILE\.tauri\codexbadge-updater.key",
    [string]$OutputDirectory = 'artifacts/candidate-v0.3.2-one-click-2026-10-02'
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
Set-Location -LiteralPath $project
$config = Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
if (!(Test-Path -LiteralPath $KeyPath)) { throw 'Updater signing key is missing; never generate a replacement key for existing users.' }
$publicKey = (Get-Content -LiteralPath ($KeyPath + '.pub') | Select-Object -Last 1).Trim()
if ($publicKey -ne $config.plugins.updater.pubkey) { throw 'Signing public key does not match application configuration.' }
$previousKey = $env:TAURI_SIGNING_PRIVATE_KEY
$previousPassword = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
try {
    $env:TAURI_SIGNING_PRIVATE_KEY = $KeyPath
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
    if (!$env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = 'D:\ChatGPT\CodexFloat\构建缓存\main-tauri-target' }
    # Windows PowerShell 5.1 removes an empty environment variable. Set the
    # empty signing password in Node so the CLI never opens an input prompt.
    node -e "process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD=''; require('@tauri-apps/cli').run(['build','--bundles','nsis','--','--locked']).catch(error=>{console.error(error);process.exit(1);});"
    if ($LASTEXITCODE -ne 0) { throw 'Candidate build failed.' }
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    $installer = Get-ChildItem -LiteralPath "$env:CARGO_TARGET_DIR\release\bundle\nsis" -Filter "*_$($config.version)_x64-setup.exe" | Select-Object -First 1
    if (!$installer -or !(Test-Path -LiteralPath ($installer.FullName + '.sig'))) { throw 'Signed installer was not generated.' }
    Copy-Item -LiteralPath $installer.FullName, ($installer.FullName + '.sig'), "$env:CARGO_TARGET_DIR\release\codex-badge-tauri.exe" -Destination $OutputDirectory
    $name = [Uri]::EscapeDataString($installer.Name)
    $manifest = @{ version=$config.version; notes='更新界面、主题与手动一键更新。'; pub_date=(Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'); platforms=@{'windows-x86_64'=@{signature=(Get-Content -LiteralPath ($installer.FullName+'.sig') -Raw).Trim();url="https://github.com/returnk/codex-usage-badge/releases/download/v$($config.version)/$name"}} }
    [IO.File]::WriteAllText((Join-Path (Resolve-Path $OutputDirectory) 'latest.json'), ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    Get-ChildItem -LiteralPath $OutputDirectory -File | Where-Object Name -ne SHA256SUMS.txt | ForEach-Object { '{0}  {1}' -f (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $_.Name } | Set-Content -LiteralPath (Join-Path $OutputDirectory 'SHA256SUMS.txt')
    Write-Output "Candidate only; not installed or published: $OutputDirectory"
} finally {
    $env:TAURI_SIGNING_PRIVATE_KEY = $previousKey
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $previousPassword
}
