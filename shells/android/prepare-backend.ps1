#Requires -Version 7.4
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$backend = Join-Path $root 'vendor/gpui-mobile'
$patch = Join-Path $root 'patches/gpui-mobile/android-app.patch'
$pin = 'c7cab3a43970bd5f1e05d907695404ed73fcdc95'
if (!(Test-Path -LiteralPath (Join-Path $backend 'Cargo.toml'))) {
    & git -c core.autocrlf=false -C $root submodule update --init --depth 1 vendor/gpui-mobile
    if ($LASTEXITCODE -ne 0) { throw 'Could not initialize the pinned GPUI mobile backend.' }
}
if ((& git -C $backend rev-parse HEAD) -cne $pin) { throw "GPUI mobile must be at $pin." }
if (!(Test-Path -LiteralPath $patch)) { throw 'Android backend patch is missing.' }
$patchLines = @(Get-Content -LiteralPath $patch)
$added = @(
    for ($i = 0; $i -lt $patchLines.Count - 1; $i++) {
        if ($patchLines[$i] -eq '--- /dev/null' -and $patchLines[$i + 1] -match '^\+\+\+ b/') {
            $patchLines[$i + 1].Substring(6)
        }
    }
)
& git -C $backend apply --reverse --check $patch 2>$null
if ($LASTEXITCODE -eq 0) {
    $unexpected = @(& git -C $backend ls-files --others --exclude-standard | Where-Object { $_ -notin $added })
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect untracked backend files.' }
    if ($unexpected.Count) { throw 'GPUI mobile has additional untracked files; preserve them before preparing it.' }
    $oldIndex = $env:GIT_INDEX_FILE
    $temporaryIndex = Join-Path ([IO.Path]::GetTempPath()) "optchat-mobile-index-$([guid]::NewGuid())"
    try {
        # Preserve the real index's sparse-checkout flags. Include patch-created files in
        # the working diff without staging anything in the actual submodule index.
        $indexPath = & git -C $backend rev-parse --path-format=absolute --git-path index
        if ($LASTEXITCODE -ne 0) { throw 'Could not locate backend index.' }
        Copy-Item -LiteralPath $indexPath -Destination $temporaryIndex
        $env:GIT_INDEX_FILE = $temporaryIndex
        if ($added.Count) {
            & git -C $backend add --intent-to-add -- @added
            if ($LASTEXITCODE -ne 0) { throw 'Could not inspect patch-created backend files.' }
        }
        $actual = (& git -C $backend diff --binary HEAD --) -join "`n"
        if ($LASTEXITCODE -ne 0) { throw 'Could not read actual backend changes.' }
        & git -C $backend read-tree HEAD
        if ($LASTEXITCODE -ne 0) { throw 'Could not create verification index.' }
        & git -C $backend apply --cached $patch
        if ($LASTEXITCODE -ne 0) { throw 'Could not verify the backend patch.' }
        $expected = (& git -C $backend diff --cached --binary HEAD --) -join "`n"
        if ($actual -cne $expected) { throw 'GPUI mobile has additional edits; preserve them before preparing it.' }
    } finally {
        $env:GIT_INDEX_FILE = $oldIndex
        Remove-Item -LiteralPath $temporaryIndex -ErrorAction SilentlyContinue
    }
    Write-Output 'GPUI mobile pin and Android patch verified.'
    exit 0
}
if (@(& git -C $backend status --porcelain --untracked-files=no).Count) {
    throw 'GPUI mobile has local or partially applied edits; preserve them before preparing it.'
}
$affected = @($patchLines | Where-Object { $_ -match '^\+\+\+ b/' } | ForEach-Object { $_.Substring(6) } | Sort-Object -Unique)
$existing = @(& git -C $backend ls-tree -r --name-only HEAD -- @affected)
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect pinned backend files.' }
& git -c core.autocrlf=false -C $backend checkout HEAD -- @existing
if ($LASTEXITCODE -ne 0) { throw 'Could not prepare LF source files.' }
& git -C $backend apply --check $patch
if ($LASTEXITCODE -ne 0) { throw 'Backend patch does not apply to the pin.' }
& git -C $backend apply $patch
if ($LASTEXITCODE -ne 0) { throw 'Could not apply backend patch.' }
Write-Output 'Prepared GPUI mobile for OptChat Android.'
