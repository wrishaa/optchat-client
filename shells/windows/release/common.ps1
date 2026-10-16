#Requires -Version 7.4
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-Native {
    param([Parameter(Mandatory)][string]$FilePath, [string[]]$Arguments = @())
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$FilePath exited with code $LASTEXITCODE" }
}

function Assert-Windows {
    if (-not $IsWindows) { throw 'Build, package and sign on Windows with MSVC and the Windows SDK.' }
}

function Find-InnoCompiler {
    param([string]$Path)
    # CI installs a checksum-verified compiler, which must win over runner PATH shims.
    if (-not $Path -and $env:OPTCHAT_INNO_COMPILER) { $Path = $env:OPTCHAT_INNO_COMPILER }
    if ($Path) {
        return (Resolve-Path -LiteralPath $Path).Path
    }
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles, "$env:LOCALAPPDATA/Programs")) {
        foreach ($version in @('7', '6')) {
            $candidate = Join-Path $base "Inno Setup $version/ISCC.exe"
            if (Test-Path $candidate) { return $candidate }
        }
    }
    throw 'Inno Setup 6.7.3 is required on the packaging machine. Install it from jrsoftware.org or pass -Compiler to build-installer.ps1.'
}

function Find-SdkTool {
    param([Parameter(Mandatory)][string]$Name)
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $sdk = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
    $hostArchitecture = if ([System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -eq 'Arm64') { 'arm64' } else { 'x64' }
    $versions = @(Get-ChildItem $sdk -Directory | Where-Object Name -match '^10\.0\.\d+\.0$' |
        Sort-Object { [version]$_.Name } -Descending)
    foreach ($version in $versions) {
        foreach ($arch in @($hostArchitecture, 'x64')) {
            $candidate = Join-Path $version.FullName "$arch/$Name"
            if (Test-Path $candidate) { return $candidate }
        }
    }
    throw "Cannot find $Name. Install Windows SDK 10.0.20348 or newer."
}

function Get-PeArchitecture {
    param([Parameter(Mandatory)][string]$Path)
    $stream = [System.IO.File]::OpenRead($Path)
    $reader = [System.IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw "Not a Windows executable: $Path" }
        $stream.Position = 0x3c
        $offset = $reader.ReadInt32()
        if ($offset -lt 0x40 -or $offset -gt $stream.Length - 6) { throw "Invalid PE header: $Path" }
        $stream.Position = $offset
        if ($reader.ReadUInt32() -ne 0x4550) { throw "Invalid PE signature: $Path" }
        switch ($reader.ReadUInt16()) {
            0x8664 { return 'x64' }
            0xaa64 { return 'arm64' }
            default { throw "Unsupported PE architecture in $Path (only x64 and ARM64 are supported)." }
        }
    } finally { $reader.Dispose() }
}

function Write-Utf8 {
    param([string]$Path, [string]$Value)
    [System.IO.File]::WriteAllText($Path, $Value, [System.Text.UTF8Encoding]::new($false))
}

function Write-PackageManifest {
    param([string]$Template, [string]$Destination, [string]$Architecture,
          [string]$Version, [string]$Publisher)
    if ($Architecture -notin @('x64', 'arm64')) { throw 'Unsupported manifest architecture.' }
    if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'MSIX needs a four-part numeric version.' }
    $parts = $Version.Split('.') | ForEach-Object { [int]$_ }
    if (@($parts | Where-Object { $_ -gt 65535 }).Count -gt 0) { throw 'MSIX version components must be <= 65535.' }
    $minVersion = if ($Architecture -eq 'arm64') { '10.0.22000.0' } else { '10.0.19045.0' }
    $xml = (Get-Content -LiteralPath $Template -Raw).Replace('@ARCH@', $Architecture).
        Replace('@VERSION@', $Version).Replace('@MIN_VERSION@', $minVersion).
        Replace('@PUBLISHER@', [System.Security.SecurityElement]::Escape($Publisher))
    if ($xml -match '@[A-Z_]+@') { throw 'Manifest has unresolved placeholders.' }
    $null = [xml]$xml
    Write-Utf8 $Destination $xml
}

function Get-SigningCertificate {
    param([string]$Thumbprint, [string]$Publisher)
    if (-not $Thumbprint) { return $null }
    if ($Thumbprint -notmatch '^[A-Fa-f0-9]{40}$') { throw 'Certificate thumbprint must be 40 hexadecimal characters.' }
    $cert = Get-Item "Cert:/CurrentUser/My/$Thumbprint"
    if (-not $cert.HasPrivateKey -or $cert.NotAfter -le (Get-Date) -or $cert.NotBefore -gt (Get-Date)) {
        throw 'Signing certificate must be current and have an accessible private key.'
    }
    if ($cert.Subject -cne $Publisher) { throw 'Certificate subject must exactly match the MSIX Publisher.' }
    if ('1.3.6.1.5.5.7.3.3' -notin @($cert.EnhancedKeyUsageList | ForEach-Object { $_.ObjectId })) {
        throw 'Signing certificate must have the Code Signing EKU.'
    }
    return $cert
}

function Sign-Artifact {
    param([string]$Path, [string]$Thumbprint, [string]$TimestampUrl)
    $tool = Find-SdkTool 'signtool.exe'
    $arguments = @('sign', '/fd', 'SHA256', '/sha1', $Thumbprint, '/s', 'My')
    if ($TimestampUrl) {
        if ($TimestampUrl -notmatch '^https://') { throw 'Use an HTTPS RFC 3161 timestamp server.' }
        $arguments += @('/tr', $TimestampUrl, '/td', 'SHA256')
    }
    Invoke-Native $tool ($arguments + @($Path))
    # Trust is deliberately not installed by the build. Verification requiring trust is a separate step.
}

function Write-Checksum {
    param([string]$Path)
    $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Utf8 "$Path.sha256" "$hash  $([System.IO.Path]::GetFileName($Path))`n"
}

function Remove-BuildDirectory {
    param([string]$Root, [string]$Path)
    $parent = [IO.Path]::GetFullPath((Join-Path $Root 'build/windows')) + [IO.Path]::DirectorySeparatorChar
    $absolute = [IO.Path]::GetFullPath($Path)
    if (-not $absolute.StartsWith($parent, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Refusing to remove a directory outside build/windows.'
    }
    if (Test-Path -LiteralPath $absolute) { Remove-Item -LiteralPath $absolute -Recurse -Force }
}

function Assert-ReleaseSource {
    param([string]$Root, [string]$Version)
    $status = @(& git -C $Root status --porcelain --untracked-files=normal)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect the source checkout.' }
    if ($status.Count) { throw 'Release builds require a clean source checkout. Commit source changes first.' }
    if (Test-Path -LiteralPath "$Root/vendor/zed/.git") {
        $untracked = @(& git -C "$Root/vendor/zed" ls-files --others --exclude-standard)
        if ($LASTEXITCODE -ne 0 -or $untracked.Count) { throw 'Release vendor checkout contains untracked source files.' }
    }
    $tags = @(& git -C $Root tag --points-at HEAD)
    if ($LASTEXITCODE -ne 0 -or "windows-v$Version" -cnotin $tags) {
        throw "Release builds require tag windows-v$Version at HEAD."
    }
}

function Assert-ProductionSignature {
    param([string]$Path, [string]$Thumbprint)
    Invoke-Native (Find-SdkTool 'signtool.exe') @('verify', '/pa', '/all', $Path)
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate -or
        $signature.SignerCertificate.Thumbprint -ne $Thumbprint -or
        $signature.SignerCertificate.Subject -eq $signature.SignerCertificate.Issuer -or
        -not $signature.TimeStamperCertificate) {
        throw "A trusted, timestamped publisher signature is required: $Path"
    }
}

function Write-ReleaseManifest {
    param([object]$Info, [string[]]$Artifacts, [string]$Destination, [string]$CompilerVersion)
    $entries = @($Artifacts | Sort-Object | ForEach-Object {
        $file = Get-Item -LiteralPath $_
        $signature = if ($IsWindows -and $file.Extension -in @('.exe', '.msix')) {
            Get-AuthenticodeSignature -LiteralPath $file.FullName
        } else { $null }
        [ordered]@{
            file = $file.Name
            bytes = $file.Length
            sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            signature = if ($signature) {
                [ordered]@{
                    status = [string]$signature.Status
                    publisher = if ($signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { $null }
                    thumbprint = if ($signature.SignerCertificate) { $signature.SignerCertificate.Thumbprint } else { $null }
                    timestamped = [bool]$signature.TimeStamperCertificate
                }
            } else { $null }
        }
    })
    $manifest = [ordered]@{
        schema_version = 1
        build = $Info
        packaging = @{ inno_setup_version = $CompilerVersion }
        artifacts = $entries
    }
    Write-Utf8 $Destination (($manifest | ConvertTo-Json -Depth 10) + "`n")
}
