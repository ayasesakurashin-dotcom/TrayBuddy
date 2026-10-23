$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
Push-Location -LiteralPath $taskRoot
try {
    New-Item -ItemType Directory -Force -Path release/logs | Out-Null
    $taskVersion = (Get-Content -LiteralPath package.json -Raw | ConvertFrom-Json).version
    if ($taskVersion -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid version' }
    $taskPortable = Join-Path $taskRoot ('release/ema-desktop-' + $taskVersion)
    $taskRecord = Get-Content -LiteralPath (Join-Path $taskPortable 'build-info.json') -Raw | ConvertFrom-Json
    $taskCommit = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $taskCommit -ne $taskRecord.SourceHead -or (& git status --porcelain=v1)) { throw 'Release source must be clean and match the prepared package' }
    $taskExe = Join-Path $taskRoot 'src-tauri/target/release/traybuddy.exe'
    $taskRecord | Add-Member -NotePropertyName PortableBinarySha256 -NotePropertyValue $taskRecord.BinarySha256
    $taskRecord | Add-Member -NotePropertyName BundleType -NotePropertyValue 'nsis'
    $taskStable = $false
    # Tauri may stamp the bundle type on the first invocation. Rebundle once so
    # the shipped provenance records the actual installer payload hash.
    foreach ($taskAttempt in 1..2) {
        $taskRecord.BinarySha256 = (Get-FileHash -LiteralPath $taskExe -Algorithm SHA256).Hash.ToLowerInvariant()
        $taskRecord | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath ('release/ema-desktop-' + $taskVersion + '-installer-build-info.json') -Encoding utf8NoBOM
        & node node_modules/@tauri-apps/cli/tauri.js bundle --bundles nsis --features custom-protocol --config release/tauri.clear-resources.json --config ('release/tauri.' + $taskVersion + '.current-user.json') --ci *> ('release/logs/bundle-' + $taskVersion + '-' + $taskAttempt + '.log')
        if ($LASTEXITCODE -ne 0) {
            Get-Content -LiteralPath ('release/logs/bundle-' + $taskVersion + '-' + $taskAttempt + '.log') -Tail 35
            throw 'NSIS bundling failed'
        }
        if ((Get-FileHash -LiteralPath $taskExe -Algorithm SHA256).Hash.ToLowerInvariant() -eq $taskRecord.BinarySha256) {
            $taskStable = $true
            break
        }
    }
    if (-not $taskStable) { throw 'Installer payload changed unexpectedly' }
    $taskInstaller = 'traybuddy_' + $taskVersion + '_x64-setup.exe'
    Copy-Item -LiteralPath (Join-Path 'src-tauri/target/release/bundle/nsis' $taskInstaller) -Destination ('release/traybuddy_' + $taskVersion + '_x64-setup-user.exe')
    Write-Output ('Built current-user installer v' + $taskVersion)
} finally {
    Pop-Location
}
