<#
.SYNOPSIS
    Builds Legible's release files: the MSI, the portable zip and their SHA-256 checksums.
.DESCRIPTION
    Into target\dist (or -OutDir):
      legible-<version>-x64.msi           per user, no admin (installer\legible.wxs)
      legible-<version>-portable.zip      the exe, the "portable" marker, the licences
      SHA256SUMS.txt                      the checksums of both, as `sha256sum -c` reads them

    Both hold legible.exe, LICENSE.txt and THIRD-PARTY-NOTICES.txt. It fails if
    THIRD-PARTY-NOTICES.txt is out of date (scripts\third-party-notices.ps1 -Check) or the bundled
    JavaScript isn't the pinned, audited files (scripts\fetch-js.ps1 -VerifyOnly), and builds the
    exe with `cargo build --release`.

    The exe is checked to carry a version resource of the crate's version, as the MSI's File
    table does, and the app and file icons the associations name; and to import no C runtime DLL
    and nothing that isn't part of Windows (the C runtime is linked in: .cargo\config.toml), so it
    starts on a clean PC. The MSI's own text comes from locales\en\window.ftl (installer-*), and
    the built MSI is checked to carry it (decompiled with `wix msi decompile`); to upgrade any
    earlier install, the same version included; to register the file associations without taking
    over a default; and to open Legible after an interactive install only. The zip is checked to
    hold exactly its four files.

    Build-only tools: the Rust toolchain, cargo-about 0.9.2, and WiX 5.0.2
    (dotnet tool install --global wix --version 5.0.2). Nothing is signed yet.
#>
[CmdletBinding()]
param(
    [string]$OutDir,
    # Skips `cargo build --release` (release.yml builds first).
    [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $root 'target\dist' }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value

# The MSI's own text, from Fluent: the installer-* messages, one line of plain text each.
function Read-InstallerText([string]$path) {
    $messages = @{}
    $lines = @(Get-Content -Encoding utf8 $path)
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^(installer-[a-z0-9-]*)\s*=\s*(.*)$') {
            $next = if ($i + 1 -lt $lines.Count) { $lines[$i + 1] } else { '' }
            if ($Matches[2] -match '[{}]' -or -not $Matches[2].Trim() -or $next -match '^\s+\S') {
                throw "$path`: $($Matches[1]) must be one line of plain text."
            }
            $messages[$Matches[1]] = $Matches[2].Trim()
        }
    }
    $messages
}
$installerText = Read-InstallerText (Join-Path $root 'locales\en\window.ftl')
foreach ($key in 'installer-downgrade', 'installer-shortcut-description', 'installer-file-type', 'installer-app-description') {
    if (-not $installerText.ContainsKey($key)) { throw "locales\en\window.ftl has no $key." }
}
# MSI "formatted" text, where [name] is a property: brackets are escaped.
function Format-Msi([string]$text) { [regex]::Replace($text, '[\[\]]', { "[\$($args[0].Value)]" }) }
$downgrade = Format-Msi $installerText['installer-downgrade']
$shortcut = $installerText['installer-shortcut-description']
$fileType = Format-Msi $installerText['installer-file-type']
$appDescription = Format-Msi $installerText['installer-app-description']

& (Join-Path $PSScriptRoot 'fetch-js.ps1') -VerifyOnly
& (Join-Path $PSScriptRoot 'third-party-notices.ps1') -Check
if (-not $NoBuild) {
    Push-Location $root
    try {
        & cargo build --release --locked --bin legible
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    }
    finally { Pop-Location }
}

# The DLLs a 64-bit PE file imports, from its import directory.
function Get-ImportedDlls([string]$path) {
    $b = [IO.File]::ReadAllBytes($path)
    $pe = [BitConverter]::ToInt32($b, 0x3C)
    $sections = [BitConverter]::ToUInt16($b, $pe + 6)
    $optional = $pe + 24
    $table = $optional + [BitConverter]::ToUInt16($b, $pe + 20)
    function ToOffset([int]$rva) {
        for ($i = 0; $i -lt $sections; $i++) {
            $s = $table + $i * 40
            $va = [BitConverter]::ToInt32($b, $s + 12)
            $size = [Math]::Max([BitConverter]::ToInt32($b, $s + 8), [BitConverter]::ToInt32($b, $s + 16))
            if ($rva -ge $va -and $rva -lt $va + $size) { return $rva - $va + [BitConverter]::ToInt32($b, $s + 20) }
        }
        throw "RVA $rva isn't in a section of $path"
    }
    $entry = ToOffset ([BitConverter]::ToInt32($b, $optional + 120))
    $dlls = @()
    while (($name = [BitConverter]::ToInt32($b, $entry + 12)) -ne 0) {
        $at = ToOffset $name
        $end = [Array]::IndexOf($b, [byte]0, $at)
        $dlls += [Text.Encoding]::ASCII.GetString($b, $at, $end - $at).ToLowerInvariant()
        $entry += 20
    }
    $dlls
}

# What both packages hold.
$stage = Join-Path $OutDir 'stage'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
$exe = Join-Path $stage 'legible.exe'
Copy-Item (Join-Path $root 'target\release\legible.exe') $exe
# It starts on a clean PC: no C runtime DLL (it's linked in), nothing that isn't Windows' own.
$system32 = Join-Path $env:SystemRoot 'System32'
$foreign = Get-ImportedDlls $exe | Where-Object {
    $_ -match '^(vcruntime|msvcp|ucrtbase|api-ms-win-crt-)' -or
    -not ($_ -match '^(api-ms-win-|ext-ms-)' -or (Test-Path (Join-Path $system32 $_)))
}
if ($foreign) { throw "legible.exe imports $($foreign -join ', '): link the C runtime statically (.cargo\config.toml)." }
# It carries a version resource of the crate's version (crates\legible\build.rs): Details shows
# it, and the installer compares it with the installed exe's.
$fileVersion = (@($version -split '[.+-]' | Select-Object -First 3) + '0') -join '.'
$info = (Get-Item $exe).VersionInfo
if ($info.FileVersionRaw -ne [version]$fileVersion -or $info.ProductVersion -ne $version -or $info.ProductName -ne 'Legible') {
    throw "legible.exe's version resource is '$($info.ProductName)' $($info.ProductVersion) ($($info.FileVersionRaw)), not Legible $version ($fileVersion)."
}
# The icons the shortcut and the file associations name: the app's (ID 1) and the file's (ID 2,
# as "legible.exe",-2).
Add-Type -AssemblyName System.Drawing
foreach ($id in 1, 2) {
    $icon = [System.Drawing.Icon]::ExtractIcon($exe, -$id, 32)
    if (-not $icon) { throw "legible.exe has no icon resource $id." }
    $icon.Dispose()
}
Copy-Item (Join-Path $root 'LICENSE') (Join-Path $stage 'LICENSE.txt')
Copy-Item (Join-Path $root 'THIRD-PARTY-NOTICES.txt') $stage

# The MSI.
$msi = Join-Path $OutDir "legible-$version-x64.msi"
& wix build (Join-Path $root 'installer\legible.wxs') -arch x64 -d "Version=$version" -d "Stage=$stage" `
    -d "DowngradeMessage=$downgrade" -d "ShortcutDescription=$shortcut" `
    -d "FileType=$fileType" -d "AppDescription=$appDescription" -o $msi
if ($LASTEXITCODE -ne 0) { throw "wix build failed ($LASTEXITCODE)" }
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($msi, '.wixpdb'))

# The built MSI carries the Fluent text, as it is.
$decompiled = Join-Path $OutDir 'msi-check.wxs'
& wix msi decompile $msi -o $decompiled
if ($LASTEXITCODE -ne 0) { throw "wix msi decompile failed ($LASTEXITCODE)" }
$xml = [xml](Get-Content -Raw -Encoding utf8 $decompiled)
Remove-Item -Force $decompiled
$ns = @{ w = 'http://wixtoolset.org/schemas/v4/wxs' }
$registryValue = {
    param($key, $name)
    $nodes = Select-Xml -Xml $xml -XPath '//w:RegistryValue' -Namespace $ns | ForEach-Object { $_.Node } |
        Where-Object { $_.Key -eq $key -and "$($_.GetAttribute('Name'))" -eq "$name" }
    @($nodes | ForEach-Object { $_.Value })[0]
}
$inMsi = @{
    'installer-downgrade'            = (Select-Xml -Xml $xml -XPath '//w:MajorUpgrade' -Namespace $ns).Node.DowngradeErrorMessage
    'installer-shortcut-description' = (Select-Xml -Xml $xml -XPath '//w:Shortcut' -Namespace $ns).Node.Description
    'installer-file-type'            = & $registryValue 'Software\Classes\Legible.Markdown' ''
    'installer-app-description'      = & $registryValue 'Software\Legible\Capabilities' 'ApplicationDescription'
}
$expected = @{
    'installer-downgrade'            = $downgrade
    'installer-shortcut-description' = $shortcut
    'installer-file-type'            = $fileType
    'installer-app-description'      = $appDescription
}
foreach ($key in $expected.Keys) {
    if ($inMsi[$key] -ne $expected[$key]) {
        throw "The MSI's $key is '$($inMsi[$key])', not window.ftl's '$($expected[$key])'."
    }
}
# Its tables, read with Windows Installer's own API (opened read-only: nothing is installed).
# `wix msi decompile` 5.0.2 gives MajorUpgrade's Schedule as afterInstallFinalize whatever it is,
# so the upgrade is checked here rather than in the decompiled source.
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $installer, @([string]$msi, [int]0))
# Each row of a query, as an array of its fields' text.
function Get-MsiRows([string]$query) {
    $view = $database.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $database, @($query))
    $view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
    while ($record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)) {
        $count = $record.GetType().InvokeMember('FieldCount', 'GetProperty', $null, $record, $null)
        , @(1..$count | ForEach-Object { $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, $_) })
    }
    $view.GetType().InvokeMember('Close', 'InvokeMethod', $null, $view, $null) | Out-Null
}
try {
    # A same-version MSI upgrades too (AllowSameVersionUpgrades: the upgrade row's VersionMax is
    # this version, inclusive), else a rebuild installs beside the old one and keeps its exe.
    $upgrade = @(Get-MsiRows "SELECT ``VersionMax``, ``Attributes`` FROM ``Upgrade`` WHERE ``ActionProperty`` = 'WIX_UPGRADE_DETECTED'")
    $inclusive = 0x200  # msidbUpgradeAttributesVersionMaxInclusive
    if ($upgrade.Count -ne 1 -or $upgrade[0][0] -ne $version -or -not ([int]$upgrade[0][1] -band $inclusive)) {
        throw "The MSI doesn't upgrade an install of its own version (AllowSameVersionUpgrades): its Upgrade row is '$($upgrade | ForEach-Object { $_ -join ', ' })'."
    }
    # Every earlier install is removed before this one installs (Schedule="afterInstallInitialize").
    $sequence = @{}
    Get-MsiRows "SELECT ``Action``, ``Sequence`` FROM ``InstallExecuteSequence``" | ForEach-Object { $sequence[$_[0]] = [int]$_[1] }
    $removeAt = $sequence['RemoveExistingProducts']
    $between = $sequence.Values | Where-Object { $_ -gt $sequence['InstallInitialize'] -and $_ -lt $removeAt }
    if (-not $removeAt -or $removeAt -le $sequence['InstallInitialize'] -or $between) {
        throw "The MSI's RemoveExistingProducts is at $removeAt, not just after InstallInitialize ($($sequence['InstallInitialize']))."
    }
    # Its File table has the exe's version, which the installer compares on an upgrade.
    $inTable = @(Get-MsiRows "SELECT ``Version`` FROM ``File`` WHERE ``File`` = 'LegibleExe'")
    if ($inTable.Count -ne 1 -or $inTable[0][0] -ne $fileVersion) {
        throw "The MSI's legible.exe has version '$($inTable | ForEach-Object { $_ })', not $fileVersion."
    }
    # Legible opens after an interactive install only (UILevel 5 is the full UI; /qn and /qb
    # are 2 and 3), never on an uninstall.
    $launch = @(Get-MsiRows "SELECT ``Condition`` FROM ``InstallExecuteSequence`` WHERE ``Action`` = 'LaunchLegible'")
    if ($launch.Count -ne 1 -or $launch[0][0] -ne 'UILevel > 3 AND NOT (REMOVE~="ALL")') {
        throw "The MSI opens Legible on '$($launch | ForEach-Object { $_ })', not after an interactive install only."
    }
    # The associations, per user: offered for each extension, and never anyone's default.
    $registry = @(Get-MsiRows "SELECT ``Root``, ``Key``, ``Name``, ``Value`` FROM ``Registry``")
    if ($registry | Where-Object { $_[0] -ne '1' }) { throw 'The MSI writes outside HKEY_CURRENT_USER.' }
    foreach ($ext in '.md', '.markdown', '.mdown') {
        if (-not ($registry | Where-Object { $_[1] -eq "Software\Classes\$ext\OpenWithProgids" -and $_[2] -eq 'Legible.Markdown' })) {
            throw "The MSI doesn't offer Legible for $ext (OpenWithProgids)."
        }
        if ($registry | Where-Object { $_[1] -eq "Software\Classes\$ext" -or $_[1] -match 'UserChoice' }) {
            throw "The MSI sets $ext's default: Windows keeps that the user's choice."
        }
    }
}
finally {
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($database)
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($installer)
}

# The portable zip: the same files and the empty marker that keeps everything beside the exe.
$portable = Join-Path $OutDir 'portable'
if (Test-Path $portable) { Remove-Item -Recurse -Force $portable }
Copy-Item -Recurse $stage $portable
New-Item -ItemType File -Path (Join-Path $portable 'portable') | Out-Null
$zip = Join-Path $OutDir "legible-$version-portable.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $portable '*') -DestinationPath $zip
Remove-Item -Recurse -Force $portable
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($zip)
try { $inZip = @($archive.Entries | ForEach-Object { $_.FullName } | Sort-Object) }
finally { $archive.Dispose() }
$wanted = @('LICENSE.txt', 'THIRD-PARTY-NOTICES.txt', 'legible.exe', 'portable') | Sort-Object
if (($inZip -join ',') -ne ($wanted -join ',')) { throw "The zip holds $($inZip -join ', '), not $($wanted -join ', ')." }

# The checksums, as `sha256sum -c SHA256SUMS.txt` reads them.
$sums = foreach ($file in $msi, $zip) {
    '{0}  {1}' -f (Get-FileHash -Algorithm SHA256 $file).Hash.ToLowerInvariant(), (Split-Path -Leaf $file)
}
[IO.File]::WriteAllText((Join-Path $OutDir 'SHA256SUMS.txt'), (($sums -join "`n") + "`n"))
Remove-Item -Recurse -Force $stage

Write-Host "Built in ${OutDir}:"
Get-ChildItem $OutDir -File | ForEach-Object { Write-Host ("  {0} ({1:N0} bytes)" -f $_.Name, $_.Length) }
