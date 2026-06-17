# Cairn installer (Windows).
#
#   irm https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.ps1 | iex
#
# Honors: $env:CAIRN_REPO, $env:CAIRN_INSTALL_DIR, $env:CAIRN_VERSION, $env:CAIRN_INSTALL_SKIP_VERIFY.
$ErrorActionPreference = 'Stop'

$Repo       = if ($env:CAIRN_REPO)              { $env:CAIRN_REPO }              else { 'Vellixia/Cairn' }
$InstallDir = if ($env:CAIRN_INSTALL_DIR)       { $env:CAIRN_INSTALL_DIR }       else { "$env:LOCALAPPDATA\Cairn\bin" }
$Version    = if ($env:CAIRN_VERSION)           { $env:CAIRN_VERSION }           else { 'latest' }
$SkipVerify = ($env:CAIRN_INSTALL_SKIP_VERIFY -eq '1')
$BaseUrl    = "https://github.com/$Repo/releases"
$target     = 'x86_64-pc-windows-msvc'
$archive    = "cairn-$target.zip"
$sumsName   = 'SHA256SUMS'

function Write-Step($msg)  { Write-Host "› $msg" -ForegroundColor Cyan }
function Write-Warn($msg)  { Write-Host "⚠ $msg" -ForegroundColor Yellow }
function Fail($msg)        { Write-Host "✗ $msg" -ForegroundColor Red; exit 1 }

# Resolve 'latest' to the concrete tag name via the GitHub releases/latest redirect.
# If $env:CAIRN_VERSION is set, return it verbatim.
function Resolve-Version {
    param([string]$Requested)
    if ($Requested -ne 'latest') { return $Requested }
    try {
        $resp = Invoke-WebRequest -Uri "$BaseUrl/latest" -MaximumRedirection 5 -Method Head -ErrorAction Stop
        $final = $resp.Headers['Location'] | Select-Object -First 1
        if (-not $final) { Fail "Could not resolve latest release for $Repo (no Location header)." }
        # Final URL is https://github.com/<repo>/releases/tag/<tag>
        if ($final -match '/releases/tag/([^/?#]+)$') { return $Matches[1] }
        Fail "Unexpected redirect URL when resolving latest: $final"
    } catch {
        Fail "Could not resolve latest release for $Repo (network error: $($_.Exception.Message))"
    }
}

# Look up the expected SHA-256 for a given archive name in a SHA256SUMS manifest.
# Format: "<hex>  <filename>" per line; filenames may have leading "./" or CR/LF endings.
function Get-ExpectedHash {
    param([string]$SumsPath, [string]$ArchiveName)
    $expected = $null
    Get-Content -LiteralPath $SumsPath | ForEach-Object {
        $line = $_.TrimEnd("`r")
        if ($line -match '^([0-9a-fA-F]{64})\s+(.+)$') {
            $hex  = $Matches[1].ToLower()
            $name = $Matches[2]
            # Strip any leading "./" and any path prefix; compare base name.
            $base = [System.IO.Path]::GetFileName($name)
            if ($base -eq $ArchiveName) {
                $script:expected = $hex
            }
        }
    }
    return $expected
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$tag = Resolve-Version -Requested $Version
Write-Step "Installing cairn $tag ($target) -> $InstallDir"

$archiveUrl = "$BaseUrl/download/$tag/$archive"
$sumsUrl    = "$BaseUrl/download/$tag/$sumsName"
$zipPath    = Join-Path $env:TEMP $archive
$sumsPath   = Join-Path $env:TEMP $sumsName

try {
    Invoke-WebRequest -Uri $archiveUrl -OutFile $zipPath -ErrorAction Stop
} catch {
    if (Get-Command cargo -ErrorAction SilentlyContinue) {
        Write-Host "No prebuilt release found; building from source with cargo…"
        cargo install --git "https://github.com/$Repo" cairn-cli
        return
    }
    Fail "No prebuilt binary available for $target and cargo is not installed."
}

# Verify the archive before unpacking — defence against a compromised or partial download.
if ($SkipVerify) {
    Write-Warn "================================================================"
    Write-Warn "  !!! CAIRN_INSTALL_SKIP_VERIFY=1 set — checksum verification !!!"
    Write-Warn "  !!! SKIPPED. You are about to execute an UNVERIFIED binary.  !!!"
    Write-Warn "  !!! This is a SECURITY RISK. Only use for local debugging.    !!!"
    Write-Warn "================================================================"
} else {
    Write-Step "Verifying SHA-256 checksum…"
    try {
        Invoke-WebRequest -Uri $sumsUrl -OutFile $sumsPath -ErrorAction Stop
    } catch {
        Fail "Could not download SHA256SUMS from $sumsUrl — refusing to install unverified artifact. Re-run after a few seconds (the release job may still be finalizing) or pin CAIRN_VERSION to a known-good release."
    }
    $expected = Get-ExpectedHash -SumsPath $sumsPath -ArchiveName $archive
    if (-not $expected) {
        Fail "$archive not listed in $sumsName — refusing to install unverified artifact."
    }
    $actual = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $expected) {
        Fail "Checksum mismatch for ${archive}: expected $expected, got $actual"
    }
    Write-Step "Checksum OK ($actual)"
}

Expand-Archive -Path $zipPath -DestinationPath $InstallDir -Force
Remove-Item $zipPath -Force
if (Test-Path $sumsPath) { Remove-Item $sumsPath -Force }

# Add to the user PATH if missing.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$InstallDir", 'User')
    Write-Host "Added $InstallDir to your PATH (restart your shell)."
}

Write-Host "Done. Start the server with:  cairn serve"
