# KeyMacros

Macros de teclado para Windows usando a **linguagem do [kanata](https://github.com/jtroo/kanata)**,
com um app em Rust que fica na bandeja e um editor visual para criar atalhos.

```
teclado ─▶ (SharpKeys / Scancode Map, opcional) ─▶ kanata  ──push-msg (TCP)──▶ keymacros (bandeja)
                                                     │                              │
                                     remapeia, camadas, tap-hold           abre/foca apps, sites,
                                     (sem atraso, em Rust)                 digita texto, janelas…
```

- **kanata** faz a parte "baixo nível": intercepta as teclas, camadas, tocar vs. segurar, sem o
  atraso/bugs de hotkeys do AHK.
- **keymacros.exe** (este projeto) faz o que o kanata não faz: focar a janela de um app que já está
  aberto (ou abrir), ir para uma aba do navegador, digitar texto unicode, alternar janelas, etc.
  O kanata manda o pedido com `(push-msg ...)` — sem criar processos, menos de 1 ms.

## Uso

**Clique em `KeyMacros.exe` (raiz do projeto).** Ele vê se algum arquivo do código mudou desde o
último build; se mudou, compila (mostrando o console), fecha o app antigo, atualiza `dist\` e reabre.
Se não mudou, só abre. O app fica na bandeja (setinha "^" perto do relógio): **um clique abre o
editor**, botão direito mostra o menu. Na primeira vez o teclado começa **pausado**; depois o app
lembra se você deixou ativo ou pausado (`keymacros-estado.txt`).

```powershell
.\build-dist.ps1            # compila e monta dist\
dist\keymacros.exe --paused # bandeja SEM mexer no teclado (para testar o editor)
dist\keymacros.exe          # bandeja + kanata ativo (atalhos valendo)
dist\keymacros.exe editor   # editor de macros (editor troca = abre na aba Troca de teclas)
dist\keymacros.exe check    # valida a config (kanata --check + ações do app)
dist\keymacros.exe layers   # lista os atalhos das camadas km-*
dist\keymacros.exe windows  # lista janelas abertas (exe/classe) para usar em "app"
dist\keymacros.exe scancode # mostra o que o SharpKeys trocou no registro
```

> **Antes de ativar:** feche o AutoHotkey (`main.ahk`). Os dois usam o F13 e vão brigar.
> "Pausar" na bandeja mata o kanata na hora e o teclado volta ao normal.

Config: `keymacros.kbd` ao lado do .exe (modo portátil) ou `%APPDATA%\KeyMacros\keymacros.kbd`
(criada do modelo na primeira execução; se o SharpKeys estiver ativo o modelo já usa `f13`).
Ao salvar o arquivo, o app valida e recarrega sozinho; se tiver erro, mantém a config anterior e avisa.

## Como voltar ao teclado normal (sem reiniciar)

O kanata (modo LLHOOK) é um programa comum: não grava nada no registro, não instala driver.
Se ele não estiver rodando, o teclado é 100% o do Windows.

1. **Botão de pânico:** segure **Ctrl esquerdo + Espaço + Esc** juntos → o kanata fecha na hora.
   Isso é checado antes de qualquer remapeamento, então funciona mesmo com a config quebrada.
2. Ícone da bandeja → **Pausar**.
3. **Ctrl+Alt+Del** (o kanata não consegue interceptar) → Gerenciador de Tarefas → finalizar `kanata.exe`.
4. O mouse nunca é afetado.
5. Reiniciar o PC sempre volta ao normal, a não ser que "Iniciar com o Windows" esteja marcado.

O KeyMacros **nunca religa o kanata sozinho**: se ele sair por qualquer motivo, fica pausado até
você clicar em "Ativar teclado". Uma config com erro nunca é carregada.

A exceção é o **SharpKeys** (Scancode Map): ele age no driver do teclado durante o boot, por isso
exige reiniciar para aplicar e para desfazer.

## O que foi portado do `main.ahk`

| AHK | kanata (`config/keymacros.kbd`) |
|---|---|
| `F13 & c` tocar → Claude, segurar 0,3 s → WezTerm/Terminal | `c (tap-hold 0 300 (push-msg app "claude.exe" …) (push-msg first (focus …) … (app "wezterm-gui.exe" …)))` |
| `F13 & t` Telegram alternando janela de chamada/normal | `(push-msg app "Telegram.exe" … "prefer=(")` — apertar de novo vai para a próxima janela |
| `F13 & d/v/o/a/f/s/e/n` | `(push-msg app EXE CAMINHO …)` (OBS com `nomax`, Explorer com `class=CabinetWClass`, Notepads por título/classe) |
| `F13 & b` e depois `u`/`y` em até 2 s | `(multi (push-msg browser) (one-shot 2000 (layer-while-held km-sites)))` |
| `F13 & g` ChatGPT na aba existente | `(push-msg browser-tab "chatgpt.com\|chat.openai.com" …)` (Ctrl+L/Ctrl+C restaurando o clipboard, Ctrl+Shift+A) |
| `^!Up` sempre no topo, `^!d` próxima janela, `^!ç` suspender | camada `km-alt` com `fork` em Ctrl |
| `#F12` desligar, `#'` suspender | camada `km-win` |
| CapsLock = Enter, Shift+CapsLock = CapsLock | `caps (fork ret caps (lsft rsft))` |
| `F13` sozinho manda F13 | `(tap-hold-press 0 200 f13 (layer-while-held km-lider))` |

Não portado: `mouse-control-alt-scroll.ahk`/`premiere.ahk` (não estavam incluídos no `main.ahk`).

## Combinações (teclas apertadas juntas)

Ex.: **R + T juntos = F14**. Digitando R e depois T, sai "rt" normal.

```
(defchordsv2
  (r t) f14 40 first-release ()
)
```

- `40` = as teclas precisam ser apertadas com até 40 ms de diferença para contar como "juntas".
- `chords-v2-min-idle 150` no defcfg: depois de qualquer outra tecla, combinações ficam desligadas
  por 150 ms, então digitando uma palavra ("parte", "arte") a combinação não dispara.
- Custo: as teclas que participam de uma combinação (R e T) saem com até 40 ms de espera, porque o
  kanata precisa esperar para saber se a outra tecla vem junto. As outras teclas não são afetadas.
- A ação pode ser qualquer coisa: uma tecla (`f14`), um atalho (`C-S-a`) ou `(push-msg app ...)`.
- No editor, toda camada tem uma **tecla líder** (primeira linha da camada, botão "Trocar tecla líder").
  Uma tecla = segurar ela e apertar o atalho. Várias teclas (ex.: `r t`) = apertar juntas e depois o
  atalho (até 2 s), gerando `(r t) (one-shot 2000 (layer-while-held km-rt)) 40 first-release ()`.
  Para criar: botão "+ Camada" → tecla líder `r t`. O editor liga `concurrent-tap-hold yes` sozinho
  (o kanata exige isso para usar combinações).
- Toda camada nova ganha `___ XX` (outras teclas não fazem nada); isso fica no arquivo, mas não aparece no editor.

## Troca de teclas e tecla líder

Aba **"Troca de teclas"** do editor. Cada troca pode ser feita de dois jeitos:

| | Pelo kanata | Pelo registro (Scancode Map, igual ao SharpKeys) |
|---|---|---|
| quando vale | na hora, ao salvar | depois de **Reiniciar** o PC |
| precisa de admin | não | sim (o Windows mostra o aviso do UAC) |
| com o teclado pausado / kanata fechado | não vale | vale |
| tela de login, UAC, apps de administrador | não vale | vale |
| velocidade | igual (menos de 1 ms) | igual |

- **Tecla líder:** escolha a tecla física e o modo. Pelo kanata, a config passa a usar a tecla que o
  kanata recebe. Pelo registro, a tecla vira uma F13–F24 livre e a config usa essa F-tecla.
- **Trocar uma tecla por outra:** "Apertar X vira Y". As trocas do kanata ficam na camada base
  (`rctl lmet`); as do registro ficam numa lista que só é gravada em "Aplicar no registro".
- **Voltar ao normal:** "Voltar ao normal (tirar tudo do registro)" remove o Scancode Map inteiro;
  no kanata, "Remover" em cada troca (ou Pausar).

### Backups automáticos (nunca apagados)

Ficam em `backups\` ao lado da config:

- `config-DATA-HORA.kbd`: a config **antes** de cada salvamento (qualquer aba).
- `registro-DATA-HORA-original.reg`: o registro como estava na primeira vez que o editor abriu.
- `registro-DATA-HORA-antes.reg`: o registro antes de cada alteração; `...-novo.reg`: o que foi aplicado.

Na seção "Backups" da aba dá para restaurar um `.reg` (pede admin, reiniciar) ou carregar uma
config antiga no editor. Os `.reg` também funcionam com duplo clique. O app nunca apaga backups e
só mexe no registro quando você clica em um botão de aplicar/restaurar.

## Ações disponíveis em `(push-msg ...)`

| comando | o que faz |
|---|---|
| `app EXE [ABRIR] [ARGS] [opções]` | foca a janela do app; se já estiver na frente, vai para a próxima; se fechado, abre e maximiza |
| `focus EXE [opções]` | só foca (não abre) — útil dentro de `first` |
| `run CMD [ARGS]` | executa programa, arquivo, URL ou `shell:AppsFolder\...` |
| `browser` / `browser-tab MATCH[\|MATCH2] [BUSCA] [URL]` | Edge: abre/foca / vai para a aba do site |
| `next-window [EXE…] [class:X] [title:X]` | próxima janela, pulando as listadas |
| `type TEXTO` | digita texto (unicode, funciona em ABNT2) |
| `topmost` `maximize` `minimize` `close-window` | janela ativa |
| `sleep-pc` `shutdown` `lock` | sistema |
| `notify TEXTO`, `wait MS` | aviso na bandeja / espera |
| `first (…) (…)` / `seq (…) (…)` | primeiro que funcionar / todos em ordem |

Opções: `nomax`, `title=…`, `title-ends=…`, `class=…`, `prefer=…`, `wait=SEGUNDOS`, `dir=…`.
Strings do kanata não têm escape: use `"..."` ou `r#"com "aspas""#`. `%LocalAppData%` etc. são expandidos.

## Memória

| processo | quando | RAM (medida) |
|---|---|---|
| `keymacros.exe` (bandeja) | sempre | ~0,5 MB no Gerenciador de Tarefas (2,2 MB privados) |
| `kanata.exe` | quando ativo | alguns MB |
| `keymacros.exe editor` | só com a janela aberta | ~150 MB (driver OpenGL do egui); fecha tudo ao sair |

## Limitações conhecidas

- Como o AHK, o kanata (modo LLHOOK) não intercepta teclas em janelas de administrador, a menos que
  o KeyMacros seja aberto como administrador.
- `windows-altgr add-lctl-release` foi escolhido por causa do AltGr do ABNT2; ainda não testado ao vivo.
- O kanata é LGPL-3.0; o `kanata.exe` é distribuído sem modificações (código: github.com/jtroo/kanata).

## Desenvolvimento

```powershell
cargo test           # parser do .kbd, protocolo push-msg, Scancode Map, config real
cargo build --release
```

Módulos: `msg.rs` (protocolo das ações), `kbd.rs` (parser/escritor de `.kbd` preservando o resto do
arquivo), `actions.rs` + `win.rs` (Win32), `daemon.rs` (bandeja, supervisor do kanata, TCP, recarga),
`editor.rs` (egui), `remap.rs` (camada base: trocas e tecla líder), `backup.rs` (backups), `theme.rs` (visual do editor: cores, fontes Segoe do Windows, botões/teclas/cartões), `scancode.rs` (Scancode Map: ler, gravar com UAC).
