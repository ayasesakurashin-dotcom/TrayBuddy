param([switch]$LiveMemory)
$ErrorActionPreference = 'Stop'
$taskRepo = Split-Path -Parent $PSScriptRoot
Push-Location $taskRepo
try {
    $taskEvents = & cargo test --manifest-path src-tauri/Cargo.toml --lib --no-run --offline --message-format=json
    if ($LASTEXITCODE -ne 0) { throw 'Rust test compilation failed.' }
    $taskExecutable = $null
    foreach ($taskLine in $taskEvents) {
        try { $taskEvent = $taskLine | ConvertFrom-Json } catch { continue }
        if ($taskEvent.reason -eq 'compiler-artifact' -and $taskEvent.target.name -eq 'traybuddy_lib' -and $taskEvent.executable) { $taskExecutable = $taskEvent.executable }
    }
    if (-not $taskExecutable) { throw 'Test executable was not produced.' }
    # lib unit tests do not receive Tauri's bin-only Common Controls manifest.
    # Embed a manifest in the generated test binary before starting it.
    $taskManifest = Join-Path $PSScriptRoot 'windows-test.manifest'
    $taskMt = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Directory |
        Where-Object { $_.Name -like '10.*' } | Sort-Object Name -Descending |
        ForEach-Object { Join-Path $_.FullName 'x64\mt.exe' } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $taskMt) { throw 'Windows SDK mt.exe was not found.' }
    & $taskMt -nologo -manifest $taskManifest "-outputresource:$taskExecutable;#1"
    if ($LASTEXITCODE -ne 0) { throw 'Could not embed the Windows test manifest.' }
    if ($LiveMemory) { & $taskExecutable live_hindsight_isolated --ignored --nocapture } else { & $taskExecutable companion --nocapture }
    if ($LASTEXITCODE -ne 0) { throw 'Companion tests failed.' }
} finally { Pop-Location }
