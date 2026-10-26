#requires -Version 7.0
# Isolated packaging smoke check. Uses pwsh.exe as inert fixture data; no product build or network.
$ErrorActionPreference = 'Stop'
$source = Join-Path (Split-Path $PSScriptRoot -Parent) 'scripts/package-release.ps1'
$fixture = Join-Path ([IO.Path]::GetTempPath()) ("codexbadge-release-test-" + [guid]::NewGuid())
function Assert-Release($Condition, [string]$Message) {
    if (!$Condition) { throw $Message }
}
function Expect-Failure([scriptblock]$Action, [string]$Pattern) {
    try { & $Action | Out-Null } catch {
        Assert-Release ($_.Exception.Message -match $Pattern) "Unexpected error: $_"
        return
    }
    throw "Expected failure: $Pattern"
}
try {
    $app = "$fixture/app"
    New-Item -ItemType Directory -Path "$app/scripts", "$app/src-tauri" -Force | Out-Null
    Copy-Item -LiteralPath $source -Destination "$app/scripts/package-release.ps1"
    $script = "$app/scripts/package-release.ps1"
    $binary = Join-Path $PSHOME 'pwsh.exe'
    $fileVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo($binary)
    $version = "$($fileVersion.FileMajorPart).$($fileVersion.FileMinorPart).$($fileVersion.FileBuildPart)"
    @{ productName = 'Codex Badge'; version = $version; plugins = @{ updater = @{ pubkey = 'fixture-key' } } } | ConvertTo-Json -Depth 5 | Set-Content "$app/src-tauri/tauri.conf.json"
    @{ version = $version } | ConvertTo-Json | Set-Content "$app/package.json"
    @{ version = $version; packages = @{ '' = @{ version = $version } } } | ConvertTo-Json -Depth 5 | Set-Content "$app/package-lock.json"
    "[package]`nname = `"codex-badge-tauri`"`nversion = `"$version`"" | Set-Content "$app/src-tauri/Cargo.toml"
    "[[package]]`nname = `"codex-badge-tauri`"`nversion = `"$version`"" | Set-Content "$app/src-tauri/Cargo.lock"
    'Fixture license' | Set-Content "$fixture/LICENSE"
    $output = "$app/output"
    $target = "$app/build"
    $arguments = @{ TargetDirectory = 'build'; OutputDirectory = 'output' }
    $check = & $script @arguments
    Assert-Release ($check.Version -eq $version -and $check.TargetDirectory -eq [IO.Path]::GetFullPath($target)) 'Read-only check returned incorrect version/path.'
    Assert-Release (!(Test-Path -LiteralPath $output)) 'Check unexpectedly created output.'

    '{"version":"0.0.0"}' | Set-Content "$app/package.json"
    Expect-Failure { & $script @arguments } 'Version mismatch'
    @{ version = $version } | ConvertTo-Json | Set-Content "$app/package.json"
    Expect-Failure { & $script @arguments -Mode Package -SkipBuild } 'requires -UpdaterNotes'
    Expect-Failure { & $script @arguments -Mode Package -SkipBuild -UpdaterNotes 'Fixture notes' } 'Missing build artifact'
    Assert-Release (!(Test-Path -LiteralPath $output)) 'Invalid inputs created output.'

    New-Item -ItemType Directory -Path "$target/release/bundle/nsis" -Force | Out-Null
    Copy-Item -LiteralPath $binary -Destination "$target/release/codex-badge-tauri.exe"
    'Inert installer fixture' | Set-Content "$target/release/bundle/nsis/Codex Badge_${version}_x64-setup.exe"
    'fixture-signature' | Set-Content "$target/release/bundle/nsis/Codex Badge_${version}_x64-setup.exe.sig"
    & $script @arguments -Mode Package -SkipBuild -UpdaterNotes 'Fixture notes' | Out-Null
    $names = @(Get-ChildItem -LiteralPath $output -File).Name
    $expected = @("Codex.Badge_${version}_x64-setup.exe", "Codex.Badge_${version}_x64-setup.exe.sig", "CodexBadge-v$version-windows-x64-portable.zip", 'latest.json', 'SHA256SUMS.txt')
    Assert-Release (!(Compare-Object $expected $names)) 'Unexpected package contents.'
    $manifest = Get-Content "$output/latest.json" -Raw | ConvertFrom-Json
    Assert-Release ($manifest.version -eq $version -and $manifest.notes -eq 'Fixture notes' -and $manifest.platforms.'windows-x86_64'.signature -eq 'fixture-signature') 'Incorrect updater metadata.'
    Assert-Release ($manifest.platforms.'windows-x86_64'.url -eq "https://github.com/returnk/codex-usage-badge/releases/download/v$version/Codex.Badge_${version}_x64-setup.exe") 'Incorrect updater URL.'
    $checksums = @(Get-Content "$output/SHA256SUMS.txt")
    Assert-Release ($checksums.Count -eq 4) 'Expected four checksums.'
    foreach ($line in $checksums) {
        $hash, $name = $line -split '  ', 2
        Assert-Release ((Get-FileHash -LiteralPath "$output/$name" -Algorithm SHA256).Hash -eq $hash) "Checksum mismatch: $name"
    }
    $archive = [IO.Compression.ZipFile]::OpenRead("$output/CodexBadge-v$version-windows-x64-portable.zip")
    try {
        Assert-Release (!(Compare-Object @('codex-badge-tauri.exe', 'LICENSE', 'README.txt') @($archive.Entries.FullName))) 'Unexpected portable ZIP entries.'
    } finally { $archive.Dispose() }
    Expect-Failure { & $script @arguments -Mode Package -SkipBuild -UpdaterNotes 'Fixture notes' } 'must be empty'
    Write-Output 'PASS: read-only preflight, version/input rejection, isolated packaging, updater metadata, ZIP entries, checksums and overwrite protection.'
} finally {
    $resolvedFixture = [IO.Path]::GetFullPath($fixture)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if (!$resolvedFixture.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path $resolvedFixture -Leaf) -notlike 'codexbadge-release-test-*') { throw 'Unexpected fixture path.' }
    if (Test-Path -LiteralPath $resolvedFixture) { Remove-Item -LiteralPath $resolvedFixture -Recurse -Force }
}
