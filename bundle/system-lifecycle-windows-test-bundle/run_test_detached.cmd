@echo off
REM Run lifecycle test detached from this CMD window (recommended for shutdown tests).
setlocal
cd /d "%~dp0"

for /f "delims=" %%P in ('py -3.14 -c "import sys; print(sys.executable)"') do set PYTHON=%%P
if errorlevel 1 (
  echo Python 3.14 not found.
  exit /b 1
)

set PYTHONW=%PYTHON:python.exe=pythonw.exe%
if not exist "%PYTHONW%" (
  echo pythonw.exe not found next to %PYTHON%
  exit /b 1
)

start "" /B "%PYTHONW%" "%~dp0test_lifecycle.py"
echo Lifecycle test started detached. Logs: %~dp0log.txt
echo Use Start menu -^> Shut down to test.
