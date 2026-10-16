#Requires -Version 7.4
# Native PowerShell equivalent of zed-vendor.sh. Never discards edits in the submodule.
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$zed = Join-Path $root 'vendor/zed'
$commit = '97dc71d8b8f79012c1441aa01121fba95667292a'

function Git-Zed {
    param([string[]]$Arguments)
    & git -C $zed @Arguments
    if ($LASTEXITCODE -ne 0) { throw "git $($Arguments -join ' ') failed" }
}

if (-not (Test-Path "$zed/Cargo.toml")) {
    & git -c core.autocrlf=false -C $root submodule update --init --depth 1 vendor/zed
    if ($LASTEXITCODE -ne 0) { throw 'Initialize the pinned vendor/zed submodule before building.' }
}
if ((Git-Zed @('rev-parse', 'HEAD')) -ne $commit) { throw "vendor/zed must be at $commit" }
# Cargo resolves Android's backend manifest even when building a desktop shell.
if (Test-Path "$root/shells/android/prepare-backend.ps1") {
    & "$root/shells/android/prepare-backend.ps1"
    if ($LASTEXITCODE -ne 0) { throw 'Could not prepare the workspace Android backend.' }
}
$patches = @(Get-ChildItem "$root/patches/zed/*.patch" | Sort-Object Name)
if ($env:ZED_OPTIONAL_PATCHES -eq '1') {
    $patches += @(Get-ChildItem "$root/patches/zed/optional/*.patch" | Sort-Object Name)
}
if ($patches.Count -eq 0) { throw 'No GPUI patches found.' }
$alreadyApplied = $true
foreach ($patch in $patches) {
    & git -C $zed apply --reverse --check $patch.FullName 2>$null
    if ($LASTEXITCODE -ne 0) { $alreadyApplied = $false }
}
if ($alreadyApplied) {
    # Compare the full diff against a temporary index so extra edits in patched files are detected too.
    $oldIndex = $env:GIT_INDEX_FILE
    $index = Join-Path ([System.IO.Path]::GetTempPath()) "optchat-index-$([guid]::NewGuid())"
    try {
        $actual = @(Git-Zed @('diff', '--binary', 'HEAD', '--')) -join "`n"
        $env:GIT_INDEX_FILE = $index
        Git-Zed @('read-tree', 'HEAD')
        foreach ($patch in $patches) { Git-Zed @('apply', '--cached', $patch.FullName) }
        $expected = @(Git-Zed @('diff', '--cached', '--binary', 'HEAD', '--')) -join "`n"
        if ($actual -cne $expected) { throw 'vendor/zed has edits beyond the expected patches; preserve them before building.' }
    } finally {
        $env:GIT_INDEX_FILE = $oldIndex
        Remove-Item $index -ErrorAction SilentlyContinue
    }
    Write-Host "vendor/zed at $($commit.Substring(0, 7)) + $($patches.Count) patches (already prepared)"
    exit 0
}
if (@(Git-Zed @('status', '--porcelain', '--untracked-files=no')).Count -gt 0) {
    throw 'vendor/zed has edits or partially applied patches. Preserve them and restore the pin before rebuilding.'
}
# A normal Git for Windows checkout may use CRLF. Re-check out only the clean files
# touched by our patches with LF; never rewrite a file before checking for user edits.
$affected = @($patches | ForEach-Object {
    Get-Content -LiteralPath $_.FullName | Where-Object { $_ -match '^\+\+\+ b/' } |
        ForEach-Object { $_.Substring(6) }
} | Sort-Object -Unique)
Git-Zed (@('-c', 'core.autocrlf=false', 'checkout', 'HEAD', '--') + $affected)
# Check the entire series first, so a bad patch cannot leave the checkout half prepared.
$paths = @($patches | ForEach-Object FullName)
Git-Zed (@('apply', '--check') + $paths)
Git-Zed (@('apply') + $paths)
Write-Host "vendor/zed at $($commit.Substring(0, 7)) + $($patches.Count) patches applied"
