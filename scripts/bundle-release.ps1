$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
Push-Location -LiteralPath $taskRoot
$taskConfigPath = Join-Path $taskRoot 'src-tauri/tauri.conf.json'
$taskManifestPath = Join-Path $taskRoot 'src-tauri/Cargo.toml'
$taskOriginalConfig = [IO.File]::ReadAllBytes($taskConfigPath)
$taskOriginalManifest = [IO.File]::ReadAllBytes($taskManifestPath)
$taskConfigTime = [IO.File]::GetLastWriteTimeUtc($taskConfigPath)
$taskManifestTime = [IO.File]::GetLastWriteTimeUtc($taskManifestPath)
try {
    $taskVersion = (Get-Content -LiteralPath package.json -Raw | ConvertFrom-Json).version
    if ($taskVersion -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid version' }
    $taskRelease = Join-Path $taskRoot ('release/v' + $taskVersion)
    New-Item -ItemType Directory -Force -Path (Join-Path $taskRelease 'logs') | Out-Null
    $taskPortable = Join-Path $taskRelease ('ema-desktop-' + $taskVersion)
    $taskRecord = Get-Content -LiteralPath (Join-Path $taskPortable 'build-info.json') -Raw | ConvertFrom-Json
    $taskCommit = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $taskCommit -ne $taskRecord.SourceHead -or (& git status --porcelain=v1)) { throw 'Release source must be clean and match the prepared package' }
    $taskExe = Join-Path $taskRoot 'src-tauri/target/release/traybuddy.exe'
    $taskRecord | Add-Member -NotePropertyName PortableBinarySha256 -NotePropertyValue $taskRecord.BinarySha256
    $taskRecord | Add-Member -NotePropertyName BundleType -NotePropertyValue 'nsis'
    # CLI overlays merge resource maps and retain unwanted default paths.
    # Temporarily replace the map itself, then restore the exact source bytes.
    $taskConfig = [Text.Encoding]::UTF8.GetString($taskOriginalConfig) | ConvertFrom-Json
    $taskOverride = Get-Content -LiteralPath (Join-Path $taskRelease ('tauri.' + $taskVersion + '.current-user.json')) -Raw | ConvertFrom-Json
    $taskConfig.bundle.resources = $taskOverride.bundle.resources
    $taskConfig.bundle.windows.nsis.installMode = 'currentUser'
    $taskConfig | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $taskConfigPath -Encoding utf8NoBOM
    $taskStable = $false
    # Tauri may stamp the bundle type on the first invocation. Rebundle once so
    # the shipped provenance records the actual installer payload hash.
    foreach ($taskAttempt in 1..2) {
        $taskRecord.BinarySha256 = (Get-FileHash -LiteralPath $taskExe -Algorithm SHA256).Hash.ToLowerInvariant()
        $taskRecord | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $taskRelease ('ema-desktop-' + $taskVersion + '-installer-build-info.json')) -Encoding utf8NoBOM
        & node node_modules/@tauri-apps/cli/tauri.js bundle --bundles nsis --features custom-protocol --ci *> (Join-Path $taskRelease ('logs/bundle-' + $taskVersion + '-' + $taskAttempt + '.log'))
        if ($LASTEXITCODE -ne 0) {
            Get-Content -LiteralPath (Join-Path $taskRelease ('logs/bundle-' + $taskVersion + '-' + $taskAttempt + '.log')) -Tail 35
            throw 'NSIS bundling failed'
        }
        if ((Get-FileHash -LiteralPath $taskExe -Algorithm SHA256).Hash.ToLowerInvariant() -eq $taskRecord.BinarySha256) {
            $taskStable = $true
            break
        }
    }
    if (-not $taskStable) { throw 'Installer payload changed unexpectedly' }
    $taskInstaller = 'traybuddy_' + $taskVersion + '_x64-setup.exe'
    Copy-Item -LiteralPath (Join-Path 'src-tauri/target/release/bundle/nsis' $taskInstaller) -Destination (Join-Path $taskRelease ('traybuddy_' + $taskVersion + '_x64-setup-user.exe'))
    Write-Output ('Built current-user installer v' + $taskVersion)
} finally {
    [IO.File]::WriteAllBytes($taskConfigPath, $taskOriginalConfig)
    [IO.File]::SetLastWriteTimeUtc($taskConfigPath, $taskConfigTime)
    # Tauri also normalizes Cargo.toml line endings while inspecting it.
    [IO.File]::WriteAllBytes($taskManifestPath, $taskOriginalManifest)
    [IO.File]::SetLastWriteTimeUtc($taskManifestPath, $taskManifestTime)
    Pop-Location
}
