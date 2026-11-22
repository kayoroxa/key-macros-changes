# Instruções para agentes (KeyMacros)

## Rodar o app

- Nunca abra `dist\keymacros.exe` com `--paused` para testar ou verificar algo. Essa flag força o teclado pausado e ignora o estado salvo em `dist\keymacros-estado.txt`, então o ícone da bandeja fica cinza.
- Para subir o app, rode `dist\keymacros.exe` sem flags, ou use o `KeyMacros.exe` da raiz (launcher). Assim ele respeita o estado salvo.
- Só use `--paused` se o usuário pedir explicitamente.

## Depois de cada mudança

- Sempre que modificar algo no código, reinicie o app: compile (`cargo build --release`), pare o `keymacros`, copie para `dist\keymacros.exe` e suba o `dist\keymacros.exe` sem flags. Não deixe a versão antiga rodando.

## Build

- Compile com `cargo build --release` e copie `target\release\keymacros.exe` para `dist\keymacros.exe`. Pare o processo `keymacros` antes de copiar.
- Não commite nem altere `dist\keymacros-estado.txt` nem arquivos em `data\`.
