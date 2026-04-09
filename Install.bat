@echo off
title Desk Switch Installer
setlocal enabledelayedexpansion
cd /d "%~dp0"

echo.
echo ══════════════════════════════════════════
echo        Desk Switch — Windows Installer
echo ══════════════════════════════════════════
echo.

:: ── 1. Check / Install Rust ──────────────────────────────────

where cargo >nul 2>&1
if !errorlevel! neq 0 (
    echo [1/4] Rust not found. Installing...
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
    if !errorlevel! neq 0 (
        echo.
        echo ERROR: Rust installation failed.
        goto :fail
    )
    set "PATH=%USERPROFILE%\.cargo\bin;!PATH!"
    echo       Rust installed.
) else (
    echo [1/4] Rust found.
    cargo --version
)

:: Verify cargo works
where cargo >nul 2>&1
if !errorlevel! neq 0 (
    echo.
    echo ERROR: cargo command not found after install.
    echo        Please close this window, open a NEW terminal, and run Install.bat again.
    echo        (Rust was installed but the current terminal doesn't see it yet.)
    goto :fail
)

:: ── 2. Build ─────────────────────────────────────────────────

echo.
echo [2/4] Building desk-switch (release)...
echo       This takes a few minutes the first time. Please wait...
echo.
cargo build --release
if !errorlevel! neq 0 (
    echo.
    echo ERROR: Build failed. See errors above.
    goto :fail
)
echo.
echo       Build complete.

:: ── 3. Install to AppData + Desktop shortcut ─────────────────

set "INSTALL_DIR=%LOCALAPPDATA%\DeskSwitch"
set "EXE_SRC=target\release\desk-switch.exe"
set "EXE_DST=%INSTALL_DIR%\desk-switch.exe"

if not exist "%EXE_SRC%" (
    echo.
    echo ERROR: Built binary not found at %EXE_SRC%
    goto :fail
)

echo [3/4] Installing to %INSTALL_DIR%...

if not exist "%INSTALL_DIR%" mkdir "%INSTALL_DIR%"
copy /y "%EXE_SRC%" "%EXE_DST%" >nul

:: Create Desktop shortcut
powershell -ExecutionPolicy Bypass -Command ^
  "$ws = New-Object -ComObject WScript.Shell; " ^
  "$sc = $ws.CreateShortcut([IO.Path]::Combine($ws.SpecialFolders('Desktop'), 'Desk Switch.lnk')); " ^
  "$sc.TargetPath = '%EXE_DST%'; " ^
  "$sc.WorkingDirectory = '%INSTALL_DIR%'; " ^
  "$sc.Description = 'Cross-platform KVM switch'; " ^
  "$sc.Save()"

:: Create Start Menu shortcut
set "START_DIR=%APPDATA%\Microsoft\Windows\Start Menu\Programs"
powershell -ExecutionPolicy Bypass -Command ^
  "$ws = New-Object -ComObject WScript.Shell; " ^
  "$sc = $ws.CreateShortcut([IO.Path]::Combine('%START_DIR%', 'Desk Switch.lnk')); " ^
  "$sc.TargetPath = '%EXE_DST%'; " ^
  "$sc.WorkingDirectory = '%INSTALL_DIR%'; " ^
  "$sc.Description = 'Cross-platform KVM switch'; " ^
  "$sc.Save()"

echo       Installed. Shortcut on Desktop + Start Menu.

:: ── 4. Firewall rules ────────────────────────────────────────

echo.
echo [4/4] Adding firewall rules (may request Administrator)...
powershell -ExecutionPolicy Bypass -Command "Start-Process netsh -ArgumentList 'advfirewall firewall add rule name=DeskSwitch-TCP dir=in action=allow protocol=TCP localport=9876-9877' -Verb RunAs -Wait" 2>nul
powershell -ExecutionPolicy Bypass -Command "Start-Process netsh -ArgumentList 'advfirewall firewall add rule name=DeskSwitch-UDP dir=in action=allow protocol=UDP localport=9876-9877' -Verb RunAs -Wait" 2>nul

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
