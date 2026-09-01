# Run the lifecycle test detached from CMD/Cursor so only this process blocks shutdown.
$ErrorActionPreference = "Stop"
$BundleDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $BundleDir

$PythonExe = & py -3.14 -c "import sys; print(sys.executable)" 2>$null
if (-not $PythonExe) {
    Write-Error "Python 3.14 not found. Install Python 3.14 and ensure 'py -3.14' works."
}

$Pythonw = $PythonExe -replace '\\python\.exe$', '\pythonw.exe' -replace '/python\.exe$', '/pythonw.exe'
if (-not (Test-Path $Pythonw)) {
    Write-Error "pythonw.exe not found next to $PythonExe"
}

Start-Process -FilePath $Pythonw `
    -ArgumentList "`"$BundleDir\test_lifecycle.py`"" `
    -WorkingDirectory $BundleDir `
    -WindowStyle Hidden

Write-Host "Lifecycle test started detached (pythonw, pid logged in log.txt)."
Write-Host "If a previous test is still running, stop pythonw.exe in Task Manager first."
Write-Host "Logs: $BundleDir\log.txt and $BundleDir\lifecycle_native.log"
Write-Host "Use Start menu -> Shut down to test. Shutdown should continue automatically after sync."
