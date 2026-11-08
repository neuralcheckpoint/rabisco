//! Suíte de performance relativa: mede os tempos deste código e os de outro
//! commit na mesma hora, alternados, e reprova se algum ficar mais de 15% pior.
//! Serve para a máquina carregada (outro job rodando), onde os limites
//! absolutos de perf/orcamento.toml estouram até no commit antigo.
//!
//! PowerShell: `$env:PERF_BASE="4200123"; cargo test --release --test perf_base -- --ignored --nocapture`
//! bash:       `PERF_BASE=4200123 cargo test --release --test perf_base -- --ignored --nocapture`
//!
//! O commit-base sai por `git archive` numa pasta temporária (fica para a
//! próxima vez) e cada lado compila no próprio target: nada no repo muda.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Quanto o atual pode ficar pior que a base.
const FOLGA: f64 = 1.15;
/// Rodadas de cada lado, em ordem alternada; vale a melhor mediana de cada métrica.
const RODADAS: usize = 4;

fn git(args: &[&str]) -> Command {
    let mut c = Command::new("git");
    c.args(["-c", "safe.directory=*"]).args(args).current_dir(env!("CARGO_MANIFEST_DIR"));
    c
}

/// Pasta com o código do commit (por git archive), criada uma vez.
fn base(commit: &str) -> PathBuf {
    let o = git(&["rev-parse", "--short", commit]).output().expect("git");
    let id = String::from_utf8_lossy(&o.stdout).trim().to_string();
    assert!(o.status.success() && !id.is_empty(), "commit {commit} não existe");
    let dir = std::env::temp_dir().join(format!("rabisco-base-{id}"));
    if !dir.join("Cargo.toml").exists() {
        std::fs::create_dir_all(&dir).unwrap();
        let tar = dir.join("base.tar");
        assert!(git(&["archive", "--format=tar", "-o"]).arg(&tar).arg(&id).status().unwrap().success(), "git archive");
        assert!(Command::new("tar").arg("-xf").arg(&tar).arg("-C").arg(&dir).status().unwrap().success(), "tar");
        let _ = std::fs::remove_file(&tar);
    }
    dir
}

/// Roda a suíte de tempos em `dir` (no próprio target) e lê as medianas do perf/ultimo.json.
/// O resultado do teste (limites absolutos) não importa aqui: só os números.
fn tempos(dir: &Path, target: &Path) -> BTreeMap<String, f64> {
    let st = Command::new(env!("CARGO"))
        .args(["test", "--release", "--bin", "rabisco", "perf_tempos", "--target-dir"])
        .arg(target)
        .args(["--", "--ignored", "--test-threads=1"])
        .current_dir(dir)
        .output()
        .expect("cargo");
    let j = std::fs::read_to_string(dir.join("perf").join("ultimo.json")).unwrap_or_default();
    let mut m = BTreeMap::new();
    let Some(t) = j.split("\"tempos\"").nth(1).and_then(|t| t.split("\"alocacoes\"").next()) else {
        panic!("sem tempos em {}: {}", dir.display(), String::from_utf8_lossy(&st.stderr).lines().last().unwrap_or(""));
    };
    for l in t.lines() {
        let Some((nome, resto)) = l.trim().strip_prefix('"').and_then(|l| l.split_once('"')) else { continue };
        let Some(v) = resto.split("\"mediana\": ").nth(1).and_then(|v| v.split(',').next()?.parse().ok()) else { continue };
        m.insert(nome.to_string(), v);
    }
    m
}

#[test]
#[ignore]
fn perf_relativo() {
    // Sem o commit-base não há o que comparar: a suíte absoluta (o mesmo filtro "perf") segue sozinha.
    let Ok(commit) = std::env::var("PERF_BASE") else {
        println!("perf_relativo: sem PERF_BASE=<commit>, nada a comparar (ver docs/src/06-performance.md)");
        return;
    };
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"));
    let b = base(&commit);
    let (tb, ta) = (b.join("target"), raiz.join("target").join("perf-atual"));
    let melhor = |m: &mut BTreeMap<String, f64>, novo: BTreeMap<String, f64>| {
        for (k, v) in novo {
            let e = m.entry(k).or_insert(v);
            *e = e.min(v);
        }
    };
    let (mut mb, mut ma) = (BTreeMap::new(), BTreeMap::new());
    for r in 0..RODADAS {
        // A ordem troca a cada rodada: quem roda primeiro pega outra máquina
        // (na 1ª medida, sempre a base antes, até código igual saiu 15-70% "pior").
        if r % 2 == 0 {
            println!("rodada {}: base {commit}, depois o atual", r + 1);
            melhor(&mut mb, tempos(&b, &tb));
            melhor(&mut ma, tempos(raiz, &ta));
        } else {
            println!("rodada {}: o atual, depois a base {commit}", r + 1);
            melhor(&mut ma, tempos(raiz, &ta));
            melhor(&mut mb, tempos(&b, &tb));
        }
    }
    let mut falhas = Vec::new();
    println!("\n{:<20} {:>10} {:>10} {:>8}", "métrica", "base", "atual", "atual/base");
    for (k, &a) in &ma {
        let Some(&v) = mb.get(k) else {
            println!("{k:<20} {:>10} {a:>10.3} {:>8}", "-", "(nova)");
            continue;
        };
        let q = a / v;
        let mark = if q > FOLGA { "  PIOROU" } else { "" };
        println!("{k:<20} {v:>10.3} {a:>10.3} {q:>8.2}{mark}");
        if q > FOLGA {
            falhas.push(format!("{k}: {a:.3} contra {v:.3} da base ({:+.0}%)", (q - 1.0) * 100.0));
        }
    }
    assert!(!ma.is_empty(), "o atual não mediu nada");
    assert!(falhas.is_empty(), "mais de {:.0}% pior que {commit}: {falhas:?}", (FOLGA - 1.0) * 100.0);
}
