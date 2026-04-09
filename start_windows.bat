@echo off
cd /d "%~dp0"
set BIN=target\release\desk-switch.exe
if not exist %BIN% (
    echo Binary not found. Building...
    cargo build --release
)
start "" %BIN%
