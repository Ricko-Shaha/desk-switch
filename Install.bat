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

:: ── 2. Check / Install build dependencies (cmake, nasm) ──────

echo [2/5] Checking build dependencies (cmake, nasm)...

where cmake >nul 2>&1 || (
    echo       cmake not found. Installing via winget...
    where winget >nul 2>&1 && (
        winget install Kitware.CMake --accept-source-agreements --accept-package-agreements -h >nul 2>&1
        set "PATH=%ProgramFiles%\CMake\bin;%PATH%"
    ) || (
        echo       winget not available. Downloading cmake...
        powershell -ExecutionPolicy Bypass -Command "$ProgressPreference='SilentlyContinue'; Invoke-WebRequest 'https://github.com/Kitware/CMake/releases/download/v3.31.4/cmake-3.31.4-windows-x86_64.zip' -OutFile '%TEMP%\cmake.zip'; Expand-Archive '%TEMP%\cmake.zip' -DestinationPath '%LOCALAPPDATA%\cmake' -Force"
        for /d %%d in ("%LOCALAPPDATA%\cmake\cmake-*") do set "PATH=%%d\bin;%PATH%"
    )
)

where cmake >nul 2>&1 && (
    echo       cmake: found
) || (
    echo.
    echo ERROR: cmake is required but could not be installed.
    echo        Install manually from https://cmake.org/download/
    echo        Then run Install.bat again.
    goto :fail
)

where nasm >nul 2>&1 || (
    echo       nasm not found. Installing via winget...
    where winget >nul 2>&1 && (
        winget install NASM.NASM --accept-source-agreements --accept-package-agreements -h >nul 2>&1
        set "PATH=%ProgramFiles%\NASM;%PATH%"
        set "PATH=%LOCALAPPDATA%\bin\NASM;%PATH%"
    ) || (
        echo       winget not available. Downloading nasm...
        powershell -ExecutionPolicy Bypass -Command "$ProgressPreference='SilentlyContinue'; Invoke-WebRequest 'https://www.nasm.us/pub/nasm/releasebuilds/2.16.03/win64/nasm-2.16.03-win64.zip' -OutFile '%TEMP%\nasm.zip'; Expand-Archive '%TEMP%\nasm.zip' -DestinationPath '%LOCALAPPDATA%\nasm' -Force"
        for /d %%d in ("%LOCALAPPDATA%\nasm\nasm-*") do set "PATH=%%d;%PATH%"
    )
)

where nasm >nul 2>&1 && (
    echo       nasm: found
) || (
    echo       nasm: not found (building without SIMD - slightly slower encoding)
)
echo.

:: ── 3. Build ─────────────────────────────────────────────────

echo [3/5] Building desk-switch (release)...
echo       This takes a few minutes the first time. Please wait...
echo.
cargo build --release 2>&1
if not exist "target\release\desk-switch.exe" (
    echo.
    echo       Fast JPEG build failed (missing cmake/nasm). Building with fallback...
    echo.
    cargo build --release --no-default-features 2>&1
)
if not exist "target\release\desk-switch.exe" (
    echo.
    echo ERROR: Build failed. The binary was not created.
    echo        Check the errors above.
    goto :fail
)
echo.
echo       Build complete.
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

echo [5/5] Adding firewall rules...
netsh advfirewall firewall delete rule name=DeskSwitch-TCP >nul 2>&1
netsh advfirewall firewall delete rule name=DeskSwitch-UDP >nul 2>&1
netsh advfirewall firewall add rule name=DeskSwitch-TCP dir=in action=allow protocol=TCP localport=9876-9877 >nul 2>&1
netsh advfirewall firewall add rule name=DeskSwitch-UDP dir=in action=allow protocol=UDP localport=9876-9877 >nul 2>&1
netsh advfirewall firewall add rule name=DeskSwitch-TCP-Out dir=out action=allow protocol=TCP remoteport=9876-9877 >nul 2>&1
netsh advfirewall firewall add rule name=DeskSwitch-UDP-Out dir=out action=allow protocol=UDP remoteport=9876-9877 >nul 2>&1
netsh advfirewall firewall add rule name=DeskSwitch-App dir=in action=allow program="%EXE_DST%" >nul 2>&1
echo       Firewall rules added (inbound + outbound).

echo.
echo ══════════════════════════════════════════
echo   Installation complete!
echo.
echo   - Desktop shortcut: "Desk Switch"
echo   - Start Menu: "Desk Switch"
echo   - Install path: %INSTALL_DIR%
echo.
echo   HOW TO USE AS 3RD SCREEN:
echo   1. On Mac: Start as Primary
echo   2. On Windows: Start as Display
echo   3. Mac creates a virtual display that
echo      Windows shows as a 3rd screen
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
