@echo off
setlocal enabledelayedexpansion

REM Detect system architecture using environment variable
if /i "%PROCESSOR_ARCHITECTURE%"=="ARM64" (
    set "bazelisk=%~dp0bazelisk-windows-arm64.exe"
) else (
    set "bazelisk=%~dp0bazelisk-windows-amd64.exe"
)

REM Check if the executable exists
if not exist "%bazelisk%" (
    echo Error: %bazelisk% not found.
    echo Detected architecture: %PROCESSOR_ARCHITECTURE%
    echo Please ensure the appropriate Bazelisk executable is available.
    exit /b 1
)

REM Execute Bazelisk with all arguments passed to this script
"%bazelisk%" %*

exit /b %ERRORLEVEL%
