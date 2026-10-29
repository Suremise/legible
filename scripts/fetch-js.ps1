<#
.SYNOPSIS
    Verifies, and if need be fetches, the pinned JavaScript bundles in third_party/js/.
.DESCRIPTION
    Reads third_party/js/VERSIONS. Each file there is kept only when its SHA-256 is the pinned
    one. Otherwise a file from npm is downloaded again from its tarball, which is checked against
    the registry's pinned sha512 integrity before the file is taken out of it and checked against
    its own SHA-256; a file Legible builds (CodeMirror) is rebuilt by its build script.
    Build-only: Legible itself never downloads anything.

    -VerifyOnly never downloads or builds: it fails unless every file is the pinned one (CI runs
    this). -Force fetches and rebuilds every file.
#>
[CmdletBinding()]
param([switch]$VerifyOnly, [switch]$Force)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path -Parent $PSScriptRoot
$jsDir = Join-Path $root 'third_party/js'

function Get-Sha256($path) { (Get-FileHash -Algorithm SHA256 -Path $path).Hash.ToLowerInvariant() }

function Get-Sri512($path) {
    $sha = [Security.Cryptography.SHA512]::Create()
    $stream = [IO.File]::OpenRead($path)
    try { 'sha512-' + [Convert]::ToBase64String($sha.ComputeHash($stream)) }
    finally { $stream.Dispose(); $sha.Dispose() }
}

# VERSIONS: [file] sections of key=value lines; '#' starts a comment line.
function Read-Pins($path) {
    $pins = [ordered]@{}
    $current = $null
    foreach ($line in Get-Content $path) {
        if ($line -match '^\s*(#|$)') { continue }
        if ($line -match '^\[(.+)\]$') {
            $current = [ordered]@{ file = $Matches[1]; inventory = @() }
            $pins[$Matches[1]] = $current
        }
        elseif ($line -match '^\s*([a-z0-9_]+)\s*=\s*(.+?)\s*$' -and $current) {
            if ($Matches[1] -eq 'inventory') { $current.inventory += $Matches[2] }
            else { $current[$Matches[1]] = $Matches[2] }
        }
        else { throw "third_party/js/VERSIONS: can't read '$line'." }
    }
    $pins
}

$pins = Read-Pins (Join-Path $jsDir 'VERSIONS')
if ($pins.Count -eq 0) { throw 'third_party/js/VERSIONS pins no files.' }

$failed = @()
foreach ($pin in $pins.Values) {
    $name = $pin.file
    if (-not $pin.Contains('sha256')) { throw "VERSIONS: [$name] has no sha256." }
    $want = $pin.sha256.ToLowerInvariant()
    $file = Join-Path $jsDir $name

    if ((Test-Path $file) -and -not $Force) {
        $have = Get-Sha256 $file
        if ($have -eq $want) { Write-Host "OK  $name (SHA-256 verified)"; continue }
        if ($VerifyOnly) {
            $failed += "$name has SHA-256 $have, not the pinned $want."
            continue
        }
        Write-Warning "$name isn't the pinned file (SHA-256 $have): fetching it again."
    }
    elseif ($VerifyOnly) {
        $failed += "$name is missing: run scripts/fetch-js.ps1."
        continue
    }

    if ($pin.Contains('build')) {
        Write-Host "Building $name with scripts/build-js.ps1 -Bundle $($pin.build)"
        & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'build-js.ps1') -Bundle $pin.build
        if ($LASTEXITCODE -ne 0) { throw "Building $name failed." }
        continue
    }

    foreach ($key in 'tarball', 'integrity', 'path') {
        if (-not $pin.Contains($key)) { throw "VERSIONS: [$name] has no '$key'." }
    }
    $work = Join-Path ([IO.Path]::GetTempPath()) ("legible-fetch-js-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        $tgz = Join-Path $work 'package.tgz'
        Write-Host "Downloading $($pin.tarball)"
        $ProgressPreference = 'SilentlyContinue'
        Invoke-WebRequest -Uri $pin.tarball -OutFile $tgz -UseBasicParsing
        $integrity = Get-Sri512 $tgz
        if ($integrity -ne $pin.integrity) {
            throw "The tarball for $name has integrity $integrity, not the pinned $($pin.integrity)."
        }
        # Windows' own tar: a Git for Windows tar earlier on the PATH reads "C:" as a host name.
        & (Join-Path $env:SystemRoot 'System32/tar.exe') -xzf $tgz -C $work $pin.path
        if ($LASTEXITCODE -ne 0) { throw "Couldn't extract $($pin.path) from $($pin.tarball)." }
        $extracted = Join-Path $work $pin.path
        $have = Get-Sha256 $extracted
        if ($have -ne $want) {
            throw "$($pin.path) in $($pin.tarball) has SHA-256 $have, not the pinned $want."
        }
        Copy-Item -Path $extracted -Destination $file -Force
        Write-Host "Fetched $name (SHA-256 verified)"
    }
    finally {
        Remove-Item -Recurse -Force -Path $work -ErrorAction SilentlyContinue
    }
}

# Every bundle in the folder is pinned: nothing unaudited is embedded.
$unpinned = @(Get-ChildItem -Path $jsDir -File -Filter '*.js' | Where-Object { -not $pins.Contains($_.Name) })
foreach ($extra in $unpinned) { $failed += "$($extra.Name) is in third_party/js but not in VERSIONS." }

# LICENSES.md records the audit of exactly these files.
$licences = Join-Path $jsDir 'LICENSES.md'
if (-not (Test-Path $licences)) {
    $failed += 'third_party/js/LICENSES.md is missing: run node scripts/audit-js-licences.mjs.'
}
else {
    $audited = Get-Content -Raw $licences
    foreach ($pin in $pins.Values) {
        if (-not $audited.Contains($pin.sha256.ToLowerInvariant())) {
            $failed += "LICENSES.md doesn't record $($pin.file) at its pinned SHA-256: rerun node scripts/audit-js-licences.mjs."
        }
    }
}

if ($failed) {
    $failed | ForEach-Object { Write-Error $_ -ErrorAction Continue }
    exit 1
}
Write-Host "All $($pins.Count) bundles are the pinned, audited files."
