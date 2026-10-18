param(
    [Parameter(Mandatory = $true)]
    [string]$Executable
)

$ErrorActionPreference = 'Stop'
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$reportPath = Join-Path ([System.IO.Path]::GetTempPath()) ("ema-package-check-" + [guid]::NewGuid() + '.json')
try {
    $checkProcess = Start-Process -FilePath $executablePath -ArgumentList @('--verify-package', ('"' + $reportPath + '"')) -WindowStyle Hidden -PassThru
    if (-not $checkProcess.WaitForExit(30000)) {
        $checkProcess.Kill()
        throw 'Package verification did not finish within 30 seconds.'
    }
    if (-not (Test-Path -LiteralPath $reportPath)) {
        throw 'Executable did not produce a package report. It may be an older build.'
    }
    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    if ($checkProcess.ExitCode -ne 0 -or -not $report.ok -or -not $report.production_mode) {
        throw ("Package verification failed: " + ($report.errors -join '; '))
    }
    $report | ConvertTo-Json -Depth 4
} finally {
    if (Test-Path -LiteralPath $reportPath) {
        Remove-Item -LiteralPath $reportPath
    }
}
