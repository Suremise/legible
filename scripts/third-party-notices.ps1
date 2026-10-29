<#
.SYNOPSIS
    Writes THIRD-PARTY-NOTICES.txt: every crate in legible.exe, and the JavaScript it bundles.
.DESCRIPTION
    Part 1 is the crates of the Windows build (cargo-about, with about.toml and about.hbs),
    without dev or build dependencies, then the copyright notices of the crates whose licence
    text there is its template (see Get-CopyrightNotices), then where the source of the two
    MPL-2.0 crates is (third_party/rust/LICENSES.md). Part 2 is the bundled JavaScript: each file,
    the packages inside it with their versions and licences (third_party/js/LICENSES.md), then
    their licence texts (third_party/js/NOTICES.txt). Both third_party files are made by
    scripts/audit-js-licences.mjs when a pin moves.

    The file is committed: About shows it, and both packages ship it.
    -Check writes it to a temp file and fails if the committed one differs (package.ps1 runs that).
    Build-only: it needs cargo-about 0.9.2 (cargo install cargo-about --version 0.9.2 --locked
    --features cli) and makes no network requests.
#>
[CmdletBinding()]
param([switch]$Check)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# A licence text whose copyright line is still its template's placeholder, whatever the licence:
# "<year> <copyright holders>" (MIT), "<year> <owner>" (BSD), "[yyyy] [name of copyright owner]"
# (Apache's appendix), and the like. An email address in angle brackets isn't one.
$TemplateCopyright = '(?im)^\s*Copyright\b.*(<(?![^>]*@)[^>]+>|\[[^\]]+\])'

# The crates (name and version) under each licence block of cargo-about's text whose copyright
# line is a placeholder.
function Get-TemplatedCrates([string]$crateText) {
    $crates = [System.Collections.Generic.List[string]]::new()
    foreach ($block in ($crateText -split "(?m)^-{80}$")) {
        if ($block -notmatch $TemplateCopyright) { continue }
        foreach ($m in [regex]::Matches($block, '(?m)^  - (\S+) (\S+)(?: |$)')) {
            $crates.Add("$($m.Groups[1].Value) $($m.Groups[2].Value)")
        }
    }
    , @($crates | Sort-Object -Unique -CaseSensitive)
}

# Where a crate ships no filled-in licence file, cargo-about gives the licence's template, and
# the crate's copyright notices are left out. This collects them from the crate's own files, as
# built (its licence files' "Copyright" lines, any other notice file whole, such as a COPYING
# or NOTICE, and its SPDX-FileCopyrightText headers), for every crate under such a template,
# into a section of their own: offline, and in a fixed order, so -Check stays reproducible.
function Get-CopyrightNotices([string]$crateText, [string]$root) {
    $rule = '-' * 80
    $crates = Get-TemplatedCrates $crateText
    if ($crates.Count -eq 0) { return '' }

    Push-Location $root
    # cargo writes UTF-8; read it as such whatever the console's code page (authors' names
    # aren't all ASCII), so the output is the same however the script is started.
    $encoding = [Console]::OutputEncoding
    [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
    try {
        $metadata = & cargo metadata --format-version 1 --offline --locked | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed ($LASTEXITCODE)" }
    }
    finally {
        [Console]::OutputEncoding = $encoding
        Pop-Location
    }
    $dirs = @{}
    $authors = @{}
    foreach ($p in $metadata.packages) {
        $dirs["$($p.name) $($p.version)"] = Split-Path -Parent $p.manifest_path
        $authors["$($p.name) $($p.version)"] = @($p.authors)
    }

    $lines = @(
        $rule,
        'Copyright notices of the crates above whose licence text is a template',
        '',
        'These crates'' licence text above is its template ("<year> <owner>" and the like): their',
        'copyright notices are in their own files, collected here from the crates as built (their',
        'licence and notice files, and the SPDX-FileCopyrightText headers of their files).',
        ''
    )
    foreach ($crate in $crates) {
        $dir = $dirs[$crate]
        if (-not $dir) { throw "no source for $crate in cargo metadata" }
        $found = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
        # Standard licence texts give their "Copyright" lines; any other notice file (such as a
        # LICENSE-LIBM-MIT for code taken from elsewhere) is given whole, below.
        $licenceFiles = @(Get-ChildItem -Path $dir -File | Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE)' } | Sort-Object Name)
        $licenceDir = Join-Path $dir 'LICENSES'
        if (Test-Path $licenceDir) { $licenceFiles += Get-ChildItem -Path $licenceDir -File | Sort-Object Name }
        $whole = [System.Collections.Generic.List[object]]::new()
        foreach ($f in $licenceFiles) {
            $standard = $f.DirectoryName -eq $licenceDir -or $f.Name -match '^LICEN[CS]E(\.(md|txt))?$|^LICEN[CS]E[-_](MIT|APACHE)'
            if (-not $standard) { $whole.Add($f); continue }
            foreach ($line in (Get-Content -Encoding utf8 $f.FullName)) {
                $t = $line.Trim()
                # A notice, with its year or (c); not a template's placeholder.
                if ($t -match '^Copyright\s+(\(c\)|©|\d)' -and $t -notmatch '[<\[{]') { [void]$found.Add($t) }
            }
        }
        $sources = Get-ChildItem -Path $dir -Recurse -File -Include *.rs, *.toml, *.md
        foreach ($m in ($sources | Select-String -Pattern 'SPDX-FileCopyrightText:\s*(.+?)\s*$' -Encoding utf8)) {
            $holder = $m.Matches[0].Groups[1].Value
            if ($holder -notmatch '^(Copyright|©)') { $holder = "Copyright $holder" }
            [void]$found.Add($holder)
        }
        $lines += $crate
        if ($found.Count -gt 0) {
            $lines += @($found | ForEach-Object { "  $_" })
        }
        foreach ($f in $whole) {
            $lines += "  $($f.Name):"
            $lines += @((Get-Content -Encoding utf8 $f.FullName) | ForEach-Object { "    $_".TrimEnd() })
        }
        if ($found.Count -eq 0 -and $whole.Count -eq 0) {
            $names = @($authors[$crate] | Where-Object { $_ })
            if ($names.Count -gt 0) {
                $lines += "  No copyright notice in its files. Its authors, as its Cargo.toml names them: $($names -join ', ')"
            }
            else {
                $lines += '  No copyright notice in its files, and its Cargo.toml names no authors.'
            }
        }
        $lines += ''
    }
    return ($lines -join "`n")
}

$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'THIRD-PARTY-NOTICES.txt'
$work = Join-Path ([IO.Path]::GetTempPath()) ("notices-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $crates = Join-Path $work 'crates.txt'
    Push-Location $root
    try {
        & cargo about generate --workspace --offline --fail about.hbs -o $crates
        if ($LASTEXITCODE -ne 0) { throw "cargo about failed ($LASTEXITCODE)" }
    }
    finally { Pop-Location }

    $version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value
    $rule = '=' * 80
    $thin = '-' * 80
    $header = @(
        "THIRD-PARTY NOTICES FOR LEGIBLE $version",
        '',
        'Legible is under the MIT licence (LICENSE, beside this file). It is built from the',
        'third-party Rust crates in part 1, and it bundles the JavaScript in part 2. Their licences',
        'and notices follow.',
        '',
        $rule
    ) -join "`n"

    # The MPL-2.0 crates: where their source is, from the table in third_party/rust/LICENSES.md.
    $mpl = @($rule, 'Where the source of the MPL-2.0 crates is', '',
        'These crates are under the Mozilla Public License 2.0 (its text is in part 1). Legible',
        'ships them unmodified, compiled into legible.exe; their source is at these addresses (the',
        'SHA-256s are the crates'' checksums in Legible''s Cargo.lock).', '')
    $rows = Get-Content -Encoding utf8 (Join-Path $root 'third_party/rust/LICENSES.md') |
        Where-Object { $_ -match '^\| `([^`]+)` \| ([^|]+) \| MPL-2\.0 \|[^|]*\| (.+) \|$' }
    if (@($rows).Count -eq 0) { throw 'third_party/rust/LICENSES.md lists no MPL-2.0 crate.' }
    foreach ($row in $rows) {
        $null = $row -match '^\| `([^`]+)` \| ([^|]+) \| MPL-2\.0 \|[^|]*\| (.+) \|$'
        $mpl += "$($Matches[1]) $($Matches[2].Trim())"
        $where = $Matches[3] -replace '[<>]', '' -replace '`', '' -replace '; ', "`n  "
        $mpl += "  $where"
        $mpl += ''
    }
    $mpl = $mpl -join "`n"

    # Part 2: each bundled file and its packages, then their licence texts.
    $js = @($rule, 'PART 2: BUNDLED JAVASCRIPT', '',
        'Legible embeds these JavaScript files in legible.exe. Each is listed with the packages',
        'inside it, at the versions bundled, and their licences; the licence texts follow, each',
        'once, with the packages it covers.')
    $file = $null
    foreach ($line in (Get-Content -Encoding utf8 (Join-Path $root 'third_party/js/LICENSES.md'))) {
        if ($line -match '^## (\S+\.js)$') {
            $file = $Matches[1]
            $js += ''
            $js += $thin
            $js += $file
        }
        elseif ($file -and $line -match '^SHA-256 `([0-9a-f]{64})`, (.+)$') {
            $js += ($Matches[2] -replace '\[([^\]]+)\]\(([^)]+)\)', '$1 <$2>' -replace '`', '')
            $js += ''
        }
        elseif ($file -and $line -match '^\| `([^`]+)` \| ([^|]+) \| ([^|]+) \|') {
            $js += ('  - {0} {1} ({2})' -f $Matches[1], $Matches[2].Trim(), $Matches[3].Trim())
        }
    }
    if (-not ($js -match '^  - ')) { throw 'third_party/js/LICENSES.md lists no bundled package.' }
    $jsNotices = (Get-Content -Raw -Encoding utf8 (Join-Path $root 'third_party/js/NOTICES.txt')) -replace "`r`n", "`n"

    $crateText = (Get-Content -Raw -Encoding utf8 $crates) -replace "`r`n", "`n"
    $copyrights = Get-CopyrightNotices $crateText $root
    $text = ($header, $crateText.TrimEnd(), '', $copyrights, $mpl, ($js -join "`n"), '', $thin, $jsNotices.TrimEnd()) -join "`n"
    $text = ($text -replace "`r`n", "`n") + "`n"

    # Every crate whose licence text above is a template has its own notices below it (a
    # crate under a BSD or MIT template with no notice of its own would ship without one).
    $templated = Get-TemplatedCrates $crateText
    $missing = @($templated | Where-Object {
            $copyrights -notmatch "(?m)^$([regex]::Escape($_))`n  \S"
        })
    if ($missing) { throw "No copyright notices collected for $($missing -join ', '), whose licence text is a template." }

    if ($Check) {
        $committed = if (Test-Path $out) { (Get-Content -Raw -Encoding utf8 $out) -replace "`r`n", "`n" } else { '' }
        if ($committed -ne $text) {
            Write-Error 'THIRD-PARTY-NOTICES.txt is out of date: run scripts/third-party-notices.ps1 and commit it.'
            exit 1
        }
        Write-Host 'THIRD-PARTY-NOTICES.txt is up to date.'
    }
    else {
        [IO.File]::WriteAllText($out, $text, [Text.UTF8Encoding]::new($false))
        Write-Host "Wrote $out ($((Get-Item $out).Length) bytes)"
    }
}
finally {
    Remove-Item -Recurse -Force $work
}
