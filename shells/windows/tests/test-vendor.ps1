#Requires -Version 7.4
# Exercise Windows-style CRLF checkouts and edit preservation in disposable Git repositories.
. "$PSScriptRoot/../release/common.ps1"
$root = (Resolve-Path "$PSScriptRoot/../../..").Path
$scratch = Join-Path $root "build/windows/vendor-fixture-$([guid]::NewGuid())"
$source = Join-Path $scratch 'source'
$fixture = Join-Path $scratch 'fixture'
$zed = Join-Path $fixture 'vendor/zed'
$null = New-Item -ItemType Directory -Force $source, "$fixture/tools", "$fixture/patches/zed"
try {
    Invoke-Native git @('-C', $source, 'init', '--quiet')
    Invoke-Native git @('-C', $source, 'config', 'user.name', 'Packaging Test')
    Invoke-Native git @('-C', $source, 'config', 'user.email', 'fixture@example.invalid')
    Invoke-Native git @('-C', $source, 'config', 'core.autocrlf', 'false')
    Write-Utf8 "$source/Cargo.toml" "# Synthetic vendor checkout`n"
    Write-Utf8 "$source/one.rs" "first`nsecond`n"
    Write-Utf8 "$source/two.rs" "alpha`nbeta`n"
    Invoke-Native git @('-C', $source, 'add', '.')
    Invoke-Native git @('-C', $source, 'commit', '--quiet', '-m', 'fixture')
    $pin = & git -C $source rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read fixture commit.' }
    Write-Utf8 "$source/one.rs" "first`nchanged`n"
    Write-Utf8 "$source/two.rs" "alpha`nmodified`n"
    foreach ($name in @('one', 'two')) {
        $patch = & git -C $source diff -- "$name.rs"
        if ($LASTEXITCODE -ne 0) { throw 'Cannot prepare fixture patch.' }
        Write-Utf8 "$fixture/patches/zed/$name.patch" (($patch -join "`n") + "`n")
    }
    Invoke-Native git @('-c', 'core.autocrlf=true', 'clone', '--quiet', $source, $zed)
    Invoke-Native git @('-C', $zed, 'config', 'core.autocrlf', 'true')
    if (-not ([IO.File]::ReadAllText("$zed/one.rs").Contains("`r`n"))) { throw 'Fixture did not create a CRLF checkout.' }
    $helper = Get-Content "$root/tools/zed-vendor.ps1" -Raw
    # The fixture has its own commit; the real helper's pin is exercised against vendor/zed separately.
    $helper = $helper -replace "(?m)^\`$commit = '[a-f0-9]{40}'", "`$commit = '$pin'"
    Write-Utf8 "$fixture/tools/zed-vendor.ps1" $helper
    & "$fixture/tools/zed-vendor.ps1"
    if ((Get-Content "$zed/one.rs" -Raw).Replace("`r`n", "`n") -cne "first`nchanged`n") { throw 'Patch failed on CRLF checkout.' }
    $before = (Get-FileHash "$zed/one.rs").Hash
    & "$fixture/tools/zed-vendor.ps1"
    if ((Get-FileHash "$zed/one.rs").Hash -ne $before) { throw 'Idempotent preparation rewrote the source.' }
    [IO.File]::AppendAllText("$zed/one.rs", "user edit`n")
    $before = (Get-FileHash "$zed/one.rs").Hash
    $rejected = $false
    try { & "$fixture/tools/zed-vendor.ps1" } catch { $rejected = $true }
    if (-not $rejected -or (Get-FileHash "$zed/one.rs").Hash -ne $before) { throw 'Vendor helper did not preserve user edits.' }
    Write-Host 'PASS: vendor preparation from CRLF, idempotence and preservation of extra edits in a patched file.'
} finally { Remove-BuildDirectory $root $scratch }
