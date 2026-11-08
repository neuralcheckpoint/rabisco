//! QA 08/11: matar o processo da gravação (`--gravar`) no meio não pode deixar
//! `.part` nem filho órfão. Usa o `--perf-memoria` como pai (desktop próprio,
//! vídeo em %TEMP%\rabisco-perf-video-<pid>; a cópia do Rafa não é tocada).
//! `cargo test --release --test qa_kill -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn ps(script: &str) -> String {
    let o = Command::new("powershell").args(["-NoProfile", "-Command", script]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn partes(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            partes(&p, out);
        } else if p.extension().is_some_and(|x| x == "part") {
            out.push(p);
        }
    }
}

#[test]
#[ignore]
fn matar_o_filho_da_gravacao_nao_deixa_part_nem_orfao() {
    let saida = std::env::temp_dir().join(format!("qa-kill-{}.txt", std::process::id()));
    let mut pai = Command::new(env!("CARGO_BIN_EXE_rabisco")).arg("--perf-memoria").arg(&saida).spawn().unwrap();
    let pid = pai.id();
    // O primeiro filho `--gravar` do pai.
    let consulta = format!("(Get-CimInstance Win32_Process -Filter \"ParentProcessId={pid} and Name='rabisco.exe'\" | Where-Object {{ $_.CommandLine -match '--gravar ' }} | Select-Object -First 1).ProcessId");
    let t = Instant::now();
    let filho = loop {
        let r = ps(&consulta);
        if let Ok(f) = r.parse::<u32>() {
            break f;
        }
        assert!(t.elapsed() < Duration::from_secs(30), "o filho --gravar não apareceu");
        std::thread::sleep(Duration::from_millis(100));
    };
    std::thread::sleep(Duration::from_millis(800)); // já gravando
    Command::new("taskkill").args(["/PID", &filho.to_string(), "/F"]).output().unwrap();
    let t = Instant::now();
    while pai.try_wait().unwrap().is_none() {
        if t.elapsed() > Duration::from_secs(90) {
            let _ = pai.kill();
            panic!("o app não terminou depois de perder o filho");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    // Filhos `--gravar` que sobraram (o pid do pai continua no ParentProcessId).
    let orfaos = ps(&format!("(Get-CimInstance Win32_Process -Filter \"ParentProcessId={pid} and Name='rabisco.exe'\" | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force; $_.ProcessId }}) -join ','"));
    std::thread::sleep(Duration::from_millis(500));
    let dir = std::env::temp_dir().join(format!("rabisco-perf-video-{pid}"));
    let mut sobras = Vec::new();
    partes(&dir, &mut sobras);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&saida);
    assert!(sobras.is_empty(), "sobrou .part depois de matar o filho: {sobras:?}");
    assert!(orfaos.is_empty(), "sobrou filho --gravar órfão gravando sem ninguém para pará-lo: pid {orfaos}");
}

/// O arquivo de `dir` (recursivo) com a extensão `ext`.
fn com_ext(dir: &Path, ext: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let achou = if p.is_dir() { com_ext(&p, ext) } else { p.extension().is_some_and(|x| x == ext).then_some(p) };
        if achou.is_some() {
            return achou;
        }
    }
    None
}

#[test]
#[ignore]
fn matar_o_pai_fecha_o_mp4_e_o_filho_sai() {
    let saida = std::env::temp_dir().join(format!("qa-kill-pai-{}.txt", std::process::id()));
    let mut pai = Command::new(env!("CARGO_BIN_EXE_rabisco")).arg("--perf-memoria").arg(&saida).spawn().unwrap();
    let pid = pai.id();
    let dir = std::env::temp_dir().join(format!("rabisco-perf-video-{pid}"));
    let consulta = format!("(Get-CimInstance Win32_Process -Filter \"ParentProcessId={pid} and Name='rabisco.exe'\" | Where-Object {{ $_.CommandLine -match '--gravar ' }} | Select-Object -First 1).ProcessId");
    let t = Instant::now();
    let filho = loop {
        if let Ok(f) = ps(&consulta).parse::<u32>() {
            break f;
        }
        assert!(t.elapsed() < Duration::from_secs(30), "o filho --gravar não apareceu");
        std::thread::sleep(Duration::from_millis(100));
    };
    // Mata o pai quando o .part já tem quadros (antes de ele mesmo parar, 2 s depois do primeiro).
    let t = Instant::now();
    while !com_ext(&dir, "part").is_some_and(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 0)) {
        assert!(t.elapsed() < Duration::from_secs(30), "a gravação não começou");
        std::thread::sleep(Duration::from_millis(50));
    }
    Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).output().unwrap();
    let _ = pai.wait();
    let t = Instant::now();
    let vivo = loop {
        let vivo = !ps(&format!("Get-Process -Id {filho} -ErrorAction SilentlyContinue")).is_empty();
        if !vivo || t.elapsed() > Duration::from_secs(20) {
            break vivo;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    if vivo {
        let _ = Command::new("taskkill").args(["/PID", &filho.to_string(), "/F"]).output();
    }
    std::thread::sleep(Duration::from_millis(300));
    let (mp4, part) = (com_ext(&dir, "mp4"), com_ext(&dir, "part"));
    let tamanho = mp4.as_ref().and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&saida);
    assert!(!vivo, "o filho seguiu gravando depois que o pai morreu");
    assert!(part.is_none(), "sobrou .part: {part:?}");
    assert!(tamanho > 0, "o filho não fechou o MP4: {mp4:?}");
}

#[test]
#[ignore]
fn parar_antes_do_primeiro_quadro_cancela_calado() {
    use windows_sys::Win32::System::Threading::CreateEventW;
    let nome = format!("Local\\RabiscoTesteParar{}", std::process::id());
    let w: Vec<u16> = nome.encode_utf16().chain(Some(0)).collect();
    // Já sinalizado: o pedido de parar chega antes do primeiro quadro.
    let ev = unsafe { CreateEventW(core::ptr::null(), 1, 1, w.as_ptr()) };
    assert!(!ev.is_null());
    let dir = std::env::temp_dir().join(format!("qa-cancela-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fin = dir.join("rabisco_teste.mp4");
    // O .part que o pai reserva.
    std::fs::write(dir.join("rabisco_teste.mp4.part"), b"").unwrap();
    let st = Command::new(env!("CARGO_BIN_EXE_rabisco")).args(["--gravar", "0", &nome, "monitor", "-"]).arg(&fin).arg(std::process::id().to_string()).status().unwrap();
    let sobras: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(st.code(), Some(2), "não saiu com o código de cancelado");
    assert!(sobras.is_empty(), "sobrou arquivo: {sobras:?}");
}
