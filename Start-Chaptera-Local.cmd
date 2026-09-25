@echo off
setlocal
cd /d "%~dp0"
where py >nul 2>nul
if %errorlevel%==0 (
  py -3 tools\run_local_full_stack.py %*
) else (
  python tools\run_local_full_stack.py %*
)
if not %errorlevel%==0 (
  echo.
  echo Chaptera Local failed. A browser error page should have opened.
  pause
)
