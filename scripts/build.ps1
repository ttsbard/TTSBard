# build.ps1 — сборка TTSBard (Tauri) под Windows.
#
# Использование:
#   .\scripts\build.ps1                  # релиз по умолчанию
#   .\scripts\build.ps1 -Mode debug      # debug-сборка (без инсталляторов)
#   .\scripts\build.ps1 -Mode release    # полная релиз-сборка (exe + nsis/msi)
#   .\scripts\build.ps1 -Clean           # очистить target/ и dist/ перед сборкой
#
# Локальная конфигурация (опционально):
#   .\scripts\build.ps1 -CargoTargetDir D:\custom-target
#   .\scripts\build.ps1 -RustBinDir %USERPROFILE%\.cargo\bin
#   .\scripts\build.ps1 -ConfigFile my-config.psd1
#
# Конфигурация загружается из scripts/build.local.psd1 (игнорируется Git).
# Пример: см. scripts/build.local.example.psd1.
#
# Обёртки для двойного клика: build-debug.bat, build-release.bat.
#
# Артефакты:
#   exe:      <cargo-target>\<debug|release>\ttsbard.exe
#   bundles:  <cargo-target>\release\bundle\{nsis,msi}\  (только release)

[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string]$Mode = 'release',

    [switch]$Clean,

    [string]$CargoTargetDir,

    [string]$RustBinDir,

    [string]$ConfigFile
)

$ErrorActionPreference = 'Stop'

# --- Цветной вывод -----------------------------------------------------------
function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-Ok($msg)   { Write-Host "    $msg" -ForegroundColor Green }
function Write-WarnLine($msg) { Write-Host "    ! $msg" -ForegroundColor Yellow }
function Write-Err($msg)  { Write-Host "    X $msg" -ForegroundColor Red }

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

# --- Вспомогательные функции для путей ----------------------------------------

function Expand-EnvRefs([string]$path) {
    return [regex]::Replace($path, '%([^%]+)%', {
        param($m)
        $envVal = [Environment]::GetEnvironmentVariable($m.Groups[1].Value)
        if ($null -ne $envVal -and $envVal -ne '') { return $envVal }
        return $m.Value
    })
}

function Resolve-Absolute([string]$path) {
    if ([System.IO.Path]::IsPathRooted($path)) {
        return [System.IO.Path]::GetFullPath($path)
    }
    return [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($repoRoot, $path))
}

function Test-IsAncestorOf([string]$candidate, [string]$child) {
    $candidate = $candidate.TrimEnd('\') + '\'
    $child = $child.TrimEnd('\') + '\'
    return $child.StartsWith($candidate, [StringComparison]::OrdinalIgnoreCase) -and
           $candidate.Length -lt $child.Length
}

function Test-IsUnsafeCleanTarget([string]$target) {
    $srcTauri = [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($repoRoot, 'src-tauri'))
    if ($target -eq $repoRoot) { return $true }
    if ($target -eq $env:USERPROFILE) { return $true }
    if ($target -eq $srcTauri) { return $true }
    if (Test-IsAncestorOf $target $repoRoot) { return $true }
    if (Test-IsAncestorOf $target $env:USERPROFILE) { return $true }
    $pathRoot = [System.IO.Path]::GetPathRoot($target)
    if ($target.TrimEnd('\') -eq $pathRoot.TrimEnd('\')) { return $true }
    return $false
}

function Import-BuildConfig([string]$path) {
    $importDataFile = Get-Command Import-PowerShellDataFile -ErrorAction SilentlyContinue
    if ($null -ne $importDataFile) {
        return Import-PowerShellDataFile -LiteralPath $path
    }

    # Windows PowerShell installations without Import-PowerShellDataFile occur
    # in the wild. The local config deliberately supports only two string keys,
    # so parse that tiny data format instead of evaluating a .psd1 as code.
    $result = @{}
    $lineNumber = 0
    foreach ($line in Get-Content -LiteralPath $path) {
        $lineNumber++
        $trimmed = $line.Trim()
        if ([string]::IsNullOrEmpty($trimmed) -or $trimmed.StartsWith('#') -or
            $trimmed -eq '@{' -or $trimmed -eq '}') {
            continue
        }

        $match = [regex]::Match(
            $trimmed,
            '^(CargoTargetDir|RustBinDir)\s*=\s*(?:''([^'']*)''|"([^"]*)"|\$null)\s*(?:#.*)?$'
        )
        if (-not $match.Success) {
            throw "Unsupported build config syntax at ${path}:$lineNumber. Use CargoTargetDir or RustBinDir with a quoted string or `$null."
        }

        $key = $match.Groups[1].Value
        if ($result.ContainsKey($key)) {
            throw "Duplicate build config key '$key' at ${path}:$lineNumber."
        }
        $result[$key] = if ($trimmed -match '=\s*\$null') {
            $null
        } elseif ($match.Groups[2].Success) {
            $match.Groups[2].Value
        } else {
            $match.Groups[3].Value
        }
    }
    return $result
}

# --- Загрузка конфигурации ----------------------------------------------------

$configFilePath = if ($ConfigFile) {
    $resolved = $ConfigFile
    if (-not [System.IO.Path]::IsPathRooted($resolved)) {
        $resolved = [System.IO.Path]::Combine($repoRoot, $resolved)
    }
    [System.IO.Path]::GetFullPath($resolved)
} else {
    [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($repoRoot, 'scripts', 'build.local.psd1'))
}

$configData = $null
$configLoaded = $false

if (Test-Path $configFilePath -PathType Leaf) {
    $configData = Import-BuildConfig $configFilePath
    $configLoaded = $true

    if ($null -eq $configData) {
        $configData = @{}
    }

    foreach ($key in $configData.Keys) {
        if ($key -ne 'CargoTargetDir' -and $key -ne 'RustBinDir') {
            Write-Err "Unknown config key '$key' in $configFilePath. Allowed keys: CargoTargetDir, RustBinDir."
            exit 1
        }
        $val = $configData[$key]
        if ($null -ne $val -and $val -isnot [string]) {
            Write-Err "Config key '$key' in $configFilePath must be a string or `$null, got $($val.GetType().Name)."
            exit 1
        }
    }
} elseif ($ConfigFile) {
    Write-Err "Config file not found: $configFilePath (explicitly supplied via -ConfigFile)"
    exit 1
}

# --- Разрешение CargoTargetDir -----------------------------------------------

$defaultTarget = Join-Path $repoRoot 'src-tauri\target'
$cargoTargetSource = 'default'

if ($PSBoundParameters.ContainsKey('CargoTargetDir')) {
    if ([string]::IsNullOrEmpty($CargoTargetDir)) {
        Write-Err 'CargoTargetDir parameter is empty.'
        exit 1
    }
    $targetDir = Resolve-Absolute (Expand-EnvRefs $CargoTargetDir)
    $cargoTargetSource = 'parameter'
} elseif (Test-Path 'Env:CARGO_TARGET_DIR') {
    $envVal = $env:CARGO_TARGET_DIR
    if ([string]::IsNullOrEmpty($envVal)) {
        Write-Err 'CARGO_TARGET_DIR environment variable is empty.'
        exit 1
    }
    $targetDir = Resolve-Absolute (Expand-EnvRefs $envVal)
    $cargoTargetSource = 'environment'
} elseif ($configData -and $configData.ContainsKey('CargoTargetDir') -and $null -ne $configData['CargoTargetDir']) {
    $cfgVal = $configData['CargoTargetDir']
    if ([string]::IsNullOrEmpty($cfgVal)) {
        Write-Err 'CargoTargetDir in config file is empty.'
        exit 1
    }
    $targetDir = Resolve-Absolute (Expand-EnvRefs $cfgVal)
    $cargoTargetSource = 'config file'
} else {
    $targetDir = Resolve-Absolute $defaultTarget
    $cargoTargetSource = 'default'
}

$env:CARGO_TARGET_DIR = $targetDir

# --- Разрешение RustBinDir ---------------------------------------------------

$rustBinDir = $null

if ($PSBoundParameters.ContainsKey('RustBinDir')) {
    if ([string]::IsNullOrEmpty($RustBinDir)) {
        Write-Err 'RustBinDir parameter is empty.'
        exit 1
    }
    $rustBinDir = Resolve-Absolute (Expand-EnvRefs $RustBinDir)
} elseif (Test-Path 'Env:TTSBARD_RUST_BIN_DIR') {
    $envVal = $env:TTSBARD_RUST_BIN_DIR
    if ([string]::IsNullOrEmpty($envVal)) {
        Write-Err 'TTSBARD_RUST_BIN_DIR environment variable is empty.'
        exit 1
    }
    $rustBinDir = Resolve-Absolute (Expand-EnvRefs $envVal)
} elseif ($configData -and $configData.ContainsKey('RustBinDir') -and $null -ne $configData['RustBinDir']) {
    $cfgVal = $configData['RustBinDir']
    if ([string]::IsNullOrEmpty($cfgVal)) {
        Write-Err 'RustBinDir in config file is empty.'
        exit 1
    }
    $rustBinDir = Resolve-Absolute (Expand-EnvRefs $cfgVal)
}

if ($rustBinDir) {
    if (-not (Test-Path $rustBinDir -PathType Container)) {
        Write-Err "RustBinDir does not exist or is not a directory: $rustBinDir"
        exit 1
    }
    $currentPath = $env:PATH
    $pathSeparator = ';'
    $entries = $currentPath -split $pathSeparator
    $rustBinNormalized = $rustBinDir.TrimEnd('\')
    $otherEntries = @($entries | Where-Object {
        $_ -and $_.TrimEnd('\') -ne $rustBinNormalized
    })
    $env:PATH = (@($rustBinDir) + $otherEntries) -join $pathSeparator
}

# --- Информация о конфигурации -----------------------------------------------

$modeLabel = $Mode
if ($Clean) { $modeLabel = "$Mode (+clean)" }
Write-Step "TTSBard build — mode: $modeLabel"
Write-Step "Repo: $repoRoot"
if ($configLoaded) { Write-Step "Config file: $configFilePath" }
Write-Step "Cargo target: $targetDir"
if ($rustBinDir) { Write-Step "Rust bin: $rustBinDir" }

# --- Проверка окружения ------------------------------------------------------
Write-Step "Checking toolchain..."

foreach ($cmd in @('node', 'npm', 'cargo', 'cmake')) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        Write-Err "$cmd not found in PATH. Установите требуемый инструмент и повторите сборку."
        exit 1
    }
}
try {
    $nodeVer = (node -v)
    $npmVer  = (npm -v)
    $rustcVer = (rustc --version)
    $cmakeVer = (cmake --version | Select-Object -First 1)
} catch {
    Write-Err "Не удалось определить версии toolchain: $_"
    exit 1
}
Write-Ok "node $nodeVer, npm $npmVer"
Write-Ok $rustcVer
Write-Ok $cmakeVer

# espeak-rs-sys v0.2.0 hardcodes Release library paths. Keep its C library on the
# default dynamic CRT (Release profile). Cargo.toml disables debug assertions only
# for that upstream package, preventing its build script from linking `msvcrtd`.
# See docs/development/windows-debug-crt.md.
if ($Mode -eq 'debug') {
    $env:ESPEAK_LIB_PROFILE = 'Release'
    Remove-Item Env:ESPEAK_STATIC_CRT -ErrorAction SilentlyContinue
    Write-Ok 'espeak-ng CMake profile: Release + compatible dynamic CRT'
}

# --- Проверка libclang (нужен для espeak-rs-sys / bindgen) --------------------
Write-Step "Checking libclang for bindgen..."
. "$PSScriptRoot\libclang-bootstrap.ps1"
$null = Initialize-LibClangPath

# --- Вспомогательные константы -----------------------------------------------
$defaultCanonical = [System.IO.Path]::GetFullPath((Join-Path $repoRoot 'src-tauri\target'))
$isExternalTarget = ($targetDir.TrimEnd('\') -ne $defaultCanonical.TrimEnd('\'))
$distDir   = Join-Path $repoRoot 'dist'
$espeakDstDir = Join-Path $repoRoot 'src-tauri\resources\espeak-ng-data'

# --- Опциональная очистка ----------------------------------------------------

if ($Clean) {
    if (Test-IsUnsafeCleanTarget $targetDir) {
        Write-Err "Refusing -Clean: target dir ($targetDir) is a filesystem root, a protected location (repository, user profile, or src-tauri), or an ancestor of such a location. Set a project-specific target directory instead."
        exit 1
    }

    if ($isExternalTarget -and (Test-Path $targetDir)) {
        $markerFile = Join-Path $targetDir '.ttsbard-build-target'
        if (-not (Test-Path $markerFile -PathType Leaf)) {
            Write-Err "Refusing -Clean: external Cargo target ($targetDir) is missing the marker file '.ttsbard-build-target'. Run a non-clean build first, or create the marker manually if this target was previously initialized for this project."
            exit 1
        }
    }

    Write-Step "Cleaning build artifacts..."
    foreach ($d in @($targetDir, $distDir, $espeakDstDir)) {
        if (Test-Path $d) {
            Remove-Item -Recurse -Force $d
            Write-Ok "removed $d"
        }
    }

    if ($isExternalTarget) {
        New-Item -ItemType Directory -Force $targetDir | Out-Null
        $markerFile = Join-Path $targetDir '.ttsbard-build-target'
        New-Item -ItemType File -Force $markerFile | Out-Null
    }
}

if ($isExternalTarget) {
    if (-not (Test-Path $targetDir)) {
        New-Item -ItemType Directory -Force $targetDir | Out-Null
    }
    $markerFile = Join-Path $targetDir '.ttsbard-build-target'
    if (-not (Test-Path $markerFile -PathType Leaf)) {
        New-Item -ItemType File -Force $markerFile | Out-Null
    }
}

# --- Установка npm-зависимостей (если нужно) ---------------------------------
Write-Step "Checking npm dependencies..."
$nodeModules = Join-Path $repoRoot 'node_modules'
if (-not (Test-Path $nodeModules)) {
    Write-Step "Installing npm dependencies..."
    npm install
    if ($LASTEXITCODE -ne 0) { Write-Err "npm install failed"; exit 1 }
    Write-Ok "npm install done"
} else {
    Write-Ok "node_modules exists, skipping install"
}

# --- Подготовка espeak-ng-data в ресурсы (ДО tauri build) ---------------------

function Find-RegistrySource {
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    $registryPattern = Join-Path $cargoHome 'registry\src\*\espeak-rs-sys-*'
    $candidates = @(Get-ChildItem -Path $registryPattern -Directory -ErrorAction SilentlyContinue |
        ForEach-Object { Join-Path $_.FullName 'espeak-ng\espeak-ng-data' } |
        Where-Object { Test-Path $_ -PathType Container } |
        Sort-Object { (Get-Item $_).LastWriteTime } -Descending)
    if ($candidates) { return $candidates[0] }
    return $null
}

function Find-CompiledOutput {
    $targetProfile = if ($Mode -eq 'debug') { 'debug' } else { 'release' }
    $candidate = Get-ChildItem -Path "$targetDir\$targetProfile\build\espeak-rs-sys-*\out\share\espeak-ng-data" -Directory -ErrorAction SilentlyContinue |
        Sort-Object -Property LastWriteTime -Descending |
        Select-Object -First 1
    if ($candidate) { return $candidate.FullName }
    return $null
}

function Test-ValidEspeakData($path) {
    return ($path -and
        (Test-Path (Join-Path $path 'voices') -PathType Container) -and
        (Test-Path (Join-Path $path 'lang') -PathType Container) -and
        (Test-Path (Join-Path $path 'en_dict') -PathType Leaf))
}

function Invoke-BootstrapAndCompile {
    # Step 1 — bootstrap from Cargo registry (has voices/, NOT en_dict)
    Write-Step "Bootstrapping espeak-ng-data from Cargo registry..."
    $regSrc = Find-RegistrySource
    if (-not $regSrc) {
        Write-Step "Registry source not found — running cargo fetch in src-tauri..."
        Push-Location (Join-Path $repoRoot 'src-tauri')
        try {
            cargo fetch
            if ($LASTEXITCODE -ne 0) { Write-Err "cargo fetch failed"; exit 1 }
        } finally { Pop-Location }
        Write-Ok "cargo fetch done"
        $regSrc = Find-RegistrySource
    }
    if (-not $regSrc) {
        # Recent Cargo versions can download crate archives without unpacking
        # them into registry/src. Building the crate both unpacks it and
        # produces the complete resource directory we ultimately need.
        Write-WarnLine "Cargo registry source unavailable; building espeak-rs with dictionaries."
        $cargoArgs = @('build', '-p', 'espeak-rs')
        if ($Mode -eq 'release') { $cargoArgs += '--release' }
        Push-Location (Join-Path $repoRoot 'src-tauri')
        try {
            & 'cargo' $cargoArgs
            if ($LASTEXITCODE -ne 0) { Write-Err "cargo build -p espeak-rs failed"; exit 1 }
        } finally { Pop-Location }

        $compiled = Find-CompiledOutput
        if (Test-ValidEspeakData $compiled) {
            if (Test-Path $espeakDstDir) { Remove-Item -Recurse -Force $espeakDstDir }
            Copy-Item -Recurse -Force $compiled $espeakDstDir
            $fileCount = (Get-ChildItem -Recurse -File -Path $espeakDstDir | Measure-Object).Count
            Write-Ok "installed compiled espeak-ng-data ($fileCount files) with en_dict"
            return
        }

        Write-Err "espeak-ng-data not found in Cargo registry or compiled build output."
        exit 1
    }
    Write-Ok "registry source: $regSrc"

    if (Test-Path $espeakDstDir) { Remove-Item -Recurse -Force $espeakDstDir }
    Copy-Item -Recurse -Force $regSrc $espeakDstDir

    if (-not (Test-Path (Join-Path $espeakDstDir 'voices'))) {
        Write-Err "Bootstrap copy missing voices/ — aborting."
        exit 1
    }
    Write-Ok "bootstrap espeak-ng-data with voices/ from registry"

    # Step 2 — espeak-rs enables dictionary compilation in espeak-rs-sys.
    Write-Step "Building espeak-rs to generate dictionaries..."
    $cargoArgs = @('build', '-p', 'espeak-rs')
    if ($Mode -eq 'release') { $cargoArgs += '--release' }
    Push-Location (Join-Path $repoRoot 'src-tauri')
    try {
        & 'cargo' $cargoArgs
        if ($LASTEXITCODE -ne 0) { Write-Err "cargo build -p espeak-rs failed"; exit 1 }
    } finally { Pop-Location }
    Write-Ok "espeak-rs compiled"

    # Step 3 — find compiled output, replace bootstrap, validate voices/ + en_dict
    Write-Step "Installing compiled espeak-ng-data with generated dictionaries..."
    $compiled = Find-CompiledOutput
    if (-not $compiled) {
        Write-Err "Compiled espeak-ng-data not found in target build output."
        exit 1
    }
    Write-Ok "compiled output: $compiled"

    Remove-Item -Recurse -Force $espeakDstDir
    Copy-Item -Recurse -Force $compiled $espeakDstDir

    if (-not (Test-ValidEspeakData $espeakDstDir)) {
        Write-Err "Compiled espeak-ng-data must contain voices/, lang/, and en_dict."
        exit 1
    }
    $fileCount = (Get-ChildItem -Recurse -File -Path $espeakDstDir | Measure-Object).Count
    Write-Ok "installed compiled espeak-ng-data ($fileCount files) with en_dict"
}

# Decide which path to take:
#   - If valid compiled output exists for this profile → reuse it directly
#   - Otherwise → bootstrap from registry, compile, replace
Write-Step "Preparing espeak-ng-data..."

$currentCompiled = Find-CompiledOutput
if ($currentCompiled -and (Test-ValidEspeakData $currentCompiled)) {
    Write-Ok "found valid compiled espeak-ng-data for '$Mode' profile: $currentCompiled"
    if (Test-Path $espeakDstDir) { Remove-Item -Recurse -Force $espeakDstDir }
    Copy-Item -Recurse -Force $currentCompiled $espeakDstDir
    $fileCount = (Get-ChildItem -Recurse -File -Path $espeakDstDir | Measure-Object).Count
    Write-Ok "copied compiled espeak-ng-data ($fileCount files) with en_dict"
} else {
    if ($currentCompiled) {
        Write-WarnLine "compiled output is incomplete (expected voices/, lang/, and en_dict)"
    } else {
        Write-WarnLine "no compiled espeak-ng-data for '$Mode' profile"
    }
    Invoke-BootstrapAndCompile
}

# --- Удаление устаревших внешних файлов словаря из ресурсов сборки ------------
# До ROADMAP-110 каталог resources/dict целиком попадал в bundle. Остатки ru.aff
# и ru.dic от прежних сборок лежат в выбранном profile и при повторной сборке
# снова упаковались бы. Удаляем только эти два файла из выходного каталога
# ресурсов, не трогая исходники src-tauri/resources и пользовательские каталоги.
function Remove-StaleExternalDict([string]$ProfileResourcesDir, [string]$BuildTargetDir) {
    $buildTarget = [System.IO.Path]::GetFullPath($BuildTargetDir).TrimEnd('\')
    $profileRes = [System.IO.Path]::GetFullPath($ProfileResourcesDir).TrimEnd('\')
    $dictOutputDir = [System.IO.Path]::GetFullPath((Join-Path $profileRes 'dict'))

    if (-not (Test-IsAncestorOf $buildTarget $profileRes)) {
        throw "Refusing to remove stale dictionary files: profile resources dir '$profileRes' is outside build target '$buildTarget'."
    }

    foreach ($fileName in @('ru.aff', 'ru.dic')) {
        $filePath = [System.IO.Path]::GetFullPath((Join-Path $dictOutputDir $fileName))
        if (-not (Test-IsAncestorOf $dictOutputDir $filePath)) {
            throw "Refusing to remove stale dictionary file outside dict output dir: '$filePath'."
        }
        if (-not (Test-Path -LiteralPath $filePath -PathType Leaf)) { continue }

        $reparse = $false
        $cursor = $filePath
        while ($cursor) {
            if (Test-Path -LiteralPath $cursor) {
                $entry = Get-Item -LiteralPath $cursor -Force
                if (($entry.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                    $reparse = $true
                    break
                }
            }
            $parent = Split-Path -Parent $cursor
            if (-not $parent -or $parent -eq $cursor) { break }
            $cursor = $parent
        }
        if ($reparse) {
            throw "Refusing to remove stale dictionary file: reparse point detected on path '$filePath'."
        }

        Remove-Item -LiteralPath $filePath -Force
        Write-Ok "removed stale external dictionary file: $filePath"
    }
}

$targetProfile = if ($Mode -eq 'debug') { 'debug' } else { 'release' }
$profileDir = Join-Path $targetDir $targetProfile
# ort-sys copies DLLs into these directories during its build script.
New-Item -ItemType Directory -Force -Path (Join-Path $profileDir 'examples'), (Join-Path $profileDir 'deps') | Out-Null
$profileResourcesDir = [System.IO.Path]::GetFullPath((Join-Path $targetDir "$targetProfile\resources"))
Remove-StaleExternalDict $profileResourcesDir $targetDir

# --- Сборка ------------------------------------------------------------------
$buildStart = Get-Date

if ($Mode -eq 'debug') {
    Write-Step "Building (tauri build --debug --no-bundle)..."
    # --debug: бэкенд в debug-профайле, фронтенд-бандл, готовый exe, БЕЗ инсталляторов.
    npm run tauri -- build --debug --no-bundle
} else {
    Write-Step "Building (tauri build, release)..."
    npm run tauri -- build
}

if ($LASTEXITCODE -ne 0) {
    Write-Err "tauri build failed (exit $LASTEXITCODE)"
    exit $LASTEXITCODE
}

$elapsed = (Get-Date) - $buildStart
Write-Ok ("build done in {0:mm\:ss}" -f $elapsed)

# --- Отчёт об артефактах -----------------------------------------------------
Write-Step "Artifacts:"

$targetProfile = if ($Mode -eq 'debug') { 'debug' } else { 'release' }
$exePath = Join-Path $targetDir "$targetProfile\ttsbard.exe"
if (Test-Path $exePath) {
    $sizeMb = [math]::Round((Get-Item $exePath).Length / 1MB, 1)
    Write-Ok "EXE  : $exePath ($sizeMb MB)"
} else {
    Write-WarnLine "EXE not found at expected path: $exePath"
}

if ($Mode -eq 'release') {
    $bundleDir = Join-Path $targetDir 'release\bundle'
    if (Test-Path $bundleDir) {
        $installers = Get-ChildItem -Recurse -Path $bundleDir -Include '*.exe','*.msi' -ErrorAction SilentlyContinue
        if ($installers) {
            foreach ($inst in $installers) {
                $sizeMb = [math]::Round($inst.Length / 1MB, 1)
                Write-Ok ("BUNDLE: {0} ({1} MB)" -f $inst.FullName, $sizeMb)
            }
        } else {
            Write-WarnLine "Bundle dir exists but no .exe/.msi installers found"
        }
    } else {
        Write-WarnLine "No bundle directory (installers) — check tauri.conf.json bundle config"
    }
}

Write-Host ""
Write-Host "BUILD SUCCEEDED" -ForegroundColor Green
