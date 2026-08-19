[CmdletBinding()]
param(
    [string] $ExecutablePath,
    [string] $OutputDirectory,
    [switch] $SkipBuild
)

$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot
$desktopDirectory = Join-Path $repository 'apps\desktop'
$tauriConfigPath = Join-Path $desktopDirectory 'src-tauri\tauri.conf.json'
$manifestTemplatePath = Join-Path $repository 'packaging\windows\msix\AppxManifest.xml.template'
$iconDirectory = Join-Path $desktopDirectory 'src-tauri\icons'

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repository 'dist\store'
}

if (-not $SkipBuild) {
    if (-not (Get-Command cargo.exe -ErrorAction SilentlyContinue)) {
        $cargoDirectory = Join-Path $env:USERPROFILE '.cargo\bin'
        $cargoPath = Join-Path $cargoDirectory 'cargo.exe'
        if (-not (Test-Path -LiteralPath $cargoPath -PathType Leaf)) {
            throw 'cargo.exe was not found. Install the stable Rust toolchain with rustup.'
        }
        $env:PATH = "$cargoDirectory;$env:PATH"
    }
    Push-Location $desktopDirectory
    try {
        & npm.cmd run tauri -- build --no-bundle
        if ($LASTEXITCODE -ne 0) {
            throw 'The Tauri release build failed.'
        }
    } finally {
        Pop-Location
    }
}

if (-not $ExecutablePath) {
    $ExecutablePath = Join-Path $repository 'target\release\quiver-desktop.exe'
}
if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
    throw "Desktop executable not found: $ExecutablePath"
}
if (-not (Test-Path -LiteralPath $manifestTemplatePath -PathType Leaf)) {
    throw "MSIX manifest template not found: $manifestTemplatePath"
}

$tauriConfig = Get-Content -Raw -LiteralPath $tauriConfigPath | ConvertFrom-Json
$versionMatch = [regex]::Match([string] $tauriConfig.version, '^(\d+)\.(\d+)\.(\d+)$')
if (-not $versionMatch.Success) {
    throw 'The desktop version must contain exactly three numeric components for MSIX packaging.'
}
$versionParts = @(
    [int] $versionMatch.Groups[1].Value
    [int] $versionMatch.Groups[2].Value
    [int] $versionMatch.Groups[3].Value
)
foreach ($part in $versionParts) {
    if ($part -lt 0 -or $part -gt 65535) {
        throw 'Each MSIX version component must be between 0 and 65535.'
    }
}
$msixVersion = "$($versionParts[0]).$($versionParts[1]).$($versionParts[2]).0"

$sdkRoot = 'C:\Program Files (x86)\Windows Kits\10\bin'
$makeAppx = Get-ChildItem -Path (Join-Path $sdkRoot '*\x64\makeappx.exe') -ErrorAction SilentlyContinue |
    Sort-Object { [Version] $_.Directory.Parent.Name } -Descending |
    Select-Object -First 1
if (-not $makeAppx) {
    throw 'MakeAppx.exe was not found. Install the Windows 10 or Windows 11 SDK.'
}

$resolvedExecutable = (Resolve-Path -LiteralPath $ExecutablePath).Path
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$resolvedOutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$outputPath = Join-Path $resolvedOutputDirectory "QuiverDL_$($msixVersion)_x64.msix"
if (Test-Path -LiteralPath $outputPath) {
    throw "Refusing to overwrite existing package: $outputPath"
}

$temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$workingDirectory = Join-Path $temporaryRoot "quiverdl-msix-$([Guid]::NewGuid().ToString('N'))"
$stageDirectory = Join-Path $workingDirectory 'stage'
$unpackDirectory = Join-Path $workingDirectory 'unpacked'

try {
    New-Item -ItemType Directory -Path (Join-Path $stageDirectory 'Assets') -Force | Out-Null
    Copy-Item -LiteralPath $resolvedExecutable -Destination (Join-Path $stageDirectory 'QuiverDL.exe')

    $assetNames = @('Square150x150Logo.png', 'Square44x44Logo.png', 'StoreLogo.png')
    foreach ($assetName in $assetNames) {
        $assetPath = Join-Path $iconDirectory $assetName
        if (-not (Test-Path -LiteralPath $assetPath -PathType Leaf)) {
            throw "Required MSIX asset not found: $assetPath"
        }
        Copy-Item -LiteralPath $assetPath -Destination (Join-Path $stageDirectory "Assets\$assetName")
    }

    $manifest = (Get-Content -Raw -LiteralPath $manifestTemplatePath).Replace('{{VERSION}}', $msixVersion)
    if ($manifest.Contains('{{')) {
        throw 'The generated MSIX manifest still contains an unresolved placeholder.'
    }
    $manifestXml = [xml] $manifest
    $identity = $manifestXml.Package.Identity
    if ($identity.Name -ne 'SHEEKOVIC.QuiverDL' -or
        $identity.Publisher -ne 'CN=BC484461-F987-4E7B-82B4-47D7995725CA' -or
        $identity.Version -ne $msixVersion) {
        throw 'The generated MSIX identity does not match the reserved Partner Center identity.'
    }
    $utf8WithoutBom = New-Object System.Text.UTF8Encoding($false)
    [IO.File]::WriteAllText((Join-Path $stageDirectory 'AppxManifest.xml'), $manifest, $utf8WithoutBom)

    $fixedTimestamp = [DateTime]::SpecifyKind([DateTime]::Parse('2000-01-01T00:00:00'), [DateTimeKind]::Utc)
    Get-ChildItem -LiteralPath $stageDirectory -File -Recurse | ForEach-Object {
        $_.LastWriteTimeUtc = $fixedTimestamp
    }

    & $makeAppx.FullName pack /h SHA256 /d $stageDirectory /p $outputPath /no
    if ($LASTEXITCODE -ne 0) {
        throw 'MakeAppx failed to create the MSIX package.'
    }

    & $makeAppx.FullName unpack /p $outputPath /d $unpackDirectory /no
    if ($LASTEXITCODE -ne 0) {
        throw 'MakeAppx failed to unpack the MSIX validation copy.'
    }

    $unpackedExecutable = Join-Path $unpackDirectory 'QuiverDL.exe'
    if (-not (Test-Path -LiteralPath $unpackedExecutable -PathType Leaf)) {
        throw 'The validated MSIX does not contain QuiverDL.exe.'
    }
    $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $resolvedExecutable).Hash
    $packageHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $unpackedExecutable).Hash
    if ($sourceHash -ne $packageHash) {
        throw 'The executable in the validated MSIX differs from the source executable.'
    }

    $sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $outputPath).Hash.ToLowerInvariant()
    Write-Output "Created unsigned Microsoft Store package: $outputPath"
    Write-Output "Identity: SHEEKOVIC.QuiverDL"
    Write-Output "Publisher: CN=BC484461-F987-4E7B-82B4-47D7995725CA"
    Write-Output "Version: $msixVersion"
    Write-Output "SHA-256: $sha256"
} catch {
    if (Test-Path -LiteralPath $outputPath) {
        Remove-Item -LiteralPath $outputPath -Force
    }
    throw
} finally {
    $fullWorkingDirectory = [IO.Path]::GetFullPath($workingDirectory)
    if ($fullWorkingDirectory.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path -Leaf $fullWorkingDirectory).StartsWith('quiverdl-msix-', [StringComparison]::Ordinal)) {
        Remove-Item -LiteralPath $fullWorkingDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}
