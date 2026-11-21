//! Vocabulário de ações que o kanata envia via `(push-msg ...)`.
//!
//! O kanata transforma `(push-msg app "claude.exe" "shell:...")` em JSON
//! `["app","claude.exe","shell:..."]` e manda pelo TCP. Este módulo converte
//! esse array num [`Cmd`] validado. É puro (sem Win32) para poder ser testado.

use serde_json::Value;

/// Filtro para encontrar janelas.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WinFilter {
    /// Nome do processo (ex.: `claude.exe`). `None` = qualquer processo.
    pub exe: Option<String>,
    /// O título precisa conter este texto (sem diferenciar maiúsculas).
    pub title: Option<String>,
    /// O título precisa terminar com este texto.
    pub title_ends: Option<String>,
    /// Classe da janela (ex.: `CabinetWClass`).
    pub class: Option<String>,
    /// Janelas cujo título contém este texto vêm primeiro no ciclo.
    pub prefer: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    /// Foca a janela do app (e alterna entre janelas se já estiver ativo);
    /// se não houver janela e `launch` for true, executa `run`.
    App {
        filter: WinFilter,
        run: Option<String>,
        args: Option<String>,
        dir: Option<String>,
        maximize: bool,
        launch: bool,
        wait_ms: u64,
    },
    /// Executa um programa/arquivo/URL/`shell:`.
    Run { cmd: String, args: Option<String>, dir: Option<String> },
    /// Ativa o navegador e vai para a aba/site (usa Ctrl+Shift+A do Edge/Chrome).
    BrowserTab {
        matches: Vec<String>,
        search: String,
        url: String,
        filter: WinFilter,
        run: String,
    },
    /// Alterna para a próxima janela que não está na lista de exclusão.
    NextWindow { skip: Vec<WinFilter> },
    /// Digita um texto (unicode, independente do layout do teclado).
    Type(String),
    Topmost,
    Maximize,
    Minimize,
    CloseWindow,
    SleepPc,
    Shutdown,
    Lock,
    Notify(String),
    Wait(u64),
    /// Executa cada subcomando até um dar certo.
    First(Vec<Cmd>),
    /// Executa todos em sequência.
    Seq(Vec<Cmd>),
}

pub const EDGE_EXE: &str = "msedge.exe";
pub const EDGE_RUN: &str = r"%ProgramFiles(x86)%\Microsoft\Edge\Application\msedge.exe";

/// Nomes de comandos aceitos (para o editor sugerir/validar).
pub const COMMANDS: &[(&str, &str)] = &[
    ("app", "app EXE [RUN] [ARGS] [title=..] [title-ends=..] [class=..] [prefer=..] [dir=..] [wait=SEG] [nomax]"),
    ("focus", "focus EXE [title=..] [class=..] — só foca, não abre"),
    ("run", "run CMD [ARGS] [dir=..]"),
    ("browser", "browser — abre/foca o Edge"),
    ("browser-tab", "browser-tab MATCH[|MATCH2] [BUSCA] [URL] [exe=..] [run=..]"),
    ("next-window", "next-window [EXE...] [class:CLS] [title:TXT] — pula essas"),
    ("type", "type TEXTO"),
    ("topmost", "topmost — alterna 'sempre no topo' da janela ativa"),
    ("maximize", "maximize"),
    ("minimize", "minimize"),
    ("close-window", "close-window"),
    ("sleep-pc", "sleep-pc — suspende o PC"),
    ("shutdown", "shutdown — desliga o PC"),
    ("lock", "lock — bloqueia a sessão"),
    ("notify", "notify TEXTO"),
    ("wait", "wait MS"),
    ("first", "first (cmd ...) (cmd ...) — o primeiro que funcionar"),
    ("seq", "seq (cmd ...) (cmd ...) — todos em ordem"),
];

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Atom(String),
    List(Vec<Item>),
}

impl Item {
    fn from_json(v: &Value) -> Result<Item, String> {
        match v {
            Value::String(s) => Ok(Item::Atom(s.clone())),
            Value::Array(a) => Ok(Item::List(a.iter().map(Item::from_json).collect::<Result<_, _>>()?)),
            Value::Number(n) => Ok(Item::Atom(n.to_string())),
            other => Err(format!("valor inesperado na mensagem: {other}")),
        }
    }
}

/// Converte a mensagem JSON do kanata num comando.
pub fn parse_json(v: &Value) -> Result<Cmd, String> {
    match Item::from_json(v)? {
        Item::List(items) => parse_items(&items),
        Item::Atom(a) => parse_items(&[Item::Atom(a)]),
    }
}

const OPTION_KEYS: &[&str] = &[
    "title", "title-ends", "class", "prefer", "dir", "wait", "exe", "run", "args", "search", "url", "max",
];

struct Parts {
    pos: Vec<String>,
    opts: Vec<(String, String)>,
    flags: Vec<String>,
    lists: Vec<Vec<Item>>,
}

impl Parts {
    fn opt(&self, k: &str) -> Option<String> {
        self.opts.iter().rev().find(|(key, _)| key == k).map(|(_, v)| v.clone())
    }
    fn flag(&self, f: &str) -> bool {
        self.flags.iter().any(|x| x == f)
    }
    fn filter(&self, exe: Option<String>) -> WinFilter {
        WinFilter {
            exe: exe.filter(|e| e != "*" && !e.is_empty()),
            title: self.opt("title"),
            title_ends: self.opt("title-ends"),
            class: self.opt("class"),
            prefer: self.opt("prefer"),
        }
    }
}

fn split(items: &[Item]) -> Result<Parts, String> {
    let mut p = Parts { pos: vec![], opts: vec![], flags: vec![], lists: vec![] };
    for it in items {
        match it {
            Item::List(l) => p.lists.push(l.clone()),
            Item::Atom(a) => {
                if let Some((k, v)) = a.split_once('=') {
                    if OPTION_KEYS.contains(&k) {
                        p.opts.push((k.to_string(), v.to_string()));
                        continue;
                    }
                }
                if a == "nomax" {
                    p.flags.push(a.clone());
                    continue;
                }
                p.pos.push(a.clone());
            }
        }
    }
    Ok(p)
}

fn wait_ms(p: &Parts) -> Result<u64, String> {
    match p.opt("wait") {
        None => Ok(5000),
        Some(s) => s
            .trim_end_matches('s')
            .parse::<f64>()
            .map(|x| (x * 1000.0) as u64)
            .map_err(|_| format!("wait={s} inválido (use segundos, ex.: wait=5)")),
    }
}

fn nonempty(s: Option<&String>) -> Option<String> {
    s.filter(|x| !x.is_empty()).cloned()
}

pub fn parse_items(items: &[Item]) -> Result<Cmd, String> {
    let (head, rest) = match items.split_first() {
        Some((Item::Atom(h), rest)) => (h.as_str(), rest),
        Some((Item::List(_), _)) => return Err("a mensagem deve começar com o nome do comando".into()),
        None => return Err("mensagem vazia".into()),
    };
    if head.contains(' ') && rest.is_empty() {
        return Err(format!(
            "\"{head}\": separe o comando dos argumentos, ex.: (push-msg app \"claude.exe\")"
        ));
    }
    let p = split(rest)?;
    let need_none = |name: &str| -> Result<(), String> {
        if !p.pos.is_empty() || !p.lists.is_empty() {
            Err(format!("{name} não recebe argumentos"))
        } else {
            Ok(())
        }
    };
    let cmd = match head {
        "app" | "focus" => {
            let exe = p.pos.first().cloned();
            if exe.is_none() && p.opt("title").is_none() && p.opt("class").is_none() {
                return Err(format!("{head}: informe o EXE (ex.: {head} \"claude.exe\")"));
            }
            let launch = head == "app";
            let run = nonempty(p.pos.get(1)).or_else(|| p.opt("run")).or_else(|| {
                if launch {
                    exe.clone().filter(|e| e != "*")
                } else {
                    None
                }
            });
            Cmd::App {
                filter: p.filter(exe),
                run,
                args: nonempty(p.pos.get(2)).or_else(|| p.opt("args")),
                dir: p.opt("dir"),
                maximize: !p.flag("nomax") && p.opt("max").as_deref() != Some("no"),
                launch,
                wait_ms: wait_ms(&p)?,
            }
        }
        "browser" => Cmd::App {
            filter: WinFilter {
                exe: Some(p.opt("exe").unwrap_or_else(|| EDGE_EXE.into())),
                title_ends: Some(p.opt("title-ends").unwrap_or_else(|| "Edge".into())),
                ..Default::default()
            },
            run: Some(p.opt("run").unwrap_or_else(|| EDGE_RUN.into())),
            args: nonempty(p.pos.first()),
            dir: None,
            maximize: !p.flag("nomax"),
            launch: true,
            wait_ms: 5000,
        },
        "run" => Cmd::Run {
            cmd: p.pos.first().cloned().ok_or("run: informe o que executar")?,
            args: nonempty(p.pos.get(1)).or_else(|| p.opt("args")),
            dir: p.opt("dir"),
        },
        "browser-tab" | "site" => {
            let m = p.pos.first().cloned().ok_or("browser-tab: informe o texto da URL (ex.: youtube.com)")?;
            let matches: Vec<String> = m.split('|').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let search = nonempty(p.pos.get(1)).or_else(|| p.opt("search")).unwrap_or_else(|| matches[0].clone());
            let url = nonempty(p.pos.get(2)).or_else(|| p.opt("url")).unwrap_or_else(|| {
                if search.contains("://") {
                    search.clone()
                } else {
                    format!("https://{search}")
                }
            });
            Cmd::BrowserTab {
                matches,
                search,
                url,
                filter: WinFilter {
                    exe: Some(p.opt("exe").unwrap_or_else(|| EDGE_EXE.into())),
                    title_ends: Some(p.opt("title-ends").unwrap_or_else(|| "Edge".into())),
                    ..Default::default()
                },
                run: p.opt("run").unwrap_or_else(|| EDGE_RUN.into()),
            }
        }
        "next-window" => {
            let skip = p
                .pos
                .iter()
                .map(|s| {
                    if let Some(c) = s.strip_prefix("class:") {
                        WinFilter { class: Some(c.into()), ..Default::default() }
                    } else if let Some(t) = s.strip_prefix("title:") {
                        WinFilter { title: Some(t.into()), ..Default::default() }
                    } else if let Some((exe, t)) = s.split_once("+title:") {
                        WinFilter { exe: Some(exe.into()), title: Some(t.into()), ..Default::default() }
                    } else {
                        WinFilter { exe: Some(s.clone()), ..Default::default() }
                    }
                })
                .collect();
            Cmd::NextWindow { skip }
        }
        "type" => Cmd::Type(p.pos.join(" ")),
        "notify" => Cmd::Notify(p.pos.join(" ")),
        "wait" => {
            let s = p.pos.first().ok_or("wait: informe os milissegundos")?;
            Cmd::Wait(s.trim_end_matches("ms").parse().map_err(|_| format!("wait: '{s}' não é número"))?)
        }
        "topmost" => need_none(head).map(|_| Cmd::Topmost)?,
        "maximize" => need_none(head).map(|_| Cmd::Maximize)?,
        "minimize" => need_none(head).map(|_| Cmd::Minimize)?,
        "close-window" => need_none(head).map(|_| Cmd::CloseWindow)?,
        "sleep-pc" => need_none(head).map(|_| Cmd::SleepPc)?,
        "shutdown" => need_none(head).map(|_| Cmd::Shutdown)?,
        "lock" => need_none(head).map(|_| Cmd::Lock)?,
        "first" | "seq" => {
            if p.lists.is_empty() {
                return Err(format!("{head}: coloque os comandos entre parênteses, ex.: ({head} (focus \"a.exe\") (app \"b.exe\"))"));
            }
            let subs = p.lists.iter().map(|l| parse_items(l)).collect::<Result<Vec<_>, _>>()?;
            if head == "first" {
                Cmd::First(subs)
            } else {
                Cmd::Seq(subs)
            }
        }
        other => {
            return Err(format!(
                "comando desconhecido \"{other}\". Comandos: {}",
                COMMANDS.iter().map(|c| c.0).collect::<Vec<_>>().join(", ")
            ))
        }
    };
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn app_basic() {
        let c = parse_json(&json!(["app", "claude.exe", "shell:AppsFolder\\Claude!Claude"])).unwrap();
        match c {
            Cmd::App { filter, run, maximize, launch, .. } => {
                assert_eq!(filter.exe.as_deref(), Some("claude.exe"));
                assert_eq!(run.as_deref(), Some("shell:AppsFolder\\Claude!Claude"));
                assert!(maximize && launch);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn app_defaults_run_to_exe_and_flags() {
        let c = parse_json(&json!(["app", "obs64.exe", "nomax", "prefer=("])).unwrap();
        match c {
            Cmd::App { run, maximize, filter, .. } => {
                assert_eq!(run.as_deref(), Some("obs64.exe"));
                assert!(!maximize);
                assert_eq!(filter.prefer.as_deref(), Some("("));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn focus_does_not_launch() {
        match parse_json(&json!(["focus", "wezterm-gui.exe"])).unwrap() {
            Cmd::App { launch, run, .. } => {
                assert!(!launch);
                assert!(run.is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn first_nested() {
        let c = parse_json(&json!(["first", ["focus", "a.exe"], ["app", "b.exe"]])).unwrap();
        match c {
            Cmd::First(v) => assert_eq!(v.len(), 2),
            _ => panic!(),
        }
    }

    #[test]
    fn browser_tab_defaults() {
        match parse_json(&json!(["browser-tab", "chatgpt.com|chat.openai.com"])).unwrap() {
            Cmd::BrowserTab { matches, search, url, .. } => {
                assert_eq!(matches, vec!["chatgpt.com", "chat.openai.com"]);
                assert_eq!(search, "chatgpt.com");
                assert_eq!(url, "https://chatgpt.com");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn option_like_paths_are_positional() {
        // "C:\a=b" não é opção conhecida -> continua posicional
        match parse_json(&json!(["run", "C:\\a=b.exe"])).unwrap() {
            Cmd::Run { cmd, .. } => assert_eq!(cmd, "C:\\a=b.exe"),
            _ => panic!(),
        }
    }

    #[test]
    fn errors_are_helpful() {
        assert!(parse_json(&json!(["app claude.exe"])).unwrap_err().contains("separe"));
        assert!(parse_json(&json!(["voar"])).unwrap_err().contains("desconhecido"));
        assert!(parse_json(&json!(["topmost", "x"])).is_err());
        assert!(parse_json(&json!(["first", "x"])).is_err());
    }

    #[test]
    fn next_window_skips() {
        match parse_json(&json!(["next-window", "Telegram.exe", "class:CabinetWClass", "title:Notepads"])).unwrap() {
            Cmd::NextWindow { skip } => {
                assert_eq!(skip.len(), 3);
                assert_eq!(skip[1].class.as_deref(), Some("CabinetWClass"));
            }
            _ => panic!(),
        }
    }
}
