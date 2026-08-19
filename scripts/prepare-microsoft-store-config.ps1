[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[A-Fa-f0-9]{40}$')]
    [string] $CertificateThumbprint
)

$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot
$tauriDirectory = Join-Path $repository 'apps\desktop\src-tauri'
$source = Join-Path $tauriDirectory 'tauri.microsoftstore.conf.json'
$destination = Join-Path $tauriDirectory 'tauri.microsoftstore.release.conf.json'
$temporary = "$destination.tmp"

if (Test-Path -LiteralPath $destination) {
    throw "Refusing to overwrite existing generated config: $destination"
}
if (Test-Path -LiteralPath $temporary) {
    throw "Remove the stale generated temporary config before retrying: $temporary"
}

$thumbprint = $CertificateThumbprint.ToUpperInvariant()
$certificate = Get-Item -LiteralPath "Cert:\CurrentUser\My\$thumbprint" -ErrorAction Stop
if (-not $certificate.HasPrivateKey) {
    throw 'The selected certificate does not have an accessible private key.'
}
$codeSigningOid = '1.3.6.1.5.5.7.3.3'
$enhancedKeyUsages = @($certificate.EnhancedKeyUsageList | ForEach-Object { $_.ObjectId.Value })
if ($enhancedKeyUsages.Count -gt 0 -and $codeSigningOid -notin $enhancedKeyUsages) {
    throw 'The selected certificate is not valid for code signing.'
}
if ($certificate.NotAfter -le [DateTime]::UtcNow) {
    throw 'The selected certificate is expired.'
}

$config = Get-Content -Raw -LiteralPath $source | ConvertFrom-Json
$config.bundle.windows | Add-Member -NotePropertyName certificateThumbprint -NotePropertyValue $thumbprint
$config.bundle.windows | Add-Member -NotePropertyName digestAlgorithm -NotePropertyValue 'sha256'
$config.bundle.windows | Add-Member -NotePropertyName timestampUrl -NotePropertyValue 'http://timestamp.digicert.com'
$json = $config | ConvertTo-Json -Depth 10
$utf8WithoutBom = New-Object System.Text.UTF8Encoding($false)
[IO.File]::WriteAllText($temporary, $json, $utf8WithoutBom)
Move-Item -LiteralPath $temporary -Destination $destination
Write-Output "Prepared ignored signed Store overlay at $destination"
