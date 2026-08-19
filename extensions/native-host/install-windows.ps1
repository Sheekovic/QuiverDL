param(
  [Parameter(Mandatory = $true)][string]$HostPath,
  [Parameter(Mandatory = $true)][string]$ChromiumExtensionId,
  [string]$RegistryBase = "HKCU:\Software"
)

$resolvedHost = (Resolve-Path -LiteralPath $HostPath).Path
$installDirectory = Join-Path $env:LOCALAPPDATA "QuiverDL\NativeMessaging"
New-Item -ItemType Directory -Force -Path $installDirectory | Out-Null

$chromiumManifestPath = Join-Path $installDirectory "app.quiverdl.native.chromium.json"
$chromiumManifest = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "chromium-host.json") | ConvertFrom-Json
$chromiumManifest.path = $resolvedHost
$chromiumManifest.allowed_origins = @("chrome-extension://$ChromiumExtensionId/")
$chromiumManifest | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 -LiteralPath $chromiumManifestPath

$chromeKey = Join-Path $RegistryBase "Google\Chrome\NativeMessagingHosts\app.quiverdl.native"
$edgeKey = Join-Path $RegistryBase "Microsoft\Edge\NativeMessagingHosts\app.quiverdl.native"
New-Item -Force -Path $chromeKey | Out-Null
Set-Item -Path $chromeKey -Value $chromiumManifestPath
New-Item -Force -Path $edgeKey | Out-Null
Set-Item -Path $edgeKey -Value $chromiumManifestPath

$firefoxManifestPath = Join-Path $installDirectory "app.quiverdl.native.firefox.json"
$firefoxManifest = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "firefox-host.json") | ConvertFrom-Json
$firefoxManifest.path = $resolvedHost
$firefoxManifest | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 -LiteralPath $firefoxManifestPath
$firefoxKey = Join-Path $RegistryBase "Mozilla\NativeMessagingHosts\app.quiverdl.native"
New-Item -Force -Path $firefoxKey | Out-Null
Set-Item -Path $firefoxKey -Value $firefoxManifestPath

Write-Host "Installed QuiverDL native messaging manifests for Chrome, Edge, and Firefox."
