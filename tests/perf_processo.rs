//! Parte da suíte de performance que precisa do exe: o tamanho dele e a
//! memória do processo de verdade (`rabisco.exe --perf-memoria`, num desktop
//! próprio e invisível; a cópia aberta do Rafa não é tocada).
//! `cargo test --release perf -- --ignored --test-threads=1 --nocapture`

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn pasta() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("perf")
}

/// `chave = número` do orçamento (o mesmo formato que src/perf.rs lê).
fn limite(k: &str) -> Option<f64> {
    let o = std::fs::read_to_string(pasta().join("orcamento.toml")).expect("perf/orcamento.toml");
    o.lines().map(|l| l.split('#').next().unwrap()).find_map(|l| {
        let (c, v) = l.split_once('=')?;
        (c.trim() == k).then(|| v.trim().replace('_', "").parse().ok())?
    })
}

/// Acrescenta a seção "processo" ao perf/ultimo.json (troca a de antes).
fn grava(secao: &str) {
    let p = pasta().join("ultimo.json");
    let j = std::fs::read_to_string(&p).unwrap_or_else(|_| "{\n}\n".into());
    let j = match j.find(",\n  \"processo\"") {
        Some(i) => format!("{}\n}}\n", &j[..i]),
        None => j,
    };
    let fim = j.trim_end().strip_suffix('}').unwrap().trim_end();
    let sep = if fim.ends_with('{') { "" } else { "," };
    std::fs::write(p, format!("{fim}{sep}\n  \"processo\": {secao}\n}}\n")).unwrap();
}

/// Canto do primeiro monitor com 3840x2160 (pixels físicos).
fn monitor_4k() -> Option<(i32, i32)> {
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
    use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
    unsafe extern "system" fn cada(_: HMONITOR, _: HDC, r: *mut RECT, achou: LPARAM) -> windows_sys::core::BOOL {
        let r = unsafe { &*r };
        let achou = unsafe { &mut *(achou as *mut Option<(i32, i32)>) };
        if achou.is_none() && r.right - r.left == 3840 && r.bottom - r.top == 2160 {
            *achou = Some((r.left, r.top));
        }
        1
    }
    let mut achou = None;
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        EnumDisplayMonitors(core::ptr::null_mut(), core::ptr::null(), Some(cada), &mut achou as *mut _ as LPARAM);
    }
    achou
}

#[test]
#[ignore]
fn perf_processo() {
    let mut falhas = Vec::new();
    let exe = env!("CARGO_BIN_EXE_rabisco");
    let bytes = std::fs::metadata(exe).unwrap().len();
    let teto = limite("exe_bytes").expect("exe_bytes no orçamento");
    println!("\nexe {bytes} bytes (teto {teto})");
    if cfg!(debug_assertions) {
        falhas.push("rode com --release (o tamanho é o do exe de release)".to_string());
    } else if bytes as f64 > teto {
        falhas.push(format!("exe_bytes: {bytes} > {teto}"));
    }

    let saida = std::env::temp_dir().join(format!("rabisco-perf-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&saida);
    let mut filho = std::process::Command::new(exe).arg("--perf-memoria").arg(&saida).spawn().expect("abrir o exe");
    let t = Instant::now();
    while filho.try_wait().unwrap().is_none() {
        if t.elapsed() > Duration::from_secs(60) {
            let _ = filho.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let texto = std::fs::read_to_string(&saida).unwrap_or_default();
    let _ = std::fs::remove_file(&saida);
    let mut estados = Vec::new();
    println!("{:<20} {:>12} {:>12} {:>10}", "estado", "working set", "privado", "limite");
    for l in texto.lines() {
        let c: Vec<&str> = l.split(' ').collect();
        let [nome, ws, pv] = c[..] else { continue };
        let (ws, pv): (u64, u64) = (ws.parse().unwrap(), pv.parse().unwrap());
        let k = format!("memoria_{nome}_privado_kb");
        let lim = limite(&k);
        if let Some(m) = lim
            && pv as f64 / 1024.0 > m
        {
            falhas.push(format!("{k}: {} > {m}", pv / 1024));
        }
        println!("{nome:<20} {:>9} KB {:>9} KB {:>10}", ws / 1024, pv / 1024, lim.map_or("-".into(), |m| format!("{m} KB")));
        estados.push(format!("      \"{nome}\": {{\"working_set_kb\": {}, \"privado_kb\": {}}}", ws / 1024, pv / 1024));
    }
    // Gravar não pode deixar este processo maior: volta a ≤ 1,1× do parado.
    let privado = |n: &str| texto.lines().find_map(|l| l.strip_prefix(&format!("{n} "))?.split(' ').nth(1)?.parse::<f64>().ok());
    match (privado("mouse_parado"), privado("gravou")) {
        (Some(p), Some(g)) if g > p * 1.1 => falhas.push(format!("gravou: {} KB > 1,1 x {} KB (parado)", g as u64 / 1024, p as u64 / 1024)),
        (Some(_), Some(_)) => {}
        _ => falhas.push("a gravação do --perf-memoria falhou (sem \"gravou\")".to_string()),
    }
    for nome in texto.lines().filter_map(|l| l.split(' ').next()).filter(|n| n.ends_with("_falhou")) {
        falhas.push(format!("o --perf-memoria falhou em {nome}"));
    }
    if estados.is_empty() {
        falhas.push("o --perf-memoria não mediu nada (sem desktop próprio ou sem janela?)".to_string());
    }
    // Gravação: 30 s de 1080p30 e de 4K30 pelo --gravar-teste (o mesmo motor
    // do processo filho). CPU = tempo de CPU do processo / tempo de relógio.
    let mut gravs = Vec::new();
    println!("\n{:<8} {:>11} {:>8} {:>13} {:>10} {:>9} {:>9}", "gravação", "tamanho", "CPU", "privado pico", "perdidos", "início", "parada");
    // As duas regiões no monitor 4K (o monitor em (0, 0) muda com o boot).
    let (mx, my) = monitor_4k().unwrap_or_else(|| {
        falhas.push("sem monitor 4K para a gravação de 4K30".to_string());
        (0, 0)
    });
    // A de som: 10 s de 1080p com o som do sistema (loopback + AAC no mesmo processo).
    for (nome, w, h, segundos, som) in [("1080p", 1920, 1080, "30", "-"), ("4k", 3840, 2160, "30", "-"), ("1080p_som", 1920, 1080, "10", "sis")] {
        let modo = format!("regiao:{mx},{my},{w},{h}");
        let modo = modo.as_str();
        let mp4 = std::env::temp_dir().join(format!("rabisco-perf-{}-{nome}.mp4", std::process::id()));
        let txt = mp4.with_extension("mp4.txt");
        let ok = std::process::Command::new(exe).args(["--gravar-teste", modo, segundos]).arg(&mp4).args(["30", som]).status().is_ok_and(|s| s.success());
        let t = std::fs::read_to_string(&txt).unwrap_or_default();
        let _ = (std::fs::remove_file(&mp4), std::fs::remove_file(&txt));
        let v = |k: &str| t.lines().find_map(|l| l.strip_prefix(&format!("{k} "))?.parse::<f64>().ok()).unwrap_or(f64::NAN);
        if !ok {
            falhas.push(format!("gravar {nome}: {}", t.lines().find(|l| l.starts_with("erro")).unwrap_or("não gravou")));
            continue;
        }
        let cpu = v("cpu_us") / v("relogio_us") * 100.0;
        let pico = v("privado_pico") / 1024.0;
        let perdidos = v("perdidos") / (v("quadros") + v("perdidos")) * 100.0;
        let (inicio, parada) = (v("inicio_us") / 1000.0, v("parada_us") / 1000.0);
        println!("{nome:<8} {:>5}x{:<5} {cpu:>7.1}% {pico:>10.0} KB {perdidos:>9.1}% {inicio:>6.0} ms {parada:>6.0} ms", v("w"), v("h"));
        let mut metricas = vec![(format!("gravar_{nome}_cpu_pct"), cpu), (format!("gravar_{nome}_privado_pico_kb"), pico), ("gravar_perdidos_pct".into(), perdidos), ("gravar_inicio_ms".into(), inicio), ("gravar_parada_ms".into(), parada)];
        let desvio = v("som_desvio_us") / 1000.0;
        if som != "-" {
            // Maior distância entre a hora de um pacote de som (QPC) e a fila do áudio.
            println!("{nome:<8} som: {} amostras, desvio {desvio:.1} ms ({})", v("som"), t.lines().find(|l| l.starts_with("notas")).unwrap_or(""));
            metricas.push(("gravar_som_desvio_ms".into(), desvio));
            if !(v("som") >= 0.95 * 48_000.0 * 10.0) {
                falhas.push(format!("gravar {nome}: sem a faixa de som ({} amostras)", v("som")));
            }
        }
        for (k, m) in metricas {
            // NaN (faltou a linha) também reprova.
            if let Some(l) = limite(&k)
                && !(m <= l)
            {
                falhas.push(format!("{k} ({nome}): {m:.1} > {l}"));
            }
        }
        gravs.push(format!("      \"{nome}\": {{\"cpu_pct\": {cpu:.2}, \"privado_pico_kb\": {pico:.0}, \"perdidos_pct\": {perdidos:.2}, \"inicio_ms\": {inicio:.0}, \"parada_ms\": {parada:.0}, \"som_desvio_ms\": {desvio:.1}}}"));
    }
    grava(&format!("{{\n    \"exe_bytes\": {bytes},\n    \"memoria\": {{\n{}\n    }},\n    \"gravacao\": {{\n{}\n    }}\n  }}", estados.join(",\n"), gravs.join(",\n")));
    assert!(falhas.is_empty(), "passou do orçamento: {falhas:?}");
}
