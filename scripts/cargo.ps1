# cargo.ps1 — convenience wrapper that sets LIBCLANG_PATH before invoking Cargo.
# It also applies RustBinDir from TTSBARD_RUST_BIN_DIR or build.local.psd1.
#
# Usage:  .\scripts\cargo.ps1 test --manifest-path src-tauri/Cargo.toml
#         .\scripts\cargo.ps1 check --manifest-path src-tauri/Cargo.toml
#         .\scripts\cargo.ps1 clippy --manifest-path src-tauri/Cargo.toml
#         .\scripts\cargo.ps1 --version
#
# All arguments are forwarded to cargo; exit code is preserved.

$ErrorActionPreference = 'Stop'

. "$PSScriptRoot\libclang-bootstrap.ps1"

$repoRoot = Split-Path -Parent $PSScriptRoot
$rustBinDir = $null

if ($env:TTSBARD_RUST_BIN_DIR) {
    $rustBinDir = $env:TTSBARD_RUST_BIN_DIR
} else {
    $localConfigPath = Join-Path $PSScriptRoot 'build.local.psd1'
    if (Test-Path $localConfigPath -PathType Leaf) {
        $localConfig = Import-PowerShellDataFile $localConfigPath
        if ($localConfig -and $localConfig.ContainsKey('RustBinDir')) {
            $rustBinDir = $localConfig['RustBinDir']
        }
    }
}

if ($rustBinDir) {
    $rustBinDir = [regex]::Replace($rustBinDir, '%([^%]+)%', {
        param($match)
        $value = [Environment]::GetEnvironmentVariable($match.Groups[1].Value)
        if ($null -ne $value -and $value -ne '') { return $value }
        return $match.Value
    })
    if (-not [System.IO.Path]::IsPathRooted($rustBinDir)) {
        $rustBinDir = Join-Path $repoRoot $rustBinDir
    }
    $rustBinDir = [System.IO.Path]::GetFullPath($rustBinDir)
    if (-not (Test-Path $rustBinDir -PathType Container)) {
        throw "RustBinDir does not exist or is not a directory: $rustBinDir"
    }

    $rustBinNormalized = $rustBinDir.TrimEnd('\')
    $otherEntries = @(($env:PATH -split ';') | Where-Object {
        $_ -and $_.TrimEnd('\') -ne $rustBinNormalized
    })
    $env:PATH = (@($rustBinDir) + $otherEntries) -join ';'
}

$null = Initialize-LibClangPath

# Cargo writes progress and lock messages to stderr. Under Windows PowerShell 5.1
# a caller redirecting them (`2>&1`) turns each line into an ErrorRecord, and the
# 'Stop' preference above would abort the wrapper with NativeCommandError even
# when Cargo succeeds. Keep initialization strict, then let stderr flow through
# and preserve the real exit code.
$ErrorActionPreference = 'Continue'

& cargo @args
exit $LASTEXITCODE
