# Gera a pasta dist\ portátil: keymacros.exe + kanata.exe + config.
# Para mandar para outra pessoa: zipe a pasta dist\ (sem o keymacros.kbd pessoal,
# o app cria um a partir do modelo na primeira execução).
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
cargo build --release
if (-not (Test-Path tools\kanata\bin\kanata.exe)) {
    cargo install kanata --version 1.11.0 --locked --root tools\kanata
}
New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item target\release\keymacros.exe dist\ -Force
Copy-Item tools\kanata\bin\kanata.exe dist\ -Force
Copy-Item config\exemplo.kbd dist\exemplo.kbd -Force
if (-not (Test-Path dist\keymacros.kbd)) { Copy-Item config\keymacros.kbd dist\keymacros.kbd }
Copy-Item README.md dist\ -Force
"dist pronto:"; Get-ChildItem dist | Select-Object Name, Length
