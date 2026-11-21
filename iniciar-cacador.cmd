@echo off
rem Inicia o servidor local do Tap/Hold Lab e abre o site. Dados em data\combo-data.json
cd /d "%~dp0"
where python >nul 2>nul && (python serve.py & goto :end)
where py >nul 2>nul && (py serve.py & goto :end)
echo Python nao encontrado no PATH. Instale em python.org e tente de novo.
:end
pause
