//! Suíte de performance: tempos (mediana e p95) e bytes alocados nos caminhos
//! quentes, contra o orçamento em `perf/orcamento.toml`.
//! `cargo test --release perf -- --ignored --test-threads=1 --nocapture`
//! (a parte do processo, memória e tamanho do exe, fica em `tests/perf_processo.rs`).

use crate::canvas::{self, Canvas, FADE_FRAME_MS, FADE_HOLD_MS, FADE_MS, FLOOR, Fading, Pt, Stroke, Tool};
use crate::{foco, png, texto};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::Instant;

// ---------- alocador contador (só no binário de teste) ----------

struct Contador;

thread_local! {
    // Por thread: os outros testes rodando ao lado não entram na conta.
    static BYTES: Cell<u64> = const { Cell::new(0) };
    static VEZES: Cell<u64> = const { Cell::new(0) };
}

fn conta(n: usize) {
    let _ = BYTES.try_with(|b| b.set(b.get() + n as u64));
    let _ = VEZES.try_with(|v| v.set(v.get() + 1));
}

unsafe impl GlobalAlloc for Contador {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        conta(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        conta(l.size());
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        conta(n);
        unsafe { System.realloc(p, l, n) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static ALOCADOR: Contador = Contador;

/// (bytes, alocações) pedidos por `f` nesta thread.
fn alocado(f: impl FnOnce()) -> (u64, u64) {
    let (b0, v0) = (BYTES.with(Cell::get), VEZES.with(Cell::get));
    f();
    (BYTES.with(Cell::get) - b0, VEZES.with(Cell::get) - v0)
}

// ---------- medição ----------

struct Linha {
    id: &'static str,
    unidade: &'static str,
    mediana: f64,
    p95: f64,
    n: usize,
}

/// Ordena as amostras (em µs) e tira mediana e p95.
fn resumo(id: &'static str, unidade: &'static str, mut us: Vec<f64>) -> Linha {
    us.sort_by(f64::total_cmp);
    let k = |q: f64| us[((us.len() - 1) as f64 * q).round() as usize];
    let div = if unidade == "ms" { 1000.0 } else { 1.0 };
    Linha { id, unidade, mediana: k(0.5) / div, p95: k(0.95) / div, n: us.len() }
}

/// `aquece` rodadas jogadas fora, depois `n` medidas de `f` (µs).
fn tempos(aquece: usize, n: usize, mut f: impl FnMut()) -> Vec<f64> {
    for _ in 0..aquece {
        f();
    }
    (0..n)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1e6
        })
        .collect()
}

// ---------- cenários (tela do Rafa: 6400x2160 físicos) ----------

const W: i32 = 6400;
const H: i32 = 2160;

fn tela_com_200() -> (Canvas, Vec<u32>) {
    let mut c = Canvas::new(W, H);
    c.strokes = canvas::sample_strokes(200);
    let mut out = vec![0u32; c.len()];
    c.rebuild(&mut out, FLOOR);
    (c, out)
}

fn traco_do_fade() -> Fading {
    let pts: Vec<Pt> = (0..60).map(|k| [800.0 + k as f32 * 15.0, 900.0 + (k as f32 / 5.0).sin() * 60.0, 4.0]).collect();
    Fading { stroke: Stroke { tool: Tool::Pen, rgb: 0x00FF00, alpha: 255, pts, num: 0, text: None, area: Default::default() }, born: 0, k: 255 }
}

/// Uma volta inteira do fade (todos os quadros), como o timer faz.
fn volta_do_fade(c: &mut Canvas, out: &mut [u32], mut cada: impl FnMut(&mut dyn FnMut())) {
    c.fading = vec![traco_do_fade()];
    // Até o último quadro, o que tira o traço da lista (e solta os blocos).
    let frames = FADE_MS.div_ceil(FADE_FRAME_MS);
    for f in 0..=frames {
        let now = (FADE_HOLD_MS + f * FADE_FRAME_MS).min(FADE_HOLD_MS + FADE_MS);
        cada(&mut || quadro_do_fade(c, out, now));
    }
}

/// Imagem parecida com uma tela real: fundo chapado, janelas, texto (ruído em
/// faixas) e uma foto (ruído em bloco: o pior caso).
/// Traço do teste com personas (QA da 0.3.0): onda de n pontos.
fn onda(i: usize, n: usize) -> Stroke {
    let y = 100.0 + (i * 7 % 1900) as f32;
    let x0 = 100.0 + (i * 13 % 5000) as f32;
    let pts = (0..n).map(|k| [x0 + k as f32 * 6.0, y + (k as f32 / 6.0).sin() * 40.0, 3.0]).collect();
    Stroke { tool: Tool::Pen, rgb: 0xFF0000, alpha: 255, pts, num: 0, text: None, area: Default::default() }
}

/// Um quadro do fade: avança e redesenha o que mudou.
fn quadro_do_fade(c: &mut Canvas, out: &mut [u32], now: u64) {
    c.fade_frame(out, now, FLOOR);
}

/// Casos pesados que o QA achou: 1.000 traços, texto longo, furo grande.
fn medir_pesados(l: &mut Vec<Linha>) {
    let mut c = Canvas::new(W, H);
    c.strokes = (0..1000).map(|i| onda(i, 150)).collect();
    let mut out = vec![0u32; c.len()];
    c.rebuild(&mut out, FLOOR);
    let mut us = Vec::new();
    for _ in 0..2 {
        c.fading = (0..50).map(|i| Fading { stroke: onda(2000 + i * 31, 150), born: 0, k: 255 }).collect();
        for f in 0..FADE_MS.div_ceil(FADE_FRAME_MS) {
            let t = Instant::now();
            quadro_do_fade(&mut c, &mut out, FADE_HOLD_MS + f * FADE_FRAME_MS);
            us.push(t.elapsed().as_secs_f64() * 1e6);
        }
    }
    l.push(resumo("fade_1000_quadro", "ms", us));
    c.fading.clear();
    c.rebuild_region(&mut out, canvas::Rect { x0: 0, y0: 0, x1: W, y1: H }, FLOOR);
    drop(out);

    // Borracha: o círculo da borracha contra os 999 outros traços, por amostra.
    let mut sc = Vec::new();
    c.strokes.pop();
    let mut k = 0;
    l.push(resumo("borracha_999_amostra", "µs", tempos(5, 200, || {
        k += 1;
        let x = 3000.0 + (k % 100) as f32;
        std::hint::black_box(c.strokes.iter().filter(|s| s.hit(x, 1000.0, 20.0, &mut sc)).count());
    })));
    drop(c);

    // Texto de 2.000 letras (17 linhas de 48 px): uma tecla no fim, como a
    // caixa aberta faz (só da linha que mudou para baixo).
    let font = texto::Font::new(48).unwrap();
    let mut frase = "Banding na borda do céu ".repeat(84);
    let (mut laid, mut c) = (texto::Laid::default(), Canvas::new(W, H));
    let mut out = vec![FLOOR; c.len()];
    let mask = canvas::TextMask::default();
    let mut s = Stroke { tool: Tool::Text, rgb: 0xFFFFFF, alpha: 255, pts: vec![[600.0, 400.0, 0.0]], num: 0, text: Some(std::sync::Arc::new(mask)), area: Default::default() };
    let mut n = 0;
    l.push(resumo("texto_2000_tecla", "ms", tempos(3, 60, || {
        n += 1;
        frase.push(if n % 7 == 0 { ' ' } else { 'a' });
        let (from, _) = font.update(&frase, frase.len(), 3000, &mut laid, std::sync::Arc::make_mut(s.text.as_mut().unwrap()));
        let y = 400 + from;
        c.clear_live_from(y);
        let r = c.raster_stroke_from(&out, &s, y);
        c.compose(&mut out, r, s.rgb, 255);
    })));
    c.end_live();
    drop(out);

    l.push(resumo("holofote_900_furo", "ms", tempos(1, 10, || drop(std::hint::black_box(foco::hole(foco::MAX_RADIUS))))));
}

fn tela_para_png() -> (usize, usize, Vec<u32>) {
    let (w, h) = (W as usize, H as usize);
    let mut seed = 7u32;
    let mut px = vec![0xFF1E2024u32; w * h];
    for y in 0..h {
        for x in 0..w {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            px[y * w + x] = match (x, y) {
                (500..=2500, 200..=1800) => 0xFF2B2E33 + ((x / 40 + y / 40) as u32 & 1) * 0x101010,
                (3000..=3800, 300..=900) => 0xFF000000 | seed >> 8,
                (4000..=6000, 100..=2000) if (y / 9) % 3 == 0 && seed >> 30 != 0 => 0xFFE8EAED,
                _ => px[y * w + x],
            };
        }
    }
    (w, h, px)
}

/// Mede tudo. Devolve (tempos, alocações por evento: id -> (bytes, vezes)).
fn medir() -> (Vec<Linha>, Vec<(&'static str, u64, u64)>) {
    let mut l = Vec::new();
    let mut a = Vec::new();

    // Preencher e refazer a tela inteira (voltar ao desenho, desfazer).
    let (mut c, mut out) = tela_com_200();
    l.push(resumo("preencher", "ms", tempos(2, 15, || out.fill(FLOOR))));
    l.push(resumo("refazer_200", "ms", tempos(2, 15, || c.rebuild(&mut out, FLOOR))));

    // Fade: cada quadro de um traço longo sumindo por cima dos 200.
    let mut us = Vec::new();
    volta_do_fade(&mut c, &mut out, |f| f());
    for _ in 0..4 {
        volta_do_fade(&mut c, &mut out, |f| {
            let t = Instant::now();
            f();
            us.push(t.elapsed().as_secs_f64() * 1e6);
        });
    }
    l.push(resumo("fade_quadro", "ms", us));
    let (mut bytes, mut vezes, mut quadros) = (0, 0, 0);
    volta_do_fade(&mut c, &mut out, |f| {
        let (b, v) = alocado(f);
        (bytes, vezes, quadros) = (bytes + b, vezes + v, quadros + 1);
    });
    a.push(("fade_quadro", bytes / quadros, vezes / quadros));
    c.fading.clear();

    // Traço em andamento: cada amostra da caneta (cápsula + compor + o ponto na lista).
    let mut live = Stroke { tool: Tool::Pen, rgb: 0xFF0000, alpha: 255, pts: vec![[400.0, 1200.0, 3.0]], num: 0, text: None, area: Default::default() };
    let mut amostra = |k: usize| {
        let last = *live.pts.last().unwrap();
        let pt = [400.0 + k as f32 * 4.0, 1200.0 + (k as f32 / 9.0).sin() * 80.0, 3.0];
        live.pts.push(pt);
        let r = c.raster_seg(&out, last, pt);
        c.compose(&mut out, r, live.rgb, live.alpha);
    };
    let mut k = 0;
    l.push(resumo("traco_amostra", "µs", tempos(200, 1000, || {
        k += 1;
        amostra(k);
    })));
    // A lista de pontos cresce dobrando: a conta é a média sobre 1000 amostras.
    let (b, v) = alocado(|| (0..1000).for_each(|i| amostra(k + 1 + i)));
    a.push(("traco_amostra", b / 1000, v / 1000));
    c.end_live();

    // Texto: uma tecla = quebra + GDI + cobertura + blocos + compor.
    let font = texto::Font::new(48).unwrap();
    let frase = "Rabisco: anotação na tela\ncom acento, cedilha e ç";
    let mut tecla = || {
        let r = font.render(frase, 3000);
        let s = Stroke { tool: Tool::Text, rgb: 0xFFFFFF, alpha: 255, pts: vec![[1000.0, 600.0, 0.0]], num: 0, text: Some(std::sync::Arc::new(r.mask)), area: Default::default() };
        c.clear_live();
        let r = c.raster_stroke(&out, &s);
        c.compose(&mut out, r, s.rgb, 255);
    };
    l.push(resumo("texto_tecla", "ms", tempos(20, 200, &mut tecla)));
    let (b, v) = alocado(&mut tecla);
    a.push(("texto_tecla", b, v));
    c.end_live();
    drop(out);

    // Holofote num monitor 4K, raio 200: mover o furo e pintar ao ligar.
    let (mw, mh) = (3840, 2160);
    let (d, hole) = foco::hole(200.0);
    let mut px = vec![foco::DARK; (mw * mh) as usize];
    let mut prev = canvas::Rect::EMPTY;
    let mut i = 0;
    let mut mover = || {
        i += 1;
        let next = foco::hole_box(500 + i % 2000, 800 + (i / 3) % 500, d);
        std::hint::black_box(foco::move_hole(&mut px, mw, mh, prev, next, &hole));
        prev = next;
    };
    l.push(resumo("holofote_mover", "µs", tempos(200, 2000, &mut mover)));
    let (b, v) = alocado(&mut mover);
    a.push(("holofote_mover", b, v));
    l.push(resumo("holofote_ligar", "ms", tempos(2, 15, || px.fill(foco::DARK))));
    drop(px);

    // Print: PNG da tela inteira (thread do Ctrl+S) e a volta (colar).
    let (w, h, img) = tela_para_png();
    let mut bytes = Vec::new();
    l.push(resumo("png_codificar", "ms", tempos(1, 7, || bytes = png::encode(w, h, &img))));
    let mut back = None;
    l.push(resumo("png_decodificar", "ms", tempos(1, 7, || back = png::decode(&bytes))));
    let (_, _, back) = back.unwrap();
    assert!(back.iter().zip(&img).all(|(b, p)| *b == p | 0xFF00_0000), "o PNG não voltou igual");
    a.push(("png_kb", bytes.len() as u64 / 1000, 0));
    medir_pesados(&mut l);
    (l, a)
}

// ---------- orçamento ----------

/// Lê `chave = número` do orçamento (comentário com #, seções ignoradas).
pub fn orcamento(texto: &str) -> Vec<(String, f64)> {
    texto
        .lines()
        .map(|l| l.split('#').next().unwrap().trim())
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().replace('_', "").parse().ok()?))
        })
        .collect()
}

fn limite(o: &[(String, f64)], k: &str) -> Option<f64> {
    o.iter().find(|(c, _)| c == k).map(|(_, v)| *v)
}

fn pasta() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("perf")
}

#[test]
fn le_o_orcamento() {
    let o = orcamento("[tempo]\nrefazer_200_ms = 120 # folga\n\n# nada\nexe_bytes = 440_320\nlixo\n");
    assert_eq!(o, vec![("refazer_200_ms".to_string(), 120.0), ("exe_bytes".to_string(), 440320.0)]);
    assert_eq!(limite(&o, "exe_bytes"), Some(440320.0));
    assert_eq!(limite(&o, "x"), None);
}

#[test]
fn mediana_e_p95() {
    let l = resumo("x", "µs", (1..=100).map(f64::from).collect());
    assert_eq!((l.mediana, l.p95), (51.0, 95.0));
    let l = resumo("x", "ms", vec![3000.0, 1000.0, 2000.0]);
    assert_eq!((l.mediana, l.p95), (2.0, 3.0));
}

#[test]
fn o_contador_ve_a_alocacao() {
    let (b, v) = alocado(|| drop(std::hint::black_box(vec![0u8; 1000])));
    assert!(b >= 1000 && v >= 1, "{b} {v}");
    assert_eq!(alocado(|| ()), (0, 0));
}

#[test]
#[ignore]
fn perf_tempos_e_alocacoes() {
    let o = orcamento(&std::fs::read_to_string(pasta().join("orcamento.toml")).expect("perf/orcamento.toml"));
    let (linhas, aloc) = medir();
    let mut falhas = Vec::new();
    println!("\n{:<18} {:>10} {:>10} {:>10}  {:>5}", "métrica", "mediana", "p95", "limite", "n");
    for x in &linhas {
        let k = format!("{}_{}", x.id, if x.unidade == "ms" { "ms" } else { "us" });
        let lim = limite(&o, &k);
        let mark = match lim {
            Some(m) if x.mediana > m => {
                falhas.push(format!("{k}: {:.3} > {m}", x.mediana));
                "  ESTOUROU"
            }
            None => "  (sem limite)",
            _ => "",
        };
        let lim = lim.map_or("-".into(), |m| format!("{m}"));
        println!("{:<18} {:>7.3} {:<2} {:>7.3} {:<2} {:>10}  {:>5}{mark}", x.id, x.mediana, x.unidade, x.p95, x.unidade, lim, x.n);
    }
    println!("\n{:<18} {:>12} {:>8} {:>12}", "alocação/evento", "bytes", "vezes", "limite");
    for &(id, b, v) in &aloc {
        let k = if id == "png_kb" { id.to_string() } else { format!("{id}_bytes") };
        let lim = limite(&o, &k);
        if let Some(m) = lim
            && b as f64 > m
        {
            falhas.push(format!("{k}: {b} > {m}"));
        }
        println!("{:<18} {:>12} {:>8} {:>12}", id, b, v, lim.map_or("-".into(), |m| format!("{m}")));
    }

    let mut j = String::from("{\n  \"tela\": [6400, 2160],\n  \"tempos\": {\n");
    let t: Vec<String> = linhas
        .iter()
        .map(|x| format!("    \"{}\": {{\"unidade\": \"{}\", \"mediana\": {:.4}, \"p95\": {:.4}, \"n\": {}}}", x.id, x.unidade, x.mediana, x.p95, x.n))
        .collect();
    j += &t.join(",\n");
    j += "\n  },\n  \"alocacoes\": {\n";
    let t: Vec<String> = aloc.iter().map(|(id, b, v)| format!("    \"{id}\": {{\"bytes\": {b}, \"vezes\": {v}}}")).collect();
    j += &t.join(",\n");
    j += "\n  }\n}\n";
    std::fs::write(pasta().join("ultimo.json"), j).unwrap();
    assert!(falhas.is_empty(), "passou do orçamento: {falhas:?}");
}


