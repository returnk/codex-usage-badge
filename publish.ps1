$ErrorActionPreference = 'Stop'

$workspaceSdk = Join-Path (Split-Path $PSScriptRoot -Parent) '.dotnet\dotnet.exe'
$dotnet = if (Test-Path -LiteralPath $workspaceSdk) { $workspaceSdk } else { (Get-Command dotnet).Source }
$project = Join-Path $PSScriptRoot 'src\CodexBadge\CodexBadge.csproj'
$nugetConfig = Join-Path $PSScriptRoot 'NuGet.Config'

function Publish-Variant {
    param([string]$Name)
    $output = Join-Path $PSScriptRoot "artifacts\$Name"
    $arguments = @(
        'publish', $project,
        '--configuration', 'Release',
        '--runtime', 'win-x64',
        '--self-contained', 'false',
        '--configfile', $nugetConfig,
        '--output', $output,
        '-p:PublishSingleFile=true',
        '-p:DebugType=None',
        '-p:DebugSymbols=false'
    )

    & $dotnet @arguments
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    $source = Join-Path $output 'CodexBadge.exe'
    $destination = Join-Path $output "CodexBadge-$Name.exe"
    if (Test-Path -LiteralPath $destination) {
        Remove-Item -LiteralPath $destination -Force
    }
    Move-Item -LiteralPath $source -Destination $destination -Force
    Write-Host "Published: $destination"
}

Publish-Variant -Name 'win-x64-framework-dependent'

$artifacts = Join-Path $PSScriptRoot 'artifacts'
$releaseFiles = @(
    Join-Path $artifacts 'win-x64-framework-dependent\CodexBadge-win-x64-framework-dependent.exe'
)
$checksumPath = Join-Path $artifacts 'SHA256SUMS.txt'
$releaseFiles |
    ForEach-Object { Get-FileHash -Algorithm SHA256 -LiteralPath $_ } |
    ForEach-Object { "{0}  {1}" -f $_.Hash.ToLowerInvariant(), (Split-Path $_.Path -Leaf) } |
    Set-Content -LiteralPath $checksumPath -Encoding ascii
Write-Host "Checksums: $checksumPath"
