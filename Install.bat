@echo off
title Desk Switch Installer
cd /d "%~dp0"

echo.
echo ══════════════════════════════════════════
echo        Desk Switch — Windows Installer
echo ══════════════════════════════════════════
echo.

:: ── 1. Check / Install Rust ──────────────────────────────────

echo [1/5] Checking for Rust...
where cargo >nul 2>&1 && (
    echo       Rust found:
    cargo --version
    goto :rust_ok
)

echo       Rust not found. Installing...
echo       Downloading rustup installer...
powershell -ExecutionPolicy Bypass -Command "Invoke-WebRequest -Uri 'https://win.rustup.rs/x86_64' -OutFile '%TEMP%\rustup-init.exe'"
if not exist "%TEMP%\rustup-init.exe" (
    echo.
    echo ERROR: Failed to download Rust installer.
    echo        Check your internet connection and try again.
    goto :fail
)
echo       Running installer...
"%TEMP%\rustup-init.exe" -y
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

where cargo >nul 2>&1 || (
    echo.
    echo ERROR: cargo not found after install.
    echo        Close this window, open a NEW terminal, and run Install.bat again.
    goto :fail
)
echo       Rust installed.

:rust_ok
echo       Updating Rust to latest version...
rustup update stable >nul 2>&1
echo.

:: ── 2. Build ─────────────────────────────────────────────────

echo [2/5] Building desk-switch (release)...
echo       This takes a few minutes the first time. Please wait...
echo.
cargo build --release 2>&1
if not exist "target\release\desk-switch.exe" (
    echo.
    echo ERROR: Build failed. The binary was not created.
    echo        Check the errors above.
    goto :fail
)
echo.
echo       Build complete.
echo.

:: ── 3. Install Virtual Display Driver ────────────────────────

echo [3/5] Setting up Virtual Display Driver (for extended display)...

set "VDD_DIR=C:\ProgramData\DeskSwitch\Driver"
if not exist "%VDD_DIR%" mkdir "%VDD_DIR%"

echo       The Virtual Display Driver lets the other laptop act as an
echo       extended screen (not just a mirror).
echo.
echo       Downloading Virtual Display Driver...
powershell -ExecutionPolicy Bypass -Command ^
  "try { " ^
  "  $releases = Invoke-RestMethod -Uri 'https://api.github.com/repos/VirtualDrivers/Virtual-Display-Driver/releases/latest'; " ^
  "  $asset = $releases.assets | Where-Object { $_.name -like '*.zip' } | Select-Object -First 1; " ^
  "  if ($asset) { " ^
  "    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile '%TEMP%\vdd.zip'; " ^
  "    Expand-Archive -Path '%TEMP%\vdd.zip' -DestinationPath '%VDD_DIR%' -Force; " ^
  "    Write-Host '       Driver downloaded.'; " ^
  "  } else { " ^
  "    Write-Host '       Could not find driver download.'; " ^
  "  } " ^
  "} catch { " ^
  "  Write-Host '       Driver download failed (extended display may not work).'; " ^
  "  Write-Host '       You can still use mirror mode.'; " ^
  "}" 2>nul

:: Try to install the driver (needs admin)
if exist "%VDD_DIR%\*.inf" (
    echo       Installing driver (may request Administrator)...
    powershell -ExecutionPolicy Bypass -Command ^
      "try { " ^
      "  $inf = Get-ChildItem '%VDD_DIR%' -Filter '*.inf' -Recurse | Select-Object -First 1; " ^
      "  if ($inf) { " ^
      "    Start-Process pnputil -ArgumentList ('/add-driver', $inf.FullName, '/install') -Verb RunAs -Wait; " ^
      "    Write-Host '       Driver installed.'; " ^
      "  } " ^
      "} catch { " ^
      "  Write-Host '       Driver install skipped (you can install manually later).'; " ^
      "}" 2>nul
) else (
    echo       No driver .inf found. Extended display may require manual setup.
    echo       See: https://github.com/VirtualDrivers/Virtual-Display-Driver
)
echo.

:: ── 4. Install to AppData + Desktop shortcut ─────────────────

set "INSTALL_DIR=%LOCALAPPDATA%\DeskSwitch"
set "EXE_SRC=target\release\desk-switch.exe"
set "EXE_DST=%INSTALL_DIR%\desk-switch.exe"

echo [4/5] Installing to %INSTALL_DIR%...

if not exist "%INSTALL_DIR%" mkdir "%INSTALL_DIR%"
copy /y "%EXE_SRC%" "%EXE_DST%" >nul
if not exist "%EXE_DST%" (
    echo.
    echo ERROR: Failed to copy binary to %INSTALL_DIR%
    goto :fail
)

:: Create Desktop shortcut
powershell -ExecutionPolicy Bypass -Command "$ws = New-Object -ComObject WScript.Shell; $sc = $ws.CreateShortcut([IO.Path]::Combine($ws.SpecialFolders('Desktop'), 'Desk Switch.lnk')); $sc.TargetPath = '%EXE_DST%'; $sc.WorkingDirectory = '%INSTALL_DIR%'; $sc.Description = 'Cross-platform KVM switch'; $sc.Save()"

:: Create Start Menu shortcut
set "START_DIR=%APPDATA%\Microsoft\Windows\Start Menu\Programs"
powershell -ExecutionPolicy Bypass -Command "$ws = New-Object -ComObject WScript.Shell; $sc = $ws.CreateShortcut([IO.Path]::Combine('%START_DIR%', 'Desk Switch.lnk')); $sc.TargetPath = '%EXE_DST%'; $sc.WorkingDirectory = '%INSTALL_DIR%'; $sc.Description = 'Cross-platform KVM switch'; $sc.Save()"

echo       Installed. Shortcut on Desktop + Start Menu.
echo.

:: ── 5. Firewall rules ────────────────────────────────────────

echo [5/5] Adding firewall rules (optional, may request Administrator)...
echo       If you see a security warning, you can click Allow or skip it.
powershell -ExecutionPolicy Bypass -Command "try { Start-Process netsh -ArgumentList 'advfirewall firewall add rule name=DeskSwitch-TCP dir=in action=allow protocol=TCP localport=9876-9877' -Verb RunAs -Wait } catch { }" 2>nul
powershell -ExecutionPolicy Bypass -Command "try { Start-Process netsh -ArgumentList 'advfirewall firewall add rule name=DeskSwitch-UDP dir=in action=allow protocol=UDP localport=9876-9877' -Verb RunAs -Wait } catch { }" 2>nul
echo       Firewall step done (or skipped).

echo.
echo ══════════════════════════════════════════
echo   Installation complete!
echo.
echo   - Desktop shortcut: "Desk Switch"
echo   - Start Menu: "Desk Switch"
echo   - Install path: %INSTALL_DIR%
echo.
echo   First time? The auth key is shown in
echo   the app. Copy it to the other machine
echo   so they can connect.
echo.
echo   Virtual Display: Extended display mode
echo   lets the other laptop be a 3rd screen.
echo   Toggle it in Settings.
echo ══════════════════════════════════════════
echo.

:: Launch
echo Launching Desk Switch...
start "" "%EXE_DST%"
echo.
echo You can close this window now.
pause
goto :eof

:fail
echo.
echo ══════════════════════════════════════════
echo   Installation did not complete.
echo   See the error above for details.
echo ══════════════════════════════════════════
echo.
pause
goto :eof
