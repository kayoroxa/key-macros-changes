//! Atualização pelo GitHub (o clone local do projeto contra `origin/master`).
//!
//! - `check()`: faz `git fetch` e vê se há commits novos no GitHub.
//! - O daemon checa ao iniciar (inclusive no início do Windows) e, se houver
//!   atualização, abre o editor já com o pop-up. O editor também checa ao abrir.
//! - "Atualizar agora": `git merge --ff-only` + `cargo build --release`, copia o
//!   build para `dist\keymacros-nova.exe`, fecha o app e roda `aplicar-atualizacao`,
//!   que troca `dist\keymacros.exe` e reabre o app sozinho.

use crate::{daemon, paths};
use std::fs::File;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

const BRANCH: &str = "master";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// PROCESS_SYNCHRONIZE: permite esperar o processo terminar
const PROCESS_SYNCHRONIZE: u32 = 0x0010_0000;

/// Passos do trabalho de atualizar, enviados para a janela do editor.
pub enum Passo {
    Texto(String),
    Pronto(Result<(), String>),
}

fn upstream() -> String {
    format!("origin/{BRANCH}")
}

/// Pasta do projeto: sobe a partir do exe até achar o Cargo.toml do keymacros.
pub fn project_root() -> Option<PathBuf> {
    let mut d = std::env::current_exe().ok()?.parent()?.to_path_buf();
    loop {
        let toml = d.join("Cargo.toml");
        if toml.is_file() && std::fs::read_to_string(&toml).map(|t| t.contains("name = \"keymacros\"")).unwrap_or(false) {
            return Some(d);
        }
        d = d.parent()?.to_path_buf();
    }
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("não consegui rodar o git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "o git falhou".into() } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `Ok(Some(mensagens))` = há commits novos no GitHub (mais recentes primeiro).
/// `Ok(None)` = já está atualizado. `Err` = não deu para checar (sem rede, sem git...).
pub fn check() -> Result<Option<Vec<String>>, String> {
    let root = project_root().ok_or("não achei a pasta do projeto (Cargo.toml)")?;
    git(&root, &["fetch", "--quiet", "origin", BRANCH])?;
    let range = format!("HEAD..{}", upstream());
    let n = git(&root, &["rev-list", "--count", range.as_str()])?;
    if n.trim() == "0" {
        return Ok(None);
    }
    let log = git(&root, &["log", "--format=%s", range.as_str()])?;
    Ok(Some(log.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()))
}

/// `check()` em outra thread, para a janela não travar.
pub fn check_in_background() -> Receiver<Result<Option<Vec<String>>, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(check());
    });
    rx
}

/// Roda a atualização inteira em outra thread; os passos chegam pelo canal.
pub fn atualizar_in_background(cfg: PathBuf) -> Receiver<Passo> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let r = atualizar(&cfg, &tx);
        let _ = tx.send(Passo::Pronto(r));
    });
    rx
}

fn atualizar(cfg: &Path, tx: &Sender<Passo>) -> Result<(), String> {
    let texto = |t: &str| {
        let _ = tx.send(Passo::Texto(t.into()));
    };
    let root = project_root().ok_or("não achei a pasta do projeto (Cargo.toml)")?;
    let dist = root.join("dist");
    let log = dist.join("keymacros-atualizacao.log");

    texto("Baixando a nova versão do GitHub…");
    let up = upstream();
    git(&root, &["merge", "--ff-only", up.as_str()]).map_err(|e| {
        format!("não consegui baixar a atualização ({e}). Se você mexeu nos arquivos do código, guarde ou faça commit dessas mudanças e tente de novo.")
    })?;

    texto("Compilando (pode levar alguns minutos)…");
    std::fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    let out = File::create(&log).map_err(|e| e.to_string())?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let ok = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("não consegui rodar o cargo: {e}"))?
        .success();
    if !ok {
        return Err(format!("o build falhou. Veja o log em {}", log.display()));
    }

    texto("Trocando a versão e reiniciando…");
    let nova = dist.join("keymacros-nova.exe");
    std::fs::copy(root.join("target/release/keymacros.exe"), &nova).map_err(|e| format!("não consegui copiar o build: {e}"))?;
    let _ = std::fs::copy(root.join("README.md"), dist.join("README.md"));
    let _ = std::fs::copy(root.join("config/exemplo.kbd"), dist.join("exemplo.kbd"));

    // fecha o daemon (e com ele o kanata) antes de o novo exe tomar o lugar do antigo
    let h = daemon::find_daemon();
    if !h.is_null() {
        unsafe { PostMessageW(h, WM_CLOSE, 0, 0) };
        let t = Instant::now();
        while !daemon::find_daemon().is_null() && t.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // o editor que está nesta janela vai fechar logo depois: o novo exe espera ele sair
    Command::new(&nova)
        .arg("aplicar-atualizacao")
        .arg("--esperar")
        .arg(std::process::id().to_string())
        .arg("--destino")
        .arg(dist.join("keymacros.exe"))
        .arg("--cfg")
        .arg(cfg)
        .current_dir(&dist)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("não consegui reiniciar o app: {e}"))?;
    Ok(())
}

/// Modo chamado pelo novo exe (`keymacros-nova.exe aplicar-atualizacao ...`):
/// espera o editor antigo sair, copia este exe para `--destino` e abre o app de novo.
pub fn aplicar(rest: &[String], cfg: &Path) -> i32 {
    let mut pid: u32 = 0;
    let mut destino: Option<PathBuf> = None;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--esperar" => pid = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--destino" => destino = it.next().map(PathBuf::from),
            _ => {}
        }
    }
    let Some(destino) = destino else {
        eprintln!("uso: aplicar-atualizacao --destino CAMINHO [--esperar PID]");
        return 2;
    };
    if pid != 0 {
        esperar_processo(pid, 30_000);
    }
    let Ok(origem) = std::env::current_exe() else { return 1 };
    let t = Instant::now();
    loop {
        match std::fs::copy(&origem, &destino) {
            Ok(_) => break,
            Err(_) if t.elapsed() < Duration::from_secs(15) => std::thread::sleep(Duration::from_millis(300)),
            Err(_) => return 1,
        }
    }
    let dir = destino.parent().map(Path::to_path_buf).unwrap_or_else(paths::exe_dir);
    let _ = Command::new(&destino).arg("--cfg").arg(cfg).current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    let _ = Command::new(&destino).arg("editor").arg("--cfg").arg(cfg).current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    0
}

fn esperar_processo(pid: u32, ms: u32) {
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return;
        }
        WaitForSingleObject(h, ms);
        CloseHandle(h);
    }
}
