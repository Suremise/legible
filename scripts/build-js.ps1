<#
.SYNOPSIS
    Builds a JavaScript bundle Legible makes itself: third_party/js/<bundle>.min.js.
.DESCRIPTION
    -Bundle codemirror: CodeMirror 6 publishes no prebuilt browser bundle.
    -Bundle mermaid: Mermaid's prebuilt bundle includes elkjs (EPL-2.0), for its optional ELK
    layout; Legible's leaves it out (docs/ARCHITECTURE.md, Web assets).

    scripts/js/<bundle>/ holds the entry (entry.js), the exact packages (package.json and
    package-lock.json, esbuild among them) and any stubs. This copies them with scripts/js/build.mjs
    to a temp folder, installs exactly the locked versions (npm ci, no install scripts), bundles
    entry.js with the pinned esbuild as one minified script, and writes it to
    third_party/js/<bundle>.min.js. It also writes scripts/js/<bundle>/bundled-packages.txt, the
    packages esbuild put in the bundle, for the licence audit (scripts/audit-js-licences.mjs).

    The build is reproducible: the output must have the SHA-256 that third_party/js/VERSIONS
    pins, or the script fails and changes nothing. -Update (after changing the packages, the
    entry or a stub) writes the new output and its SHA-256 into VERSIONS; then rerun the licence
    audit. Build-only: needs Node.js and the npm registry, never shipped.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('codemirror', 'mermaid')][string]$Bundle,
    [switch]$Update
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path -Parent $PSScriptRoot
$source = Join-Path $root "scripts/js/$Bundle"
$versions = Join-Path $root 'third_party/js/VERSIONS'
$name = "$Bundle.min.js"
$output = Join-Path $root "third_party/js/$name"

function Get-Sha256($path) { (Get-FileHash -Algorithm SHA256 -Path $path).Hash.ToLowerInvariant() }

$work = Join-Path ([IO.Path]::GetTempPath()) ("legible-build-$Bundle-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    Get-ChildItem -Path $source -File | Where-Object Name -ne 'bundled-packages.txt' |
        Copy-Item -Destination $work
    Copy-Item -Path (Join-Path $root 'scripts/js/build.mjs') -Destination $work
    Push-Location $work
    try {
        npm ci --ignore-scripts --no-audit --no-fund --loglevel=error
        if ($LASTEXITCODE -ne 0) { throw 'npm ci failed.' }
        node build.mjs $name meta.json
        if ($LASTEXITCODE -ne 0) { throw 'esbuild failed.' }
        # The packages in the bundle, as name@version from each one's package.json: those with
        # code in the output (the metafile's inputs also list files tree-shaking left out).
        $packages = node -e @'
const fs = require("fs");
const output = Object.values(JSON.parse(fs.readFileSync("meta.json", "utf8")).outputs)[0];
const inputs = Object.entries(output.inputs).filter(([, i]) => i.bytesInOutput > 0).map(([p]) => p);
const names = new Set(inputs.map(p => p.match(/node_modules\/((?:@[^/]+\/)?[^/]+)/)?.[1]).filter(Boolean));
const lines = [...names].map(n => `${n}@${JSON.parse(fs.readFileSync(`node_modules/${n}/package.json`, "utf8")).version}`);
console.log(lines.sort().join("\n"));
'@
        if ($LASTEXITCODE -ne 0) { throw "Couldn't list the bundled packages." }
    }
    finally { Pop-Location }

    $built = Join-Path $work $name
    $have = Get-Sha256 $built
    $section = [regex]::Match((Get-Content -Raw $versions), '(?ms)^\[' + [regex]::Escape($name) + '\]\s*$(.*?)(?=^\[|\z)')
    if (-not $section.Success) { throw "VERSIONS has no [$name] section." }
    $pinned = [regex]::Match($section.Groups[1].Value, '(?m)^sha256=([0-9a-f]*)').Groups[1].Value
    if ($have -ne $pinned -and -not $Update) {
        throw "The build has SHA-256 $have, not the pinned $pinned. If the packages, entry.js or a stub changed on purpose, rerun with -Update."
    }

    Copy-Item -Path $built -Destination $output -Force
    $header = "# Written by scripts/build-js.ps1: the packages esbuild bundled into $name."
    Set-Content -Path (Join-Path $source 'bundled-packages.txt') -Value (@($header) + $packages) -Encoding utf8NoBOM
    if ($have -ne $pinned) {
        $text = (Get-Content -Raw $versions) -replace "(?ms)(^\[$([regex]::Escape($name))\].*?^sha256=)[0-9a-f]*", "`${1}$have"
        Set-Content -Path $versions -Value $text -NoNewline -Encoding utf8NoBOM
        Write-Host "Updated VERSIONS: $name is now $have. Rerun node scripts/audit-js-licences.mjs."
    }
    Write-Host "Built $name (SHA-256 $have)"
}
finally {
    Remove-Item -Recurse -Force -Path $work -ErrorAction SilentlyContinue
}
