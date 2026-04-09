@echo off
title Virtual Display Driver Installer
cd /d "%~dp0"

echo.
echo ══════════════════════════════════════════
echo   Virtual Display Driver (VDD) Installer
echo ══════════════════════════════════════════
echo.
echo   This driver lets Desk Switch create a
echo   virtual 3rd monitor on Windows so the
echo   other laptop is an extended screen
echo   (not just a mirror).
echo.
echo   NOTE: You may need to enable test signing:
echo     1. Open CMD as Administrator
echo     2. Run: bcdedit /set testsigning on
echo     3. Reboot
echo     4. Then run this script again
echo.
pause

echo.
echo [1/3] Downloading Virtual Display Driver...

set "VDD_DIR=%TEMP%\VDD_Install"
if not exist "%VDD_DIR%" mkdir "%VDD_DIR%"

powershell -ExecutionPolicy Bypass -Command ^
  "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; " ^
  "$ProgressPreference = 'SilentlyContinue'; " ^
  "Invoke-WebRequest -Uri 'https://github.com/VirtualDrivers/Virtual-Display-Driver/releases/latest/download/VirtualDisplayDriver.zip' -OutFile '%VDD_DIR%\vdd.zip'; " ^
  "if (Test-Path '%VDD_DIR%\vdd.zip') { Expand-Archive -Path '%VDD_DIR%\vdd.zip' -DestinationPath '%VDD_DIR%\extracted' -Force; Write-Host 'Download OK' } else { Write-Host 'Download FAILED' }"

if not exist "%VDD_DIR%\extracted" (
    echo.
    echo       Automatic download failed. Trying alternate URL...
    powershell -ExecutionPolicy Bypass -Command ^
      "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; " ^
      "$ProgressPreference = 'SilentlyContinue'; " ^
      "$r = Invoke-RestMethod 'https://api.github.com/repos/VirtualDrivers/Virtual-Display-Driver/releases/latest' -TimeoutSec 15; " ^
      "$a = $r.assets | Where-Object { $_.name -like '*.zip' } | Select-Object -First 1; " ^
      "if ($a) { Invoke-WebRequest $a.browser_download_url -OutFile '%VDD_DIR%\vdd.zip' -TimeoutSec 60; Expand-Archive '%VDD_DIR%\vdd.zip' -DestinationPath '%VDD_DIR%\extracted' -Force; Write-Host 'Download OK' } else { Write-Host 'No zip asset found' }"
)

if not exist "%VDD_DIR%\extracted" (
    echo.
    echo ERROR: Could not download the driver.
    echo        Please download manually from:
    echo        https://github.com/VirtualDrivers/Virtual-Display-Driver/releases
    echo.
    echo        Extract the zip, then right-click the .inf file
    echo        and choose "Install".
    echo.
    pause
    goto :eof
)

echo       Downloaded.
echo.

echo [2/3] Looking for driver .inf file...
set "INF_FILE="
for /r "%VDD_DIR%\extracted" %%f in (*.inf) do (
    set "INF_FILE=%%f"
)

if "%INF_FILE%"=="" (
    echo       No .inf file found in download.
    echo       Please install manually from the extracted files at:
    echo       %VDD_DIR%\extracted
    pause
    goto :eof
)

echo       Found: %INF_FILE%
echo.

echo [3/3] Installing driver (requires Administrator)...
echo       A UAC prompt will appear — click Yes.
echo.

powershell -ExecutionPolicy Bypass -Command "Start-Process pnputil -ArgumentList '/add-driver','%INF_FILE%','/install' -Verb RunAs -Wait"

echo.
echo ══════════════════════════════════════════
echo   Done! You may need to reboot for the
echo   driver to take effect.
echo.
echo   After reboot, open Desk Switch and
echo   make sure "Virtual Display" is set to
echo   "Extended (3rd screen)" in Settings.
echo ══════════════════════════════════════════
echo.

:: Create the driver config directory
set "VDD_CFG=C:\ProgramData\DeskSwitch\Driver"
if not exist "%VDD_CFG%" mkdir "%VDD_CFG%"
echo Created config directory: %VDD_CFG%

pause
