//! Traços, rasterização e composição. Tudo em CPU, pixels BGRA
//! pré-multiplicados (u32 0xAARRGGBB, que é o formato da DIB do Windows).
//!
//! Não existe cópia da tela aqui: a DIB da janela (`out`) já é o desenho.
//! O traço em andamento usa blocos de 64x64 alocados só onde a caneta passa;
//! cada bloco guarda a cobertura do traço (máximo por pixel) e o pedaço da
//! tela que estava embaixo. Assim o marca-texto não escurece onde cruza, a
//! forma pode ser redesenhada enquanto arrasta, e a memória extra some quando
//! o traço termina.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32, // exclusivo
    pub y1: i32, // exclusivo
}

impl Rect {
    pub const EMPTY: Rect = Rect { x0: i32::MAX, y0: i32::MAX, x1: i32::MIN, y1: i32::MIN };

    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        Rect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }

    pub fn clip(&self, w: i32, h: i32) -> Rect {
        let r = Rect { x0: self.x0.max(0), y0: self.y0.max(0), x1: self.x1.min(w), y1: self.y1.min(h) };
        if r.is_empty() { Rect::EMPTY } else { r }
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let r = Rect { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) };
        if r.is_empty() { Rect::EMPTY } else { r }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Marker,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Eraser,
    /// Marcador numerado: círculo com o número dentro (um clique).
    Number,
    /// Caixa de texto: a cobertura vem pronta do GDI (`Stroke::text`).
    Text,
}

impl Tool {
    pub fn is_freehand(self) -> bool {
        matches!(self, Tool::Pen | Tool::Marker)
    }
}

/// Ponto do traço: x, y e raio (já com a pressão aplicada).
pub type Pt = [f32; 3];

#[derive(Clone, Debug)]
pub struct Stroke {
    pub tool: Tool,
    pub rgb: u32,
    pub alpha: u8,
    /// Mão livre: todos os pontos. Formas: [início, fim].
    /// Marcador numerado: [centro, centro + (raio do círculo, 0)]; o terceiro
    /// valor dos dois é a metade da espessura da linha.
    pub pts: Vec<Pt>,
    /// Número do marcador (só `Tool::Number`; 0 nos outros).
    pub num: u16,
    /// Texto já rasterizado (só `Tool::Text`), com o canto em `pts[0]`.
    /// `Arc`: o desfazer e a lista guardam o mesmo bitmap, sem cópia (e a
    /// reconstrução em faixas lê de várias threads).
    pub text: Option<std::sync::Arc<TextMask>>,
    /// Caixa guardada no primeiro `bbox()`: o fade, a borracha e o refazer
    /// região perguntam a caixa de cada traço toda hora. Traço pronto não muda
    /// os pontos (só o em andamento, que não pede a caixa).
    pub area: Memo,
}

/// A caixa guardada: dois cantos em atômicos (as faixas leem de várias
/// threads). Menor no exe que um `OnceLock`.
#[derive(Debug, Default)]
pub struct Memo {
    done: core::sync::atomic::AtomicBool,
    a: core::sync::atomic::AtomicU64,
    b: core::sync::atomic::AtomicU64,
}

impl Clone for Memo {
    fn clone(&self) -> Memo {
        let m = Memo::default();
        if let Some(r) = self.get() {
            m.set(r);
        }
        m
    }
}

impl Memo {
    fn get(&self) -> Option<Rect> {
        use core::sync::atomic::Ordering::{Acquire, Relaxed};
        let pair = |v: u64| ((v >> 32) as u32 as i32, v as u32 as i32);
        self.done.load(Acquire).then(|| {
            let ((x0, y0), (x1, y1)) = (pair(self.a.load(Relaxed)), pair(self.b.load(Relaxed)));
            Rect { x0, y0, x1, y1 }
        })
    }

    fn set(&self, r: Rect) {
        use core::sync::atomic::Ordering::{Relaxed, Release};
        let pair = |x: i32, y: i32| (x as u32 as u64) << 32 | y as u32 as u64;
        self.a.store(pair(r.x0, r.y0), Relaxed);
        self.b.store(pair(r.x1, r.y1), Relaxed);
        self.done.store(true, Release);
    }
}

/// Cobertura (0-255) do texto numa caixa w x h.
#[derive(Clone, Debug, Default)]
pub struct TextMask {
    pub w: i32,
    pub h: i32,
    pub cov: Vec<u8>,
    /// O texto e o tamanho da fonte, para abrir a caixa de novo e editar.
    pub src: String,
    pub px: i32,
}

impl Stroke {
    /// Lista de cápsulas (segmentos com raio) que desenham o traço.
    pub fn segments(&self, out: &mut Vec<(Pt, Pt)>) {
        out.clear();
        let p = &self.pts;
        if p.is_empty() {
            return;
        }
        match self.tool {
            Tool::Text => {}
            Tool::Number => {
                let (cx, cy, w) = (p[0][0], p[0][1], p[0][2]);
                let r = if p.len() > 1 { (p[1][0] - cx).abs().max(4.0) } else { 4.0 };
                number_segments(cx, cy, r, w, self.num, out);
            }
            Tool::Pen | Tool::Marker | Tool::Eraser => {
                if p.len() == 1 {
                    out.push((p[0], p[0]));
                } else {
                    for k in 0..p.len() {
                        freehand_piece(p, k, out);
                    }
                }
            }
            Tool::Line => out.push((p[0], p[p.len() - 1])),
            Tool::Arrow => arrow(p[0], p[p.len() - 1], out),
            Tool::Rect => {
                let (a, b) = (p[0], p[p.len() - 1]);
                let r = a[2];
                let c = [[a[0], a[1], r], [b[0], a[1], r], [b[0], b[1], r], [a[0], b[1], r]];
                for i in 0..4 {
                    out.push((c[i], c[(i + 1) % 4]));
                }
            }
            Tool::Ellipse => {
                let (a, b) = (p[0], p[p.len() - 1]);
                let (cx, cy) = ((a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5);
                let (rx, ry) = (((b[0] - a[0]) * 0.5).abs(), ((b[1] - a[1]) * 0.5).abs());
                let n = (((rx + ry) * 0.25) as usize).clamp(24, 160);
                ellipse(cx, cy, rx, ry, a[2], n, out);
            }
        }
    }

    /// O círculo (x, y, r) encosta no traço?
    pub fn hit(&self, x: f32, y: f32, r: f32, scratch: &mut Vec<(Pt, Pt)>) -> bool {
        // Pela caixa guardada: quase todo traço fica longe da borracha.
        let b = self.bbox(scratch);
        let near = x + r >= b.x0 as f32 && x - r < b.x1 as f32 && y + r >= b.y0 as f32 && y - r < b.y1 as f32;
        if !near || self.text.is_some() {
            return near;
        }
        // shortcut: à mão testa a linha dos pontos crus, não a curva (no máximo ~1/4 de um canto de diferença); a curva custava 116 µs com 999 traços.
        let raw = self.tool.is_freehand() && self.pts.len() > 1;
        if raw {
            scratch.clear();
            scratch.extend(self.pts.windows(2).map(|w| (w[0], w[1])));
        } else {
            self.segments(scratch);
        }
        scratch.iter().any(|(a, b)| {
            let (t, d) = seg_dist(a, b, x, y);
            d < a[2] + (b[2] - a[2]) * t + r
        })
    }

    /// Caixa que o traço ocupa na tela (com a ponta da seta).
    pub fn bbox(&self, scratch: &mut Vec<(Pt, Pt)>) -> Rect {
        if let Some(b) = self.text_box() {
            return b;
        }
        if let Some(r) = self.area.get() {
            return r;
        }
        self.segments(scratch);
        let r = scratch.iter().fold(Rect::EMPTY, |r, (a, b)| r.union(&seg_box(a, b, i32::MAX, i32::MAX)));
        self.area.set(r); // duas threads calculando juntas gravam o mesmo valor
        r
    }
}

impl Stroke {
    /// Caixa do texto na tela (None se não é texto).
    pub fn text_box(&self) -> Option<Rect> {
        let t = self.text.as_ref()?;
        let (x0, y0) = (self.pts[0][0].round() as i32, self.pts[0][1].round() as i32);
        Some(Rect { x0, y0, x1: x0 + t.w, y1: y0 + t.h })
    }
}

/// Maior número de marcador (dois dígitos cabem melhor no círculo).
pub const MAX_NUMBER: u16 = 99;

/// Raio do círculo e meia espessura do marcador para a espessura da barra.
pub fn marker_dims(size: f32) -> (f32, f32) {
    ((size * 2.0).max(10.0) + 8.0, (size * 0.25).max(1.3))
}

/// Número do próximo marcador: o maior que está na tela + 1. Sai da lista de
/// traços, então desfazer, refazer, borracha e "apagar tudo" ficam certos sem
/// um contador separado (apagar tudo zera: a lista fica sem marcadores).
pub fn next_number<'a>(strokes: impl IntoIterator<Item = &'a Stroke>) -> u16 {
    strokes.into_iter().filter(|s| s.tool == Tool::Number).map(|s| s.num).max().map_or(1, |m| (m + 1).min(MAX_NUMBER))
}

impl Stroke {
    pub fn marker(cx: f32, cy: f32, size: f32, num: u16, rgb: u32) -> Stroke {
        let (r, w) = marker_dims(size);
        Stroke { tool: Tool::Number, rgb, alpha: 255, pts: vec![[cx, cy, w], [cx + r, cy, w]], num, text: None, area: Default::default() }
    }
}

/// Dígitos desenhados à mão: polilinhas numa caixa de 0.6 x 1 (y para baixo).
/// Mesmas cápsulas dos traços: sem fonte, sem GDI, e o marcador continua vetor.
const GLYPHS: [&[&[(f32, f32)]]; 10] = [
    &[&[(0.1, 0.0), (0.5, 0.0), (0.6, 0.15), (0.6, 0.85), (0.5, 1.0), (0.1, 1.0), (0.0, 0.85), (0.0, 0.15), (0.1, 0.0)]],
    &[&[(0.1, 0.2), (0.35, 0.0), (0.35, 1.0)]],
    &[&[(0.02, 0.2), (0.15, 0.03), (0.45, 0.03), (0.58, 0.2), (0.58, 0.4), (0.0, 1.0), (0.6, 1.0)]],
    &[
        &[(0.02, 0.08), (0.15, 0.0), (0.45, 0.0), (0.58, 0.12), (0.58, 0.35), (0.42, 0.48), (0.25, 0.5)],
        &[(0.42, 0.48), (0.6, 0.62), (0.6, 0.88), (0.45, 1.0), (0.15, 1.0), (0.02, 0.92)],
    ],
    &[&[(0.45, 1.0), (0.45, 0.0), (0.0, 0.68), (0.6, 0.68)]],
    &[&[(0.55, 0.0), (0.08, 0.0), (0.03, 0.48), (0.3, 0.42), (0.5, 0.45), (0.6, 0.6), (0.6, 0.85), (0.45, 1.0), (0.15, 1.0), (0.02, 0.92)]],
    &[&[(0.55, 0.05), (0.35, 0.0), (0.15, 0.1), (0.02, 0.35), (0.0, 0.7), (0.1, 0.93), (0.3, 1.0), (0.5, 0.93), (0.6, 0.72), (0.5, 0.52), (0.3, 0.47), (0.1, 0.55), (0.0, 0.7)]],
    &[&[(0.0, 0.0), (0.6, 0.0), (0.2, 1.0)]],
    &[
        &[(0.3, 0.0), (0.5, 0.08), (0.55, 0.25), (0.3, 0.48), (0.05, 0.25), (0.1, 0.08), (0.3, 0.0)],
        &[(0.3, 0.48), (0.55, 0.7), (0.6, 0.85), (0.45, 1.0), (0.15, 1.0), (0.0, 0.85), (0.05, 0.7), (0.3, 0.48)],
    ],
    &[&[(0.05, 0.95), (0.25, 1.0), (0.45, 0.9), (0.58, 0.65), (0.6, 0.3), (0.5, 0.07), (0.3, 0.0), (0.1, 0.07), (0.0, 0.28), (0.1, 0.48), (0.3, 0.53), (0.5, 0.45), (0.6, 0.3)]],
];

/// Círculo + dígitos do número, como cápsulas.
/// Peça `k` do traço à mão (precisa de 2 pontos ou mais): reta do primeiro
/// ponto até o meio do primeiro trecho, curvas de um meio de trecho ao outro
/// com o ponto do meio como controle, e reta do último meio até o último ponto.
/// A peça k (fora a última) só usa os pontos k-1..=k+1: o traço ao vivo
/// desenha cada uma assim que o ponto seguinte chega.
pub fn freehand_piece(p: &[Pt], k: usize, out: &mut Vec<(Pt, Pt)>) {
    let mid = |a: Pt, b: Pt| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, (a[2] + b[2]) * 0.5];
    let n = p.len();
    if k == 0 {
        return out.push((p[0], mid(p[0], p[1])));
    }
    if k == n - 1 {
        return out.push((mid(p[n - 2], p[n - 1]), p[n - 1]));
    }
    let (a, c, b) = (mid(p[k - 1], p[k]), p[k], mid(p[k], p[k + 1]));
    let len = (c[0] - a[0]).hypot(c[1] - a[1]) + (b[0] - c[0]).hypot(b[1] - c[1]);
    let steps = ((len / 4.0) as usize).clamp(1, 16);
    let mut prev = a;
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let u = 1.0 - t;
        let q: Pt = core::array::from_fn(|j| u * u * a[j] + 2.0 * u * t * c[j] + t * t * b[j]);
        out.push((prev, q));
        prev = q;
    }
}

pub fn number_segments(cx: f32, cy: f32, r: f32, w: f32, num: u16, out: &mut Vec<(Pt, Pt)>) {
    ellipse(cx, cy, r, r, w, ((r * 1.5) as usize).clamp(24, 96), out);
    let digits: Vec<usize> = num.to_string().bytes().map(|b| (b - b'0') as usize).collect();
    let n = digits.len().min(3);
    // Altura do dígito em função do raio e da quantidade de dígitos.
    let s = r * [1.2, 0.9, 0.68][n - 1];
    let total = s * (0.6 + 0.75 * (n - 1) as f32);
    let (left, top) = (cx - total / 2.0, cy - s / 2.0);
    for (k, &d) in digits.iter().take(n).enumerate() {
        let ox = left + k as f32 * 0.75 * s;
        for line in GLYPHS[d] {
            for p in line.windows(2) {
                out.push(([ox + p[0].0 * s, top + p[0].1 * s, w], [ox + p[1].0 * s, top + p[1].1 * s, w]));
            }
        }
    }
}

/// Seta de a até b (ponta em b).
pub fn arrow(a: Pt, b: Pt, out: &mut Vec<(Pt, Pt)>) {
    out.push((a, b));
    let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    let head = (b[2] * 6.0).max(14.0).min(len * 0.6);
    let (ux, uy) = (dx / len * head, dy / len * head);
    let (s, c) = 0.5f32.sin_cos(); // ~28 graus
    for sign in [1.0f32, -1.0] {
        let hx = ux * c - uy * s * sign;
        let hy = ux * s * sign + uy * c;
        out.push((b, [b[0] + hx, b[1] + hy, b[2]]));
    }
}

pub fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32, r: f32, n: usize, out: &mut Vec<(Pt, Pt)>) {
    let at = |i: usize| {
        let t = i as f32 / n as f32 * std::f32::consts::TAU;
        [cx + rx * t.cos(), cy + ry * t.sin(), r]
    };
    for i in 0..n {
        out.push((at(i), at(i + 1)));
    }
}

/// Parâmetro t no segmento e distância do ponto ao segmento.
#[inline]
pub fn seg_dist(a: &Pt, b: &Pt, x: f32, y: f32) -> (f32, f32) {
    let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = vx * vx + vy * vy;
    let t = if l2 > 1e-6 { (((x - a[0]) * vx + (y - a[1]) * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (dx, dy) = (x - (a[0] + vx * t), y - (a[1] + vy * t));
    (t, (dx * dx + dy * dy).sqrt())
}

/// Caixa (já recortada) de uma cápsula.
pub fn seg_box(a: &Pt, b: &Pt, w: i32, h: i32) -> Rect {
    let rmax = a[2].max(b[2]) + 1.0;
    Rect {
        x0: (a[0].min(b[0]) - rmax).floor() as i32,
        y0: (a[1].min(b[1]) - rmax).floor() as i32,
        x1: (a[0].max(b[0]) + rmax).ceil() as i32 + 1,
        y1: (a[1].max(b[1]) + rmax).ceil() as i32 + 1,
    }
    .clip(w, h)
}

/// Cobertura (0..1) do pixel (x, y) pela cápsula a-b.
#[inline]
pub fn coverage(a: &Pt, b: &Pt, x: i32, y: i32) -> f32 {
    let (t, d) = seg_dist(a, b, x as f32 + 0.5, y as f32 + 0.5);
    (a[2] + (b[2] - a[2]) * t - d + 0.5).clamp(0.0, 1.0)
}

/// Cápsula preparada para rasterizar rápido: a divisão sai do laço, e a raiz
/// quadrada só roda na faixa de 1 px da borda (dentro = 255, fora = 0).
struct Seg {
    ax: f32,
    ay: f32,
    vx: f32,
    vy: f32,
    inv_l2: f32,
    r0: f32,
    dr: f32,
    rmax: f32,
}

impl Seg {
    fn new(a: &Pt, b: &Pt) -> Seg {
        let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = vx * vx + vy * vy;
        Seg { ax: a[0], ay: a[1], vx, vy, inv_l2: if l2 > 1e-6 { 1.0 / l2 } else { 0.0 }, r0: a[2], dr: b[2] - a[2], rmax: a[2].max(b[2]) }
    }

    /// Distância² ao segmento e o raio naquele ponto.
    #[inline]
    fn dist2(&self, px: f32, py: f32) -> (f32, f32) {
        let (wx, wy) = (px - self.ax, py - self.ay);
        let t = ((wx * self.vx + wy * self.vy) * self.inv_l2).clamp(0.0, 1.0);
        let (dx, dy) = (wx - self.vx * t, wy - self.vy * t);
        (dx * dx + dy * dy, self.r0 + self.dr * t)
    }

    #[inline]
    fn cov(&self, x: i32, y: i32) -> u8 {
        let (d2, rad) = self.dist2(x as f32 + 0.5, y as f32 + 0.5);
        let outer = rad + 0.5;
        if d2 >= outer * outer {
            return 0;
        }
        let inner = rad - 0.5;
        if inner > 0.0 && d2 <= inner * inner {
            return 255;
        }
        ((outer - d2.sqrt()).clamp(0.0, 1.0) * 255.0 + 0.5) as u8
    }
}

/// a*b/255 arredondado, sem divisão.
#[inline]
pub fn mul255(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

/// src sobre dst, os dois pré-multiplicados.
#[inline]
pub fn over(src: u32, dst: u32) -> u32 {
    let sa = src >> 24;
    if sa == 255 {
        return src;
    }
    if sa == 0 {
        return dst;
    }
    let inv = 255 - sa;
    let ch = |sh: u32| (((src >> sh) & 255) + mul255((dst >> sh) & 255, inv)).min(255) << sh;
    ch(24) | ch(16) | ch(8) | ch(0)
}

/// Cor 0xRRGGBB com alfa a, pré-multiplicada.
#[inline]
pub fn premul(rgb: u32, a: u32) -> u32 {
    (a << 24) | (mul255((rgb >> 16) & 255, a) << 16) | (mul255((rgb >> 8) & 255, a) << 8) | mul255(rgb & 255, a)
}

/// Modo fade: o traço fica inteiro por `FADE_HOLD_MS` e some em `FADE_MS`.
pub const FADE_HOLD_MS: u64 = 3000;
pub const FADE_MS: u64 = 500;
/// Intervalo entre quadros enquanto algum traço está sumindo (30 por segundo
/// basta para 0,5 s de fade e custa metade de 60).
pub const FADE_FRAME_MS: u64 = 33;

/// Opacidade (0-255) de um traço com fade `elapsed` ms depois de terminado;
/// None = já sumiu (sai da lista).
pub fn fade_level(elapsed: u64) -> Option<u8> {
    if elapsed < FADE_HOLD_MS {
        return Some(255);
    }
    let t = elapsed - FADE_HOLD_MS;
    (t < FADE_MS).then(|| (255 - t * 255 / FADE_MS) as u8)
}

/// Traço do modo fade: fica fora da lista de traços e do desfazer, e some sozinho.
pub struct Fading {
    pub stroke: Stroke,
    /// Quando terminou (ms, relógio de quem chama).
    pub born: u64,
    /// Opacidade atual (multiplica o alfa do traço).
    pub k: u8,
}

/// Fundo do modo desenho: quase invisível, mas a janela pega o clique.
pub const FLOOR: u32 = 0x0100_0000;

const TS: i32 = 64;
/// Threads do `rebuild`. Mais que isso não ganha: a memória vira o gargalo.
const REBUILD_THREADS: usize = 8;
const TN: usize = (TS * TS) as usize;

struct Tile {
    mask: [u8; TN],
    base: [u32; TN],
}

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    tw: i32,
    tiles: Vec<Option<Box<Tile>>>,
    used: Vec<usize>,
    /// Blocos para reaproveitar enquanto reconstrói; esvaziado no fim.
    pool: Vec<Box<Tile>>,
    /// Reconstruindo: cada traço é composto logo depois de rasterizado, então
    /// `out` ainda é o que está embaixo e o bloco não precisa da cópia (`base`).
    pooling: bool,
    pub strokes: Vec<Stroke>,
    /// Traços do modo fade, por cima dos outros.
    pub fading: Vec<Fading>,
    segs: Vec<(Pt, Pt)>,
    /// Para as caixas (`bbox`) do fade e do refazer região: sem alocar por quadro.
    scratch: Vec<(Pt, Pt)>,
    /// Camada-base do fade: fundo + traços fixos, por bloco 64x64, só onde algo
    /// some. Vale para (fundo, nº de traços fixos); solta quando o fade acaba.
    base: Vec<Option<Box<[u32; TN]>>>,
    base_key: Option<(u32, usize)>,
    /// Caixas que o último `fade_tick` mudou (uma por traço).
    dirty: Vec<Rect>,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Canvas {
        let (w, h) = (w.max(1), h.max(1));
        let tw = (w + TS - 1) / TS;
        let th = (h + TS - 1) / TS;
        let mut tiles = Vec::new();
        tiles.resize_with((tw * th) as usize, || None);
        Canvas { w, h, tw, tiles, used: Vec::new(), pool: Vec::new(), pooling: false, strokes: Vec::new(), fading: Vec::new(), segs: Vec::new(), scratch: Vec::new(), base: Vec::new(), base_key: None, dirty: Vec::new() }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        (self.w * self.h) as usize
    }

    /// Bytes que os blocos do traço em andamento ocupam agora.
    #[cfg(test)]
    pub fn live_bytes(&self) -> usize {
        self.used.len() * size_of::<Tile>()
    }

    /// Bloco (tx, ty), criado na hora com o pedaço de `out` embaixo.
    fn tile(&mut self, tx: i32, ty: i32, out: &[u32]) -> &mut Tile {
        let i = (ty * self.tw + tx) as usize;
        if self.tiles[i].is_none() {
            // Box::new de um array grande passaria pela pilha.
            let mut t = match self.pool.pop() {
                Some(mut t) => {
                    t.mask.fill(0);
                    t
                }
                None => unsafe { Box::new_zeroed().assume_init() },
            };
            let (x0, y0) = (tx * TS, ty * TS);
            let cw = (self.w - x0).min(TS) as usize;
            let rows = if self.pooling { 0 } else { (self.h - y0).min(TS) };
            for y in 0..rows {
                let src = ((y0 + y) * self.w + x0) as usize;
                let dst = (y * TS) as usize;
                t.base[dst..dst + cw].copy_from_slice(&out[src..src + cw]);
            }
            self.tiles[i] = Some(t);
            self.used.push(i);
        }
        self.tiles[i].as_mut().unwrap()
    }

    /// Escreve uma cápsula na cobertura do traço em andamento. Devolve a área tocada.
    pub fn raster_seg(&mut self, out: &[u32], a: Pt, b: Pt) -> Rect {
        self.raster_seg_in(out, a, b, Rect { x0: 0, y0: 0, x1: self.w, y1: self.h })
    }

    /// Como `raster_seg`, mas só dentro de `clip`.
    fn raster_seg_in(&mut self, out: &[u32], a: Pt, b: Pt, clip: Rect) -> Rect {
        let r = seg_box(&a, &b, self.w, self.h).intersect(&clip);
        if r.is_empty() {
            return Rect::EMPTY;
        }
        let seg = Seg::new(&a, &b);
        // Meia diagonal do bloco: bloco mais longe que isso do segmento não é tocado.
        let reach = seg.rmax + 0.5 + TS as f32 * std::f32::consts::FRAC_1_SQRT_2;
        for ty in r.y0 / TS..=(r.y1 - 1) / TS {
            for tx in r.x0 / TS..=(r.x1 - 1) / TS {
                let (bx, by) = (tx * TS, ty * TS);
                let half = TS as f32 / 2.0;
                if seg.dist2(bx as f32 + half, by as f32 + half).0 > reach * reach {
                    continue; // linha longa na diagonal: não cria bloco à toa
                }
                let (x0, x1) = (r.x0.max(bx), r.x1.min(bx + TS));
                let (y0, y1) = (r.y0.max(by), r.y1.min(by + TS));
                let t = self.tile(tx, ty, out);
                for y in y0..y1 {
                    let row = ((y - by) * TS - bx) as isize;
                    for x in x0..x1 {
                        let c = seg.cov(x, y);
                        if c != 0 {
                            let m = &mut t.mask[(row + x as isize) as usize];
                            *m = (*m).max(c);
                        }
                    }
                }
            }
        }
        r
    }

    /// Escreve o traço inteiro na cobertura.
    pub fn raster_stroke(&mut self, out: &[u32], s: &Stroke) -> Rect {
        self.raster_stroke_in(out, s, Rect { x0: 0, y0: 0, x1: self.w, y1: self.h })
    }

    fn raster_stroke_in(&mut self, out: &[u32], s: &Stroke, clip: Rect) -> Rect {
        if let (Some(t), Some(b)) = (&s.text, s.text_box()) {
            return self.raster_text_in(out, t, b, clip);
        }
        let mut segs = std::mem::take(&mut self.segs);
        s.segments(&mut segs);
        let mut r = Rect::EMPTY;
        for &(a, b) in &segs {
            r = r.union(&self.raster_seg_in(out, a, b, clip));
        }
        self.segs = segs;
        r
    }

    /// Copia a cobertura do texto (caixa b na tela) para os blocos, dentro de clip.
    fn raster_text_in(&mut self, out: &[u32], t: &TextMask, b: Rect, clip: Rect) -> Rect {
        let r = b.clip(self.w, self.h).intersect(&clip);
        if r.is_empty() {
            return Rect::EMPTY;
        }
        for ty in r.y0 / TS..=(r.y1 - 1) / TS {
            for tx in r.x0 / TS..=(r.x1 - 1) / TS {
                let (bx, by) = (tx * TS, ty * TS);
                let (x0, x1) = (r.x0.max(bx), r.x1.min(bx + TS));
                let (y0, y1) = (r.y0.max(by), r.y1.min(by + TS));
                let tile = self.tile(tx, ty, out);
                let n = (x1 - x0) as usize;
                for y in y0..y1 {
                    let src = ((y - b.y0) * t.w + x0 - b.x0) as usize;
                    let dst = ((y - by) * TS + x0 - bx) as usize;
                    for (m, &c) in tile.mask[dst..dst + n].iter_mut().zip(&t.cov[src..src + n]) {
                        *m = (*m).max(c);
                    }
                }
            }
        }
        r
    }

    /// Zera a cobertura (forma sendo redesenhada enquanto arrasta).
    pub fn clear_live(&mut self) {
        for &i in &self.used {
            self.tiles[i].as_mut().unwrap().mask.fill(0);
        }
    }

    /// Zera a cobertura da linha y para baixo (a caixa de texto refaz só dali).
    pub fn clear_live_from(&mut self, y: i32) {
        for &i in &self.used {
            let y0 = (i as i32 / self.tw) * TS;
            let from = ((y - y0).clamp(0, TS) * TS) as usize;
            self.tiles[i].as_mut().unwrap().mask[from..].fill(0);
        }
    }

    /// `raster_stroke` só da linha y para baixo.
    pub fn raster_stroke_from(&mut self, out: &[u32], s: &Stroke, y: i32) -> Rect {
        self.raster_stroke_in(out, s, Rect { x0: 0, y0: y.max(0), x1: self.w, y1: self.h })
    }

    /// Pinta em `out`, dentro de r: o que estava embaixo + o traço com a cor dada.
    pub fn compose(&self, out: &mut [u32], r: Rect, rgb: u32, alpha: u8) {
        let r = r.clip(self.w, self.h);
        if r.is_empty() {
            return;
        }
        for ty in r.y0 / TS..=(r.y1 - 1) / TS {
            for tx in r.x0 / TS..=(r.x1 - 1) / TS {
                let Some(t) = &self.tiles[(ty * self.tw + tx) as usize] else { continue };
                let (bx, by) = (tx * TS, ty * TS);
                let (x0, x1) = (r.x0.max(bx), r.x1.min(bx + TS));
                for y in r.y0.max(by)..r.y1.min(by + TS) {
                    let row = ((y - by) * TS - bx) as isize;
                    let orow = (y * self.w) as usize;
                    for x in x0..x1 {
                        let i = (row + x as isize) as usize;
                        let m = t.mask[i] as u32;
                        out[orow + x as usize] =
                            if m == 0 { t.base[i] } else { over(premul(rgb, mul255(alpha as u32, m)), t.base[i]) };
                    }
                }
            }
        }
    }

    /// `compose` da reconstrução: o bloco não tem `base` (ver `pooling`), então
    /// mistura direto em `out` e deixa intocado o que a cobertura não pega.
    /// `out` começa na linha `oy` (uma faixa da tela).
    fn compose_over(&self, out: &mut [u32], oy: i32, r: Rect, rgb: u32, alpha: u8) {
        let r = r.clip(self.w, self.h);
        if r.is_empty() {
            return;
        }
        for ty in r.y0 / TS..=(r.y1 - 1) / TS {
            for tx in r.x0 / TS..=(r.x1 - 1) / TS {
                let Some(t) = &self.tiles[(ty * self.tw + tx) as usize] else { continue };
                let (bx, by) = (tx * TS, ty * TS);
                let (x0, x1) = (r.x0.max(bx), r.x1.min(bx + TS));
                for y in r.y0.max(by)..r.y1.min(by + TS) {
                    let m0 = ((y - by) * TS + x0 - bx) as usize;
                    let o0 = ((y - oy) * self.w + x0) as usize;
                    let n = (x1 - x0) as usize;
                    for (o, &m) in out[o0..o0 + n].iter_mut().zip(&t.mask[m0..m0 + n]) {
                        if m != 0 {
                            *o = over(premul(rgb, mul255(alpha as u32, m as u32)), *o);
                        }
                    }
                }
            }
        }
    }

    /// Fecha o traço em andamento: `out` já tem o resultado, os blocos vão embora.
    /// Devolve a área que os blocos cobriam.
    pub fn end_live(&mut self) -> Rect {
        let mut r = Rect::EMPTY;
        for i in self.used.drain(..) {
            if let Some(t) = self.tiles[i].take() {
                if self.pooling {
                    self.pool.push(t);
                }
            }
            let (tx, ty) = (i as i32 % self.tw, i as i32 / self.tw);
            r = r.union(&Rect { x0: tx * TS, y0: ty * TS, x1: tx * TS + TS, y1: ty * TS + TS });
        }
        r.clip(self.w, self.h)
    }

    /// Desiste do traço em andamento: devolve o que estava embaixo.
    pub fn cancel_live(&mut self, out: &mut [u32]) -> Rect {
        self.clear_live();
        let r = self.end_r();
        self.compose(out, r, 0, 0);
        self.end_live()
    }

    /// Área dos blocos do traço em andamento.
    pub fn live_area(&self) -> Rect {
        self.end_r()
    }

    fn end_r(&self) -> Rect {
        let mut r = Rect::EMPTY;
        for &i in &self.used {
            let (tx, ty) = (i as i32 % self.tw, i as i32 / self.tw);
            r = r.union(&Rect { x0: tx * TS, y0: ty * TS, x1: tx * TS + TS, y1: ty * TS + TS });
        }
        r.clip(self.w, self.h)
    }

    /// Redesenha `out` inteiro a partir da lista de traços (desfazer, borracha,
    /// voltar ao modo desenho). `floor` é a cor do fundo.
    ///
    /// Em faixas horizontais, uma thread cada: cada pixel recebe os traços na
    /// mesma ordem, então o resultado é igual ao de uma thread só.
    pub fn rebuild(&mut self, out: &mut [u32], floor: u32) {
        self.end_live();
        self.drop_base();
        self.pool = Vec::new(); // os blocos que o fade guardou (ex.: apagar tudo no meio)
        let (w, h) = (self.w, self.h);
        let band = self.band();
        let (strokes, fading) = (&self.strokes, &self.fading);
        let at = SendPtr(out.as_mut_ptr());
        par(((h + band - 1) / band) as usize, &|i| {
            let y0 = i as i32 * band;
            let clip = Rect { x0: 0, y0, x1: w, y1: (y0 + band).min(h) };
            // Cada faixa só mexe nas próprias linhas.
            let rows = unsafe { std::slice::from_raw_parts_mut(at.get().add((y0 * w) as usize), ((clip.y1 - y0) * w) as usize) };
            let mut c = Canvas::new(w, h);
            c.pooling = true;
            rows.fill(floor);
            for s in strokes {
                let r = c.raster_stroke_in(&[], s, clip);
                c.compose_over(rows, y0, r, s.rgb, s.alpha);
                c.end_live();
            }
            c.compose_fading(rows, y0, fading, clip);
        });
    }

    /// Altura das faixas em paralelo: múltiplo do bloco (nenhum bloco fica em
    /// duas threads), uma por thread.
    fn band(&self) -> i32 {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(REBUILD_THREADS) as i32;
        ((self.h + TS - 1) / TS + threads - 1) / threads * TS
    }

    /// Compõe os traços do fade que passam por r (dentro de `rows`, que começa na linha y0).
    fn compose_fading(&mut self, rows: &mut [u32], y0: i32, fading: &[Fading], r: Rect) {
        let mut scratch = std::mem::take(&mut self.scratch);
        for f in fading {
            if f.k == 0 || f.stroke.bbox(&mut scratch).intersect(&r).is_empty() {
                continue;
            }
            let rr = self.raster_stroke_in(&[], &f.stroke, r);
            self.compose_over(rows, y0, rr, f.stroke.rgb, mul255(f.stroke.alpha as u32, f.k as u32) as u8);
            self.end_live();
        }
        self.scratch = scratch;
    }

    /// Redesenha só a área r (desfazer um traço, borracha): pinta o fundo nela
    /// e refaz, recortados, só os traços que passam por ali. Devolve a área.
    pub fn rebuild_region(&mut self, out: &mut [u32], r: Rect, floor: u32) -> Rect {
        let r = r.clip(self.w, self.h);
        if r.is_empty() {
            return r;
        }
        self.end_live();
        self.drop_base(); // os traços fixos mudaram (desfazer, borracha)
        self.pooling = true;
        for y in r.y0..r.y1 {
            let row = (y * self.w) as usize;
            out[row + r.x0 as usize..row + r.x1 as usize].fill(floor);
        }
        let strokes = std::mem::take(&mut self.strokes);
        let mut scratch = std::mem::take(&mut self.scratch);
        for s in &strokes {
            if s.bbox(&mut scratch).intersect(&r).is_empty() {
                continue;
            }
            let rr = self.raster_stroke_in(out, s, r);
            self.compose_over(out, 0, rr, s.rgb, s.alpha);
            self.end_live();
        }
        self.strokes = strokes;
        let fading = std::mem::take(&mut self.fading);
        for f in &fading {
            if f.k == 0 || f.stroke.bbox(&mut scratch).intersect(&r).is_empty() {
                continue;
            }
            let rr = self.raster_stroke_in(out, &f.stroke, r);
            self.compose_over(out, 0, rr, f.stroke.rgb, mul255(f.stroke.alpha as u32, f.k as u32) as u8);
            self.end_live();
        }
        self.fading = fading;
        self.scratch = scratch;
        self.pooling = false;
        // Com algo sumindo, os blocos ficam para o próximo quadro do fade.
        if self.fading.is_empty() {
            self.pool = Vec::new();
        }
        r
    }
}

impl Canvas {
    /// Avança o fade para o instante `now` (ms): atualiza a opacidade, tira o que
    /// sumiu e devolve a área que mudou e daqui a quantos ms chamar de novo
    /// (None = nada mais para sumir: o timer pode morrer).
    pub fn fade_tick(&mut self, now: u64) -> (Rect, Option<u64>) {
        let mut dirty = Rect::EMPTY;
        let mut next: Option<u64> = None;
        let mut scratch = std::mem::take(&mut self.scratch);
        let mut list = std::mem::take(&mut self.dirty);
        list.clear();
        self.fading.retain_mut(|f| {
            let age = now.saturating_sub(f.born);
            let k = fade_level(age).unwrap_or(0);
            if k != f.k {
                f.k = k;
                let b = f.stroke.bbox(&mut scratch);
                dirty = dirty.union(&b);
                list.push(b);
            }
            if k == 0 {
                return false;
            }
            let wait = if age < FADE_HOLD_MS { FADE_HOLD_MS - age } else { FADE_FRAME_MS };
            next = Some(next.map_or(wait, |n| n.min(wait)));
            true
        });
        self.scratch = scratch;
        self.dirty = list;
        if self.fading.is_empty() {
            self.drop_base(); // sem quadro (janela escondida): não sobra memória
        }
        (dirty, next)
    }

    /// Um quadro do fade direto em `out`: avança (`fade_tick`) e, em cada caixa
    /// que mudou, copia a camada-base e compõe só os traços do fade por cima.
    /// Com 1.000 traços fixos, refazer a região toda a cada quadro levava
    /// 150-350 ms; a base é feita uma vez por fade. Devolve a união das caixas
    /// (para o `prcDirty`) e o próximo passo, como `fade_tick`.
    pub fn fade_frame(&mut self, out: &mut [u32], now: u64, floor: u32) -> (Rect, Option<u64>) {
        let (all, next) = self.fade_tick(now);
        if !all.is_empty() {
            if self.base_key != Some((floor, self.strokes.len())) {
                self.drop_base();
                self.base_key = Some((floor, self.strokes.len()));
            }
            if self.base.is_empty() {
                self.base.resize_with(self.tiles.len(), || None);
            }
            let mut rects = std::mem::take(&mut self.dirty);
            for r in rects.iter_mut() {
                let c = r.clip(self.w, self.h);
                // Em blocos inteiros: a base é guardada por bloco.
                let up = |v: i32, max: i32| ((v + TS - 1) / TS * TS).min(max);
                *r = if c.is_empty() { c } else { Rect { x0: c.x0 / TS * TS, y0: c.y0 / TS * TS, x1: up(c.x1, self.w), y1: up(c.y1, self.h) } };
            }
            rects.retain(|r| !r.is_empty());
            merge_rects(&mut rects);
            self.end_live();
            self.pooling = true;
            let mut scratch = std::mem::take(&mut self.scratch);
            for &r in &rects {
                self.ensure_base(out, r, floor, &mut scratch);
            }
            self.scratch = scratch;
            let area: i64 = rects.iter().map(|r| (r.x1 - r.x0) as i64 * (r.y1 - r.y0) as i64).sum();
            let fading = std::mem::take(&mut self.fading);
            if area < PAR_FADE_AREA {
                // Um traço sumindo: em faixas, as threads custariam mais que o quadro.
                for &r in &rects {
                    copy_base(&self.base, self.tw, self.w, self.h, out, 0, r);
                    self.compose_fading(out, 0, &fading, r);
                }
            } else {
                let (w, h, band, tw) = (self.w, self.h, self.band(), self.tw);
                let (base, fl, rects) = (&self.base, &fading, &rects);
                let at = SendPtr(out.as_mut_ptr());
                par(((h + band - 1) / band) as usize, &|i| {
                    let y0 = i as i32 * band;
                    let clip = Rect { x0: 0, y0, x1: w, y1: (y0 + band).min(h) };
                    let rows = unsafe { std::slice::from_raw_parts_mut(at.get().add((y0 * w) as usize), ((clip.y1 - y0) * w) as usize) };
                    let mut c = Canvas::new(w, h);
                    c.pooling = true;
                    for r in rects.iter().map(|r| r.intersect(&clip)).filter(|r| !r.is_empty()) {
                        copy_base(base, tw, w, h, rows, y0, r);
                        c.compose_fading(rows, y0, fl, r);
                    }
                });
            }
            self.fading = fading;
            self.pooling = false;
            rects.clear();
            self.dirty = rects;
        }
        if self.fading.is_empty() {
            self.drop_base();
            self.pool = Vec::new();
            self.dirty = Vec::new();
        }
        (all.clip(self.w, self.h), next)
    }

    /// Garante a camada-base dentro de r (em blocos inteiros): faltando algum
    /// bloco, faz a área toda agora em `out` (fundo + traços fixos) e guarda.
    fn ensure_base(&mut self, out: &mut [u32], r: Rect, floor: u32, scratch: &mut Vec<(Pt, Pt)>) {
        let (tx0, tx1, ty0, ty1) = (r.x0 / TS, (r.x1 - 1) / TS, r.y0 / TS, (r.y1 - 1) / TS);
        let missing = (ty0..=ty1).any(|ty| (tx0..=tx1).any(|tx| self.base[(ty * self.tw + tx) as usize].is_none()));
        if !missing {
            return;
        }
        {
            for y in r.y0..r.y1 {
                let row = (y * self.w) as usize;
                out[row + r.x0 as usize..row + r.x1 as usize].fill(floor);
            }
            let strokes = std::mem::take(&mut self.strokes);
            for s in &strokes {
                if s.bbox(scratch).intersect(&r).is_empty() {
                    continue;
                }
                let rr = self.raster_stroke_in(out, s, r);
                self.compose_over(out, 0, rr, s.rgb, s.alpha);
                self.end_live();
            }
            self.strokes = strokes;
        }
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let (x0, y0) = (tx * TS, ty * TS);
                let (cw, y1) = (((x0 + TS).min(self.w) - x0) as usize, (y0 + TS).min(self.h));
                let mut b: Box<[u32; TN]> = unsafe { Box::new_zeroed().assume_init() };
                for y in y0..y1 {
                    let (src, dst) = ((y * self.w + x0) as usize, ((y - y0) * TS) as usize);
                    b[dst..dst + cw].copy_from_slice(&out[src..src + cw]);
                }
                self.base[(ty * self.tw + tx) as usize] = Some(b);
            }
        }
    }

    fn drop_base(&mut self) {
        self.base = Vec::new();
        self.base_key = None;
    }

    /// Bytes da camada-base do fade agora.
    #[cfg(test)]
    pub fn base_bytes(&self) -> usize {
        self.base.iter().flatten().count() * size_of::<[u32; TN]>() + self.base.capacity() * size_of::<usize>()
    }
}

/// Área (px) a partir da qual o quadro do fade vai em faixas paralelas.
const PAR_FADE_AREA: i64 = 1 << 20;

/// Copia a camada-base para `rows` (que começa na linha y0) dentro de r, que
/// está em blocos inteiros na horizontal.
fn copy_base(base: &[Option<Box<[u32; TN]>>], tw: i32, w: i32, h: i32, rows: &mut [u32], y0: i32, r: Rect) {
    for ty in r.y0 / TS..=(r.y1 - 1) / TS {
        for tx in r.x0 / TS..=(r.x1 - 1) / TS {
            let Some(b) = &base[(ty * tw + tx) as usize] else { continue };
            let bx = tx * TS;
            let cw = ((bx + TS).min(w) - bx) as usize;
            for y in r.y0.max(ty * TS)..r.y1.min(ty * TS + TS).min(h) {
                let (dst, src) = (((y - y0) * w + bx) as usize, ((y - ty * TS) * TS) as usize);
                rows[dst..dst + cw].copy_from_slice(&b[src..src + cw]);
            }
        }
    }
}

/// Ponteiro que as faixas dividem: cada uma escreve só nas próprias linhas.
#[derive(Clone, Copy)]
struct SendPtr(*mut u32);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}
impl SendPtr {
    fn get(self) -> *mut u32 {
        self.0
    }
}

/// Roda f(0..n), uma thread cada. A thread que o Windows não abrir roda aqui
/// mesmo, depois das outras: mais lento, mas sem panic. Por `dyn`: uma cópia
/// só do código das threads no exe.
pub fn par(n: usize, f: &(dyn Fn(usize) + Sync)) {
    std::thread::scope(|sc| {
        let jobs: Vec<_> = (0..n).map(|i| std::thread::Builder::new().spawn_scoped(sc, move || f(i)).map_err(|_| i)).collect();
        for j in jobs {
            match j {
                Ok(t) => drop(t.join()),
                Err(i) => f(i),
            }
        }
    })
}

/// Junta as caixas que se encostam: nenhum pixel é refeito duas vezes no quadro.

fn merge_rects(rs: &mut Vec<Rect>) {
    let mut i = 0;
    while i < rs.len() {
        let mut grew = false;
        let mut j = i + 1;
        while j < rs.len() {
            if rs[i].intersect(&rs[j]).is_empty() {
                j += 1;
            } else {
                let o = rs.swap_remove(j);
                rs[i] = rs[i].union(&o);
                grew = true;
            }
        }
        if !grew {
            i += 1;
        }
    }
}


/// 200 ondas de 80 pontos espalhadas pela tela: o cenário da suíte de
/// performance e do `--perf-memoria`.
pub fn sample_strokes(n: usize) -> Vec<Stroke> {
    (0..n)
        .map(|i| {
            let y = 100.0 + (i * 9 % 1900) as f32;
            let pts = (0..80).map(|k| [300.0 + k as f32 * 20.0, y + (k as f32 / 6.0).sin() * 40.0, 3.0]).collect();
            Stroke { tool: Tool::Pen, rgb: 0xFF0000, alpha: 255, pts, num: 0, text: None, area: Default::default() }
        })
        .collect()
}

/// Troca a cor de fundo (entrar/sair do modo desenho).
pub fn swap_floor(out: &mut [u32], from: u32, to: u32) {
    for p in out {
        if *p == from {
            *p = to;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traco_a_mao_vira_curva_e_ao_vivo_bate_com_o_final() {
        // Zigue-zague com cantos de 90°.
        let pts: Vec<Pt> = (0..8).map(|i| [i as f32 * 20.0, if i % 2 == 0 { 0.0 } else { 20.0 }, 3.0 + i as f32 * 0.1]).collect();
        let s = Stroke { tool: Tool::Pen, rgb: 0, alpha: 255, pts: pts.clone(), num: 0, text: None, area: Default::default() };
        let mut all = Vec::new();
        s.segments(&mut all);
        // Ao vivo: cada ponto novo desenha a peça de antes; soltar desenha a última.
        let mut live = Vec::new();
        for n in 2..=pts.len() {
            freehand_piece(&pts[..n], n - 2, &mut live);
        }
        freehand_piece(&pts, pts.len() - 1, &mut live);
        assert_eq!(all, live);
        // Nenhuma quina: o ângulo entre um trecho e o próximo fica bem abaixo dos 90° dos pontos crus.
        let dir = |(a, b): &(Pt, Pt)| (b[1] - a[1]).atan2(b[0] - a[0]);
        let worst = all.windows(2).map(|w| (dir(&w[1]) - dir(&w[0])).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.5, "maior virada {worst} rad");
        assert_eq!((all[0].0, all.last().unwrap().1), (pts[0], pts[7]), "começa e termina nos pontos de verdade");
        // Um e dois pontos continuam um ponto e uma reta.
        let mut one = Vec::new();
        Stroke { pts: vec![pts[0]], ..s.clone() }.segments(&mut one);
        assert_eq!(one, [(pts[0], pts[0])]);
        Stroke { pts: pts[..2].to_vec(), ..s.clone() }.segments(&mut one);
        assert_eq!(one.len(), 2);
    }

    fn draw(c: &mut Canvas, out: &mut [u32], s: &Stroke) {
        let r = c.raster_stroke(out, s);
        c.compose(out, r, s.rgb, s.alpha);
        c.end_live();
    }

    fn pen(rgb: u32, alpha: u8, pts: Vec<Pt>) -> Stroke {
        Stroke { tool: Tool::Pen, rgb, alpha, pts, num: 0, text: None, area: Default::default() }
    }

    #[test]
    fn over_opaco_e_transparente() {
        assert_eq!(over(0xFF112233, 0x80808080), 0xFF112233);
        assert_eq!(over(0, 0x80808080), 0x80808080);
        assert_eq!(over(premul(0xFFFFFF, 128), 0), 0x80808080);
    }

    #[test]
    fn segmento_pinta_dentro_do_raio() {
        let mut c = Canvas::new(150, 100);
        let mut out = vec![0u32; c.len()];
        draw(&mut c, &mut out, &pen(0xFF0000, 255, vec![[10.0, 70.0, 4.0], [140.0, 70.0, 4.0]]));
        assert_eq!(out[70 * 150 + 100], 0xFFFF0000); // atravessa a borda de bloco
        assert_eq!(out[10 * 150 + 100], 0);
        assert_eq!(c.live_bytes(), 0);
    }

    #[test]
    fn marca_texto_nao_escurece_no_cruzamento() {
        let mut c = Canvas::new(64, 64);
        let mut out = vec![0u32; c.len()];
        let s = Stroke { tool: Tool::Marker, ..pen(0xFFFF00, 90, vec![[10.0, 32.0, 6.0], [50.0, 32.0, 6.0], [10.0, 32.0, 6.0]]) };
        draw(&mut c, &mut out, &s);
        assert_eq!(out[32 * 64 + 30] >> 24, 90);
    }

    #[test]
    fn forma_redesenhada_e_cancelada_volta_ao_fundo() {
        let mut c = Canvas::new(200, 200);
        let mut out = vec![FLOOR; c.len()];
        let before = out.clone();
        let mut s = Stroke { tool: Tool::Rect, ..pen(0x00FF00, 255, vec![[10.0, 10.0, 2.0], [150.0, 150.0, 2.0]]) };
        let r = c.raster_stroke(&out, &s);
        c.compose(&mut out, r, s.rgb, s.alpha);
        c.clear_live();
        s.pts[1] = [60.0, 60.0, 2.0];
        let r2 = c.raster_stroke(&out, &s);
        c.compose(&mut out, r.union(&r2), s.rgb, s.alpha);
        assert_eq!(out[150 * 200 + 80], FLOOR); // canto antigo sumiu
        assert_eq!(out[10 * 200 + 30], 0xFF00FF00);
        c.cancel_live(&mut out);
        assert_eq!(out, before);
    }

    #[test]
    fn rebuild_reproduz_os_tracos() {
        let mut c = Canvas::new(130, 70);
        let mut a = vec![FLOOR; c.len()];
        for s in [
            Stroke { tool: Tool::Rect, ..pen(0x00FF00, 255, vec![[8.0, 8.0, 2.0], [100.0, 40.0, 2.0]]) },
            Stroke { tool: Tool::Marker, ..pen(0xFFFF00, 90, vec![[0.0, 20.0, 9.0], [129.0, 25.0, 9.0]]) },
        ] {
            draw(&mut c, &mut a, &s);
            c.strokes.push(s);
        }
        let mut b = vec![0u32; c.len()];
        c.rebuild(&mut b, FLOOR);
        assert_eq!(a, b);
        assert!(c.strokes[0].hit(8.0, 30.0, 1.0, &mut Vec::new()));
        assert!(!c.strokes[0].hit(50.0, 5.0, 1.0, &mut Vec::new()));
    }

    #[test]
    fn rebuild_sem_copia_do_fundo_igual_ao_traco_ao_vivo() {
        // Muitos traços cruzados, opacos e de marca-texto, mais um sumindo.
        let mut c = Canvas::new(1700, 700);
        let mut live = vec![FLOOR; c.len()];
        for (i, mut s) in sample_strokes(60).into_iter().enumerate() {
            s.pts.iter_mut().for_each(|p| p[1] = p[1] % 650.0 + 20.0);
            if i % 3 == 0 {
                (s.tool, s.rgb, s.alpha) = (Tool::Marker, 0xFFFF00, 90);
            }
            draw(&mut c, &mut live, &s);
            c.strokes.push(s);
        }
        let f = Fading { stroke: pen(0x00FF00, 255, vec![[50.0, 50.0, 6.0], [1600.0, 600.0, 6.0]]), born: 0, k: 120 };
        let r = c.raster_stroke(&live, &f.stroke);
        c.compose(&mut live, r, 0x00FF00, 120);
        c.end_live();
        c.fading.push(f);
        let mut full = vec![0u32; c.len()];
        c.rebuild(&mut full, FLOOR);
        assert!(live == full, "rebuild mudou {} pixels", live.iter().zip(&full).filter(|(a, b)| a != b).count());
        let r = Rect { x0: 300, y0: 100, x1: 900, y1: 400 };
        full[..].fill(0);
        c.rebuild(&mut full, FLOOR);
        let mut part = full.clone();
        c.rebuild_region(&mut part, r, FLOOR);
        assert!(part == full);
    }

    #[test]
    fn refazer_regiao_igual_a_refazer_tudo() {
        let mut c = Canvas::new(300, 200);
        let mut out = vec![FLOOR; c.len()];
        let mk = |tool, rgb, alpha, pts: Vec<Pt>| Stroke { tool, rgb, alpha, pts, num: 0, text: None, area: Default::default() };
        for s in [
            mk(Tool::Line, 0x00FF00, 255, vec![[5.0, 5.0, 3.0], [290.0, 190.0, 3.0]]),
            mk(Tool::Marker, 0xFFFF00, 90, vec![[0.0, 100.0, 10.0], [299.0, 90.0, 10.0]]),
            mk(Tool::Arrow, 0x0000FF, 255, vec![[250.0, 20.0, 2.0], [120.0, 150.0, 2.0]]),
        ] {
            draw(&mut c, &mut out, &s);
            c.strokes.push(s);
        }
        // Tira o do meio e refaz só a caixa dele.
        let gone = c.strokes.remove(1);
        let r = gone.bbox(&mut Vec::new());
        c.rebuild_region(&mut out, r, FLOOR);
        let mut full = vec![0u32; c.len()];
        c.rebuild(&mut full, FLOOR);
        assert_eq!(out, full);
    }

    #[test]
    fn linha_diagonal_nao_cria_bloco_a_toa() {
        let mut c = Canvas::new(2048, 2048);
        let out = vec![0u32; c.len()];
        c.raster_seg(&out, [0.0, 0.0, 2.0], [2047.0, 2047.0, 2.0]);
        // 32x32 blocos na caixa; a diagonal toca uns 3 por linha de blocos.
        assert!(c.used.len() < 32 * 4, "{} blocos", c.used.len());
    }

    fn marcadores(n: u16) -> Vec<Stroke> {
        (1..=n).map(|k| Stroke::marker(50.0 * k as f32, 40.0, 4.0, k, 0xFF0000)).collect()
    }

    #[test]
    fn numeracao_segue_a_lista_com_desfazer_borracha_e_apagar_tudo() {
        let mut l = Vec::new();
        assert_eq!(next_number(&l), 1);
        // Três cliques: 1, 2, 3.
        for _ in 0..3 {
            let n = next_number(&l);
            l.push(Stroke::marker(10.0, 10.0, 4.0, n, 0xFF0000));
        }
        assert_eq!(l.iter().map(|s| s.num).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(next_number(&l), 4);
        // Traço que não é marcador não conta.
        l.push(pen(0xFF0000, 255, vec![[1.0, 1.0, 2.0]]));
        assert_eq!(next_number(&l), 4);
        // Desfazer o último marcador: o próximo clique reaproveita o 3.
        let desfeito = l.remove(3); // a caneta
        l.pop().unwrap();
        assert_eq!(next_number(&l), 3);
        // Refazer volta a lista e a contagem.
        l.push(Stroke::marker(10.0, 10.0, 4.0, 3, 0xFF0000));
        l.push(desfeito);
        assert_eq!(next_number(&l), 4);
        // Borracha no 2: o 3 continua na tela, então o próximo é 4 (sem repetir).
        let apagado = l.remove(1);
        assert_eq!(next_number(&l), 4);
        l.insert(1, apagado); // desfazer a borracha
        assert_eq!(next_number(&l), 4);
        // Apagar tudo zera; desfazer o apagar traz de volta.
        let todos = std::mem::take(&mut l);
        assert_eq!(next_number(&l), 1);
        l = todos;
        assert_eq!(next_number(&l), 4);
        // Passou de 99: fica em 99.
        assert_eq!(next_number(&marcadores(99)), 99);
    }

    #[test]
    fn marcador_pinta_circulo_e_digitos_e_fica_vetor() {
        let mut c = Canvas::new(300, 120);
        let mut out = vec![FLOOR; c.len()];
        let m = Stroke::marker(60.0, 60.0, 4.0, 7, 0x00FF00);
        let (r, _) = marker_dims(4.0);
        draw(&mut c, &mut out, &m);
        c.strokes.push(m);
        assert_eq!(out[60 * 300 + (60.0 + r) as usize] >> 24, 255); // anel
        assert_eq!(out[60 * 300 + 20], FLOOR); // fora do círculo
        let o = &out;
        let dentro = (45..75).flat_map(|y| (45..75).map(move |x| o[y * 300 + x])).filter(|&p| p == 0xFF00FF00).count();
        assert!(dentro > 30, "dígito não apareceu: {dentro}");
        // Refazer a partir da lista dá a mesma imagem.
        let mut b = vec![0u32; c.len()];
        c.rebuild(&mut b, FLOOR);
        assert_eq!(out, b);
        // A caixa cobre o círculo e a borracha acerta o anel.
        let bb = c.strokes[0].bbox(&mut Vec::new());
        assert!(bb.x0 <= (60.0 - r) as i32 && bb.x1 >= (60.0 + r) as i32);
        assert!(c.strokes[0].hit(60.0 + r, 60.0, 1.0, &mut Vec::new()));
    }

    #[test]
    fn digitos_cabem_dentro_do_circulo_em_todos_os_tamanhos() {
        for size in [2.0, 4.0, 8.0, 16.0, 40.0] {
            let (r, w) = marker_dims(size);
            for num in 1..=MAX_NUMBER {
                let mut segs = Vec::new();
                number_segments(0.0, 0.0, r, w, num, &mut segs);
                // Os dígitos são os segmentos depois do anel.
                let ring = ((r * 1.5) as usize).clamp(24, 96);
                assert!(segs.len() > ring);
                for (a, b) in &segs[ring..] {
                    for p in [a, b] {
                        assert!(p[0].hypot(p[1]) + w < r - w, "{num} a {size}px: ({}, {}) sai do anel", p[0], p[1]);
                    }
                }
            }
        }
    }

    /// Texto falso: um bloco cheio de 20x10 com borda suave, no canto (x, y).
    fn text_stroke(x: f32, y: f32, rgb: u32) -> Stroke {
        let (w, h) = (20, 10);
        let cov = (0..w * h).map(|i| if i % w == 0 { 128 } else { 255 }).collect();
        Stroke { tool: Tool::Text, rgb, alpha: 255, pts: vec![[x, y, 0.0]], num: 0, text: Some(std::sync::Arc::new(TextMask { w, h, cov, ..Default::default() })), area: Default::default() }
    }

    #[test]
    fn texto_pinta_a_caixa_e_a_borracha_acha() {
        let mut c = Canvas::new(128, 64);
        let mut out = vec![FLOOR; c.len()];
        let t = text_stroke(70.0, 30.0, 0x0000FF);
        draw(&mut c, &mut out, &t);
        assert_eq!(out[35 * 128 + 75], 0xFF0000FF);
        assert_eq!(out[35 * 128 + 70] >> 24, 128); // borda suave
        assert_eq!(out[35 * 128 + 69], FLOOR);
        let mut sc = Vec::new();
        assert_eq!(t.bbox(&mut sc), Rect { x0: 70, y0: 30, x1: 90, y1: 40 });
        assert!(t.hit(95.0, 35.0, 6.0, &mut sc)); // borracha encosta pela direita
        assert!(!t.hit(100.0, 35.0, 6.0, &mut sc));
        assert_eq!(c.live_bytes(), 0);
    }

    #[test]
    fn desfazer_e_refazer_texto_pela_caixa() {
        // Texto cruzando a borda de bloco (x 60-80) por cima de um traço.
        let (w, h) = (128, 64);
        let mut c = Canvas::new(w, h);
        c.strokes.push(pen(0xFF0000, 255, vec![[0.0, 35.0, 4.0], [127.0, 35.0, 4.0]]));
        let mut out = vec![FLOOR; c.len()];
        c.rebuild(&mut out, FLOOR);
        let before = out.clone();
        let t = text_stroke(60.0, 30.0, 0x00FF00);
        c.strokes.push(t.clone());
        let r = c.rebuild_region(&mut out, t.bbox(&mut Vec::new()), FLOOR);
        let mut full = vec![0u32; c.len()];
        c.rebuild(&mut full, FLOOR);
        assert!(out == full, "refazer a caixa = refazer tudo");
        assert_ne!(out, before);
        // Desfazer: sai da lista e a caixa volta a ser só o traço de baixo.
        c.strokes.pop();
        c.rebuild_region(&mut out, r, FLOOR);
        assert!(out == before);
    }

    #[test]
    fn fade_fica_some_e_sai_da_lista() {
        assert_eq!(fade_level(0), Some(255));
        assert_eq!(fade_level(FADE_HOLD_MS - 1), Some(255));
        assert_eq!(fade_level(FADE_HOLD_MS + FADE_MS / 2), Some(128));
        assert_eq!(fade_level(FADE_HOLD_MS + FADE_MS), None);
        let mut prev = 255;
        for t in (FADE_HOLD_MS..FADE_HOLD_MS + FADE_MS).step_by(16) {
            let k = fade_level(t).unwrap();
            assert!(k <= prev && k > 0);
            prev = k;
        }

        let mut c = Canvas::new(128, 64);
        let mut out = vec![FLOOR; c.len()];
        c.strokes.push(pen(0xFF0000, 255, vec![[0.0, 10.0, 3.0], [127.0, 10.0, 3.0]]));
        let s = pen(0x0000FF, 255, vec![[20.0, 40.0, 4.0], [60.0, 40.0, 4.0]]);
        c.fading.push(Fading { stroke: s.clone(), born: 1000, k: 255 });
        c.rebuild(&mut out, FLOOR);
        assert_eq!(out[40 * 128 + 40], 0xFF0000FF);
        // Parado: nada muda, e o próximo passo é quando começa a sumir.
        assert_eq!(c.fade_tick(1000 + 1000), (Rect::EMPTY, Some(FADE_HOLD_MS - 1000)));
        // Sumindo: muda só a caixa do traço, de quadro em quadro.
        let (dirty, next) = c.fade_tick(1000 + FADE_HOLD_MS + FADE_MS / 2);
        assert_eq!(dirty, s.bbox(&mut Vec::new()));
        assert_eq!(next, Some(FADE_FRAME_MS));
        c.rebuild_region(&mut out, dirty, FLOOR);
        assert_eq!(out[40 * 128 + 40] >> 24, 128);
        assert_eq!(out[10 * 128 + 40], 0xFFFF0000); // o traço fixo fica
        // Acabou: sai da lista, a caixa volta ao fundo e o timer pode morrer.
        let (dirty, next) = c.fade_tick(1000 + FADE_HOLD_MS + FADE_MS);
        assert!(c.fading.is_empty() && next.is_none());
        c.rebuild_region(&mut out, dirty, FLOOR);
        assert_eq!(out[40 * 128 + 40], FLOOR);
        assert_eq!(c.fade_tick(99_999), (Rect::EMPTY, None));
    }

    /// O quadro do fade pela camada-base guardada é igual a refazer tudo, também
    /// com um traço fixo novo no meio do fade; a base some quando o fade acaba.
    #[test]
    fn quadro_do_fade_pela_base_igual_a_refazer_tudo() {
        let (w, h) = (300, 200);
        let mut c = Canvas::new(w, h);
        let mut out = vec![FLOOR; c.len()];
        let mut full = vec![FLOOR; c.len()];
        c.strokes.push(pen(0xFF0000, 255, vec![[0.0, 50.0, 5.0], [299.0, 60.0, 5.0]]));
        c.strokes.push(Stroke { tool: Tool::Marker, ..pen(0xFFFF00, 90, vec![[150.0, 0.0, 9.0], [150.0, 199.0, 9.0]]) });
        c.fading.push(Fading { stroke: pen(0x0000FF, 255, vec![[20.0, 40.0, 4.0], [200.0, 70.0, 4.0]]), born: 0, k: 255 });
        c.fading.push(Fading { stroke: pen(0x00FF00, 200, vec![[100.0, 150.0, 6.0], [280.0, 20.0, 6.0]]), born: 200, k: 255 });
        c.fading.push(Fading { stroke: pen(0xFFFFFF, 255, vec![[10.0, 190.0, 3.0], [40.0, 180.0, 3.0]]), born: 100, k: 255 });
        c.rebuild(&mut out, FLOOR);
        let mut now = FADE_HOLD_MS + 50;
        let mut frames = 0;
        while !c.fading.is_empty() {
            if frames == 4 {
                // Traço fixo novo (fade desligado no meio): a base fica velha.
                // Dentro da caixa do primeiro, que muda todo quadro (fora dela, o
                // traço novo fica por cima do fade até o fim, como já era).
                c.strokes.push(pen(0xFF00FF, 255, vec![[0.0, 45.0, 4.0], [120.0, 55.0, 4.0]]));
                let st = c.strokes.last().unwrap().clone();
                let r = c.raster_stroke(&out, &st);
                c.compose(&mut out, r, 0xFF00FF, 255);
                c.end_live();
            }
            let (_, next) = c.fade_frame(&mut out, now, FLOOR);
            // Referência num canvas à parte (o rebuild solta a base).
            let mut r = Canvas::new(w, h);
            r.strokes = c.strokes.clone();
            r.fading = c.fading.iter().map(|f| Fading { stroke: f.stroke.clone(), born: f.born, k: f.k }).collect();
            r.rebuild(&mut full, FLOOR);
            assert!(out == full, "quadro {frames} diferente de refazer tudo");
            if frames < 3 {
                assert!(c.base_bytes() > 0, "a base fica enquanto algo some");
            }
            now += next.unwrap_or(FADE_FRAME_MS).min(FADE_FRAME_MS);
            frames += 1;
        }
        assert_eq!(c.base_bytes(), 0, "a base é solta no fim do fade");
    }

    /// Caixa grande (mais que PAR_FADE_AREA): o quadro vai em faixas paralelas.
    #[test]
    fn quadro_do_fade_em_faixas_igual_a_refazer_tudo() {
        let (w, h) = (1700, 1300);
        let mut c = Canvas::new(w, h);
        let mut out = vec![FLOOR; c.len()];
        let mut full = vec![FLOOR; c.len()];
        c.strokes = (0..40).map(|i| pen(0xFF0000 + i, 255, vec![[0.0, 30.0 * i as f32, 5.0], [1699.0, 1299.0 - 30.0 * i as f32, 5.0]])).collect();
        c.fading.push(Fading { stroke: pen(0x0000FF, 200, vec![[10.0, 10.0, 8.0], [1690.0, 1290.0, 8.0]]), born: 0, k: 255 });
        c.fading.push(Fading { stroke: pen(0x00FF00, 255, vec![[10.0, 1290.0, 6.0], [1690.0, 10.0, 6.0]]), born: 2000, k: 255 });
        c.rebuild(&mut out, FLOOR);
        for f in 0..6 {
            c.fade_frame(&mut out, FADE_HOLD_MS + 40 + f * 90, FLOOR);
            let mut r = Canvas::new(w, h);
            r.strokes = c.strokes.clone();
            r.fading = c.fading.iter().map(|f| Fading { stroke: f.stroke.clone(), born: f.born, k: f.k }).collect();
            r.rebuild(&mut full, FLOOR);
            assert!(out == full, "quadro {f} diferente de refazer tudo");
        }
    }

    #[test]
    fn caixa_guardada_e_borracha_pela_caixa() {
        let s = pen(0xFF0000, 255, vec![[10.0, 10.0, 3.0], [100.0, 10.0, 3.0]]);
        let mut sc = Vec::new();
        let b = s.bbox(&mut sc);
        assert_eq!(s.bbox(&mut sc), b);
        assert!(s.hit(50.0, 12.0, 2.0, &mut sc));
        assert!(!s.hit(50.0, 40.0, 2.0, &mut sc));
        assert!(!s.hit(500.0, 500.0, 20.0, &mut sc));
    }

    #[test]
    fn fade_marca_o_proximo_passo_pelo_mais_adiantado() {
        let mut c = Canvas::new(64, 64);
        let s = pen(0xFFFFFF, 255, vec![[10.0, 10.0, 2.0]]);
        c.fading.push(Fading { stroke: s.clone(), born: 0, k: 255 });
        c.fading.push(Fading { stroke: s, born: 2500, k: 255 });
        // O primeiro está sumindo: quadro a quadro, mesmo com o outro parado.
        assert_eq!(c.fade_tick(FADE_HOLD_MS + 10).1, Some(FADE_FRAME_MS));
        // O primeiro sumiu: espera o segundo começar.
        let (_, next) = c.fade_tick(FADE_HOLD_MS + FADE_MS);
        assert_eq!(c.fading.len(), 1);
        assert_eq!(next, Some(2500 + FADE_HOLD_MS - (FADE_HOLD_MS + FADE_MS)));
    }

    #[test]
    fn troca_de_fundo() {
        let mut out = vec![FLOOR, 0xFF00FF00, FLOOR];
        swap_floor(&mut out, FLOOR, 0);
        assert_eq!(out, vec![0, 0xFF00FF00, 0]);
    }
}
