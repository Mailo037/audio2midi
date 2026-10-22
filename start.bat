@echo off
rem Richtet beim ersten Start alles ein und oeffnet die Weboberflaeche.
cd /d "%~dp0"
if exist .venv goto run

echo Erstelle Umgebung ...
py -3.11 -m venv .venv || python -m venv .venv
.venv\Scripts\python -m pip install --upgrade pip
.venv\Scripts\pip install -r requirements.txt
.venv\Scripts\pip uninstall -y tensorflow tensorflow-estimator keras >nul 2>&1
set /p a="Spurtrennung (Gesang/Bass/...) installieren? ~1 GB [j/N] "
if /i not "%a%"=="j" goto run
.venv\Scripts\pip install -r requirements-stems.txt --extra-index-url https://download.pytorch.org/whl/cpu

:run
where ffmpeg >nul 2>&1 || echo WARNUNG: ffmpeg fehlt - installieren mit: winget install ffmpeg
.venv\Scripts\python app.py %*
pause
