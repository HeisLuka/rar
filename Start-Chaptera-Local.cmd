@echo off
setlocal
cd /d "%~dp0"

if exist "%~dp0runtime\python\python.exe" goto packaged

where py >nul 2>nul
if %errorlevel%==0 (
  py -3 tools\run_local_full_stack.py %*
) else (
  python tools\run_local_full_stack.py %*
)
goto done

:packaged
set "CHAPTERA_LOCAL_RUNTIME_ROOT=%~dp0"
if defined LOCALAPPDATA (
  set "CHAPTERA_LOCAL_STATE_ROOT=%LOCALAPPDATA%\Chaptera\Local"
) else (
  set "CHAPTERA_LOCAL_STATE_ROOT=%~dp0.chaptera-local"
)
set "CHAPTERA_LOCAL_PACKAGED=1"
set "PYTHONDONTWRITEBYTECODE=1"
"%~dp0runtime\python\python.exe" "%~dp0launcher\run_local_full_stack.py" %*

:done
set "CHAPTERA_EXIT=%errorlevel%"
if not "%CHAPTERA_EXIT%"=="0" (
  echo.
  echo Chaptera Local failed. A browser error page should have opened.
  pause
)
exit /b %CHAPTERA_EXIT%
