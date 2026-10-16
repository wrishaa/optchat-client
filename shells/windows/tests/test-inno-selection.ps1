#Requires -Version 7.4
# No compiler is executed: verify that a runner's stale PATH cannot override the pinned tool.
. "$PSScriptRoot/../release/common.ps1"
$root = (Resolve-Path "$PSScriptRoot/../../..").Path
$scratch = Join-Path $root "build/windows/inno-selection-$([guid]::NewGuid())"
$pinnedDirectory = Join-Path $scratch 'Verified Inno Setup 6'
$shadowDirectory = Join-Path $scratch 'old-path'
$explicitDirectory = Join-Path $scratch 'explicit'
$null = New-Item -ItemType Directory -Force $pinnedDirectory, $shadowDirectory, $explicitDirectory
$pinned = Join-Path $pinnedDirectory 'ISCC.exe'
$shadow = Join-Path $shadowDirectory 'ISCC.exe'
$explicit = Join-Path $explicitDirectory 'ISCC.exe'
foreach ($path in @($pinned, $shadow, $explicit)) {
    Set-Content -LiteralPath $path -Value 'Synthetic selection fixture; never execute.'
    if (-not $IsWindows) {
        [IO.File]::SetUnixFileMode($path, [IO.UnixFileMode]::UserRead -bor [IO.UnixFileMode]::UserWrite -bor [IO.UnixFileMode]::UserExecute)
    }
}
$oldPath = $env:PATH
$oldCompiler = $env:OPTCHAT_INNO_COMPILER
try {
    $env:PATH = $shadowDirectory + [IO.Path]::PathSeparator + $oldPath
    if ((Get-Command ISCC.exe -ErrorAction Stop).Source -cne $shadow) { throw 'Fixture did not shadow the compiler on PATH.' }
    $env:OPTCHAT_INNO_COMPILER = $pinned
    if ((Find-InnoCompiler) -cne $pinned) { throw 'The verified compiler did not override the old PATH shim.' }
    if ((Find-InnoCompiler $explicit) -cne $explicit) { throw 'An explicit compiler path must take precedence.' }
    $env:OPTCHAT_INNO_COMPILER = Join-Path $scratch 'missing.exe'
    $rejected = $false
    try { $null = Find-InnoCompiler } catch { $rejected = $true }
    if (-not $rejected) { throw 'A missing pinned compiler must fail instead of falling back to PATH.' }
    Write-Host 'PASS: pinned Inno compiler overrides PATH; explicit paths win; missing pinned tools fail closed.'
} finally {
    $env:PATH = $oldPath
    $env:OPTCHAT_INNO_COMPILER = $oldCompiler
    Remove-BuildDirectory $root $scratch
}
