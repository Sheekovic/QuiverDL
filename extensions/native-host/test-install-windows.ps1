$ErrorActionPreference = "Stop"

$testId = [Guid]::NewGuid().ToString("N")
$temporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) "quiverdl-installer-test-$testId"
$registryBase = "HKCU:\Software\QuiverDLInstallerTest\$testId"

try {
  New-Item -ItemType Directory -Force -Path $temporaryDirectory | Out-Null
  $hostPath = Join-Path $temporaryDirectory "quiver&host.exe"
  Set-Content -Encoding ASCII -LiteralPath $hostPath -Value "fixture"
  $env:LOCALAPPDATA = Join-Path $temporaryDirectory "local"
  $env:APPDATA = Join-Path $temporaryDirectory "roaming"
  $extensionId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

  & (Join-Path $PSScriptRoot "install-windows.ps1") `
    -HostPath $hostPath `
    -ChromiumExtensionId $extensionId `
    -RegistryBase $registryBase

  $installDirectory = Join-Path $env:LOCALAPPDATA "QuiverDL\NativeMessaging"
  $installedHostPath = Join-Path $installDirectory "quiver-native-host.exe"
  if (-not (Test-Path -LiteralPath $installedHostPath -PathType Leaf)) {
    throw "The native messaging host was not copied into its durable install directory"
  }
  $expectations = @(
    @("Google\Chrome\NativeMessagingHosts\app.quiverdl.native", "app.quiverdl.native.chromium.json"),
    @("Microsoft\Edge\NativeMessagingHosts\app.quiverdl.native", "app.quiverdl.native.chromium.json"),
    @("Mozilla\NativeMessagingHosts\app.quiverdl.native", "app.quiverdl.native.firefox.json")
  )
  foreach ($expectation in $expectations) {
    $key = Join-Path $registryBase $expectation[0]
    $manifestPath = Join-Path $installDirectory $expectation[1]
    if ((Get-Item -LiteralPath $key).GetValue("") -ne $manifestPath) {
      throw "Native messaging registry value is incorrect: $key"
    }
    $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
    if ($manifest.path -ne (Resolve-Path -LiteralPath $installedHostPath).Path) {
      throw "Native messaging host path is incorrect: $manifestPath"
    }
  }
} finally {
  if ($registryBase -like "HKCU:\Software\QuiverDLInstallerTest\*") {
    Remove-Item -LiteralPath $registryBase -Recurse -Force -ErrorAction SilentlyContinue
  }
  $resolvedTemporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
  $resolvedTemporaryDirectory = [IO.Path]::GetFullPath($temporaryDirectory)
  if ($resolvedTemporaryDirectory.StartsWith($resolvedTemporaryRoot, [StringComparison]::OrdinalIgnoreCase) -and
      (Split-Path -Leaf $resolvedTemporaryDirectory) -like "quiverdl-installer-test-*") {
    Remove-Item -LiteralPath $resolvedTemporaryDirectory -Recurse -Force -ErrorAction SilentlyContinue
  }
}
