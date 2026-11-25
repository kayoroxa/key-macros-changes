#![cfg_attr(not(test), windows_subsystem = "windows")]
//! KeyMacros — bandeja + editor de macros em cima do kanata.
//!
//!   keymacros                 inicia na bandeja e liga o kanata
//!   keymacros --paused        inicia na bandeja SEM mexer no teclado
//!   keymacros editor          abre o editor de macros
//!   keymacros check           valida a config (kanata --check + ações)
//!   keymacros layers          lista as camadas km-* e seus atalhos
//!   keymacros msg JSON        mostra como um push-msg seria interpretado
//!   keymacros exec JSON       executa uma ação agora (teste manual)
//!   keymacros scancode        mostra o Scancode Map (SharpKeys) atual
//!   keymacros windows         lista as janelas abertas (exe, classe, título)
//!
//! Opções: --cfg ARQUIVO  --port N

mod actions;
mod backup;
mod chords;
mod clock;
mod features;
mod trigger;
mod daemon;
mod editor;
mod kbd;
mod msg;
mod paths;
mod remap;
mod scancode;
mod theme;
mod update;
mod win;

fn attach_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

fn main() {
    // Quando roda como administrador, deixa o editor/launcher/Explorer (sem admin)
    // continuarem mandando mensagens para as nossas janelas (o UIPI bloquearia).
    win::allow_messages_from_lower(&[
        windows_sys::Win32::UI::WindowsAndMessaging::WM_CLOSE,
        daemon::WM_KM_TOGGLE_PAUSE,
        daemon::WM_KM_RELOAD,
        daemon::WM_KM_SET_PAUSED,
        daemon::WM_KM_FEATURES,
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(win::wide("TaskbarCreated").as_ptr()) },
    ]);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg_arg: Option<String> = None;
    let mut port = paths::DEFAULT_PORT;
    let mut paused = false;
    let mut rest: Vec<String> = vec![];
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--cfg" | "-c" => cfg_arg = it.next(),
            "--port" | "-p" => port = it.next().and_then(|p| p.parse().ok()).unwrap_or(port),
            "--paused" => paused = true,
            _ => rest.push(a),
        }
    }
    let cfg = paths::config_path(cfg_arg.as_deref());
    let code = match rest.first().map(String::as_str) {
        None => daemon::run(cfg, port, paused),
        // aberto pelo início do Windows só para mostrar o pop-up de atualização
        Some("editor") => editor::run(cfg, rest.get(1).map(String::as_str), rest.iter().any(|a| a == "--atualizacao")),
        // chamado pelo próprio app depois de atualizar: troca o exe e reabre
        Some("aplicar-atualizacao") => update::aplicar(&rest[1..], &cfg),
        Some(cmd) => {
            attach_console();
            cli(cmd, &rest[1..], &cfg)
        }
    };
    std::process::exit(code);
}

fn cli(cmd: &str, rest: &[String], cfg: &std::path::Path) -> i32 {
    match cmd {
        "check" => match paths::check_config(cfg) {
            Ok(()) => {
                println!("OK: {} é válida", cfg.display());
                0
            }
            Err(e) => {
                eprintln!("ERRO em {}:\n{e}", cfg.display());
                1
            }
        },
        "layers" => {
            let text = match std::fs::read_to_string(cfg) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{e}");
                    return 1;
                }
            };
            match kbd::managed_layers(&text) {
                Ok(layers) => {
                    for l in layers {
                        println!("[{}]", l.name);
                        for b in l.bindings {
                            let hold = b.hold.map(|(ms, a)| format!("   | segurar {ms}ms: {}", a.summary())).unwrap_or_default();
                            println!("  {:<6} {}{hold}", b.key, b.tap.summary());
                        }
                    }
                    if let Ok((_, list)) = chords::read(&text) {
                        if !list.is_empty() {
                            println!("[combinações — teclas juntas]");
                            for c in list {
                                println!("  {:<10} {}   (até {} ms)", c.keys.join("+"), c.action.summary(), c.timeout);
                            }
                        }
                    }
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        "msg" | "exec" => {
            let Some(json) = rest.first() else {
                eprintln!("uso: keymacros {cmd} '[\"app\",\"notepad.exe\"]'");
                return 2;
            };
            let v: serde_json::Value = match serde_json::from_str(json) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("JSON inválido: {e}");
                    return 2;
                }
            };
            match msg::parse_json(&v) {
                Ok(c) => {
                    println!("{c:#?}");
                    if cmd == "exec" {
                        win::mask_win_alt();
                        let r = actions::exec(&c, &|t| println!("notify: {t}"));
                        println!("resultado: {r:?}");
                    }
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        "admin" => {
            println!("administrador: {}  pode elevar: {}", win::is_elevated(), win::can_elevate());
            0
        }
        // usado pelo próprio app (elevado) para ligar/desligar a tarefa de início
        "autostart" => {
            let on = !matches!(rest.first().map(String::as_str), Some("off"));
            match daemon::set_autostart(on, cfg) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        "atualizacao" => match update::check() {
            Ok(None) => {
                println!("Atualizado: nenhuma mudança nova no GitHub.");
                0
            }
            Ok(Some(commits)) => {
                println!("Há {} atualização(ões) no GitHub:", commits.len());
                for c in commits {
                    println!("  • {c}");
                }
                0
            }
            Err(e) => {
                eprintln!("Não consegui checar: {e}");
                1
            }
        },
        "windows" => {
            for w in win::list_windows() {
                let t: String = w.title.chars().take(60).collect();
                println!("{:<28} class={:<28} {}{}", w.exe, w.class, t, if w.cloaked { "  [oculta]" } else { "" });
            }
            0
        }
        "scancode" => match scancode::read_system() {
            Ok(None) => {
                println!("Nenhum Scancode Map no registro (SharpKeys não está ativo).");
                0
            }
            Ok(Some(m)) => {
                println!("Scancode Map (HKLM\\SYSTEM\\CurrentControlSet\\Control\\Keyboard Layout):");
                for x in &m {
                    println!("  {}", scancode::describe(x));
                }
                if let Some(l) = scancode::sharpkeys_leader(&m) {
                    println!("=> no kanata, o Windows esquerdo físico chega como `{l}`.");
                }
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        _ => {
            eprintln!("comando desconhecido: {cmd}. Use: editor | check | layers | msg | exec | scancode | windows");
            2
        }
    }
}
