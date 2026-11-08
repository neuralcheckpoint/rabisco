//! Barra flutuante: layout, desenho dos botões e ícones, e o que cada um diz.
//! Os ícones são cápsulas como os traços, desenhados num quadrado de 24x24
//! e escalados pelo DPI, então não há imagem nem fonte embutida.

use crate::canvas::{Pt, Rect, Tool, coverage, ellipse, mul255, number_segments, over, premul, seg_box};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Item {
    Grip,
    Close,
    Mouse,
    Tool(Tool),
    Color(usize),
    Size(usize),
    Undo,
    Redo,
    Clear,
    Hide,
    /// Liga e alterna as guias de enquadramento.
    Guides,
    /// Salvar print com as anotações.
    Save,
    /// Gravar vídeo (● abre o menu; ■ para).
    Rec,
    /// Liga/desliga o modo fade (traço novo some sozinho).
    Fade,
    /// Holofote: escurece tudo menos em volta do cursor.
    Spot,
    /// Lupa em volta do cursor.
    Zoom,
}

/// Espessuras da caneta (px) nos 4 botões.
pub const SIZES: [f32; 4] = [2.0, 4.0, 8.0, 16.0];

pub const COLOR_NAMES: [&str; 9] =
    ["Vermelho", "Laranja", "Amarelo", "Verde", "Ciano", "Azul", "Magenta", "Branco", "Preto"];

pub struct State<'a> {
    pub draw: bool,
    pub tool: Tool,
    pub rgb: u32,
    pub size: f32,
    pub hidden: bool,
    /// Alguma guia de enquadramento está ligada.
    pub guides: bool,
    /// Modo fade ligado.
    pub fade: bool,
    pub hover: Option<Item>,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Gravando (o botão vira ■).
    pub rec: bool,
    pub colors: &'a [u32],
}

pub struct Layout {
    pub w: i32,
    pub h: i32,
    pub s: f32,
    pub items: Vec<(Item, Rect)>,
    seps: Vec<i32>,
}

const W: f32 = 80.0;
const PAD: f32 = 6.0;
const INNER: f32 = W - 2.0 * PAD;

pub fn layout(s: f32) -> Layout {
    let mut items = Vec::new();
    let mut seps = Vec::new();
    let px = |v: f32| (v * s).round() as i32;
    let mut put = |item: Item, x: f32, y: f32, w: f32, h: f32| {
        items.push((item, Rect { x0: px(x), y0: px(y), x1: px(x + w), y1: px(y + h) }));
    };
    let mut y = PAD;
    put(Item::Grip, PAD, y, INNER - 20.0, 18.0);
    put(Item::Close, PAD + INNER - 20.0, y, 20.0, 18.0);
    y += 20.0;

    let tools = [
        Item::Mouse,
        Item::Tool(Tool::Pen),
        Item::Tool(Tool::Marker),
        Item::Tool(Tool::Eraser),
        Item::Tool(Tool::Line),
        Item::Tool(Tool::Arrow),
        Item::Tool(Tool::Rect),
        Item::Tool(Tool::Ellipse),
        Item::Tool(Tool::Number),
        Item::Tool(Tool::Text),
        Item::Guides,
        Item::Fade,
    ];
    let half = INNER / 2.0;
    for (i, t) in tools.into_iter().enumerate() {
        put(t, PAD + (i % 2) as f32 * half, y + (i / 2) as f32 * 32.0, half, 32.0);
    }
    y += 6.0 * 32.0;

    let mut sep = |y: &mut f32| {
        seps.push(px(*y + 4.5));
        *y += 9.0;
    };
    sep(&mut y);
    let third = INNER / 3.0;
    for i in 0..9 {
        put(Item::Color(i), PAD + (i % 3) as f32 * third, y + (i / 3) as f32 * 22.0, third, 22.0);
    }
    y += 3.0 * 22.0;
    sep(&mut y);
    let quarter = INNER / 4.0;
    for i in 0..4 {
        put(Item::Size(i), PAD + i as f32 * quarter, y, quarter, 24.0);
    }
    y += 24.0;
    sep(&mut y);
    for (i, a) in [Item::Undo, Item::Redo, Item::Clear, Item::Hide, Item::Spot, Item::Zoom].into_iter().enumerate() {
        put(a, PAD + (i % 2) as f32 * half, y + (i / 2) as f32 * 30.0, half, 30.0);
    }
    y += 90.0;
    put(Item::Save, PAD, y, half, 30.0);
    put(Item::Rec, PAD + half, y, half, 30.0);
    y += 30.0 + PAD;
    Layout { w: px(W), h: px(y), s, items, seps }
}

/// Botão em (x, y), em pixels da barra. `None` = fundo (arrasta a barra).
pub fn hit(l: &Layout, x: i32, y: i32) -> Option<Item> {
    l.items.iter().find(|(_, r)| r.contains(x, y)).map(|(i, _)| *i)
}

pub fn label(item: Item) -> String {
    match item {
        Item::Grip => "Arraste para mover".into(),
        Item::Close => "Guardar na bandeja · sair pelo ícone da bandeja (Ctrl+Shift+Q)".into(),
        Item::Mouse => "Mouse: clicar nos programas (Esc)".into(),
        Item::Tool(t) => match t {
            Tool::Pen => "Caneta (P)",
            Tool::Marker => "Marca-texto (M)",
            Tool::Eraser => "Borracha: apaga o traço inteiro (E)",
            Tool::Line => "Linha (L) · Shift trava em 45°",
            Tool::Arrow => "Seta (A) · Shift trava em 45°",
            Tool::Rect => "Retângulo (R) · Shift deixa quadrado",
            Tool::Ellipse => "Elipse (O) · Shift deixa círculo",
            Tool::Number => "Marcador numerado (N): cada clique põe 1, 2, 3… na cor atual",
            Tool::Text => "Texto (T): clique e digite · Enter quebra a linha · Esc ou clicar fora confirma",
        }
        .into(),
        Item::Color(i) => format!("{} ({})", COLOR_NAMES[i], i + 1),
        Item::Size(i) => format!(
            "{} · {} px (roda do mouse ajusta)",
            ["Fino", "Médio", "Grosso", "Bem grosso"][i],
            SIZES[i]
        ),
        Item::Undo => "Desfazer (Ctrl+Z)".into(),
        Item::Redo => "Refazer (Ctrl+Y)".into(),
        Item::Clear => "Apagar tudo (C)".into(),
        Item::Hide => "Esconder/mostrar desenhos (Ctrl+Shift+H)".into(),
        Item::Guides => "Guias (G): terços · 16:9 e área segura · terços no 16:9 · desligar".into(),
        Item::Save => "Salvar print com as anotações (Ctrl+S, ou Ctrl+Shift+S em qualquer programa)".into(),
        Item::Rec => "Gravar vídeo (Ctrl+Shift+R): o menu escolhe monitor, janela ou região e o som · gravando, para".into(),
        Item::Fade => "Fade (F): os traços novos somem sozinhos 3 s depois".into(),
        Item::Spot => "Holofote (S): escurece a tela menos em volta do cursor · roda muda o tamanho · Esc sai".into(),
        Item::Zoom => "Zoom (Z): lupa em volta do cursor · roda de 1,5x a 4x · Esc sai".into(),
    }
}

const PANEL: u32 = 0x1F2125;
const EDGE: u32 = 0x3A3D42;
const HOVER: u32 = 0x33363C;
const SELECTED: u32 = 0x4A5059;
const FG: u32 = 0xE8EAED;
const DIM: u32 = 0x6B6F75;
const GRIP: u32 = 0x8A8F96;
const REC: u32 = 0xE61E1E;

/// Dígitos 3x5 do contador (bit 2 = coluna da esquerda), e o ':' no fim.
const DIGITS: [[u8; 5]; 11] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
    [0, 2, 0, 2, 0],
];

/// Lado do "pixel" dos dígitos do contador.
fn digit_px(s: f32) -> i32 {
    (2.0 * s).round().max(1.0) as i32
}

/// Tamanho do contador de gravação: o tempo (`chars` caracteres) e o ponto de diâmetro `d`.
pub fn badge_size(s: f32, chars: usize, d: i32) -> (i32, i32) {
    let px = digit_px(s);
    let tw = chars as i32 * 4 * px - px;
    ((4.0 * s).round() as i32 + tw + (6.0 * s).round() as i32 + d, d.max(5 * px) + 2 * s.round().max(1.0) as i32)
}

/// Pinta o contador (BGRA pré-multiplicado): "mm:ss" em branco com sombra e o
/// ponto vermelho na direita. Sem fonte: os dígitos são 3x5.
pub fn paint_badge(buf: &mut [u32], w: i32, h: i32, s: f32, text: &str, d: i32) {
    buf.fill(0);
    let px = digit_px(s);
    let (x0, y0) = ((4.0 * s).round() as i32, (h - 5 * px) / 2);
    let mut put = |x: i32, y: i32, c: u32| {
        if x >= 0 && y >= 0 && x < w && y < h {
            buf[(y * w + x) as usize] = c;
        }
    };
    let sh = px.max(2) / 2;
    // A sombra primeiro; o branco vem por cima.
    for (off, color) in [(sh, 0xA000_0000u32), (0, 0xFFFF_FFFF)] {
        for (i, ch) in text.chars().enumerate() {
            let g = match ch {
                '0'..='9' => DIGITS[ch as usize - '0' as usize],
                ':' => DIGITS[10],
                _ => continue,
            };
            for (row, bits) in g.iter().enumerate() {
                for col in 0..3 {
                    if bits >> (2 - col) & 1 == 0 {
                        continue;
                    }
                    let (gx, gy) = (x0 + i as i32 * 4 * px + col * px + off, y0 + row as i32 * px + off);
                    for k in 0..px * px {
                        put(gx + k % px, gy + k / px, color);
                    }
                }
            }
        }
    }
    let (cx, cy, r) = ((w - d) as f32 + d as f32 / 2.0, h as f32 / 2.0, d as f32 / 2.0);
    for y in 0..h {
        for x in (w - d).max(0)..w {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let a = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            if a > 0.0 {
                let c = |v: f32| (v * a) as u32;
                buf[(y * w + x) as usize] = c(255.0) << 24 | c(230.0) << 16 | c(30.0) << 8 | c(30.0);
            }
        }
    }
}

/// Pinta em `buf` (w*h, BGRA pré-multiplicado). Ícones passam por uma máscara
/// de cobertura máxima para as emendas das linhas não engrossarem.
struct Painter<'a> {
    buf: &'a mut [u32],
    mask: Vec<u8>,
    dirty: Rect,
    w: i32,
    h: i32,
}

impl Painter<'_> {
    fn seg(&mut self, a: Pt, b: Pt) {
        let r = seg_box(&a, &b, self.w, self.h);
        if r.is_empty() {
            return;
        }
        self.dirty = self.dirty.union(&r);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let c = coverage(&a, &b, x, y);
                if c > 0.0 {
                    let m = &mut self.mask[(y * self.w + x) as usize];
                    *m = (*m).max((c * 255.0 + 0.5) as u8);
                }
            }
        }
    }

    /// Aplica a máscara com a cor e limpa.
    fn ink(&mut self, rgb: u32) {
        let r = self.dirty;
        self.dirty = Rect::EMPTY;
        if r.is_empty() {
            return;
        }
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let i = (y * self.w + x) as usize;
                let m = self.mask[i] as u32;
                if m != 0 {
                    self.buf[i] = over(premul(rgb, m), self.buf[i]);
                    self.mask[i] = 0;
                }
            }
        }
    }

    /// Retângulo arredondado preenchido (distância com sinal, borda suave).
    fn round_rect(&mut self, r: Rect, rad: f32, rgb: u32, alpha: u32, stroke: Option<u32>) {
        let (x0, y0, x1, y1) = (r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32);
        for y in r.y0.max(0)..r.y1.min(self.h) {
            for x in r.x0.max(0)..r.x1.min(self.w) {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let qx = (x0 + rad - px).max(px - (x1 - rad)).max(0.0);
                let qy = (y0 + rad - py).max(py - (y1 - rad)).max(0.0);
                let d = (qx * qx + qy * qy).sqrt() - rad; // < 0 dentro
                let fill = (0.5 - d).clamp(0.0, 1.0);
                if fill <= 0.0 {
                    continue;
                }
                let i = (y * self.w + x) as usize;
                self.buf[i] = over(premul(rgb, mul255(alpha, (fill * 255.0) as u32)), self.buf[i]);
                if let Some(edge) = stroke {
                    let e = (1.0 - (d + 0.75).abs()).clamp(0.0, 1.0) * fill;
                    if e > 0.0 {
                        self.buf[i] = over(premul(edge, (e * 255.0) as u32), self.buf[i]);
                    }
                }
            }
        }
    }
}

/// Aviso de "print salvo" (✓) ou de falha (✗): tamanho em pixels.
pub fn toast_size(s: f32) -> (i32, i32) {
    ((96.0 * s).round() as i32, (64.0 * s).round() as i32)
}

pub fn paint_toast(buf: &mut [u32], s: f32, ok: bool) {
    let (w, h) = toast_size(s);
    buf.fill(0);
    let mut p = Painter { buf, mask: vec![0; (w * h) as usize], dirty: Rect::EMPTY, w, h };
    p.round_rect(Rect { x0: 0, y0: 0, x1: w, y1: h }, 14.0 * s, PANEL, 242, Some(EDGE));
    let m = |u: f32, v: f32| -> Pt { [u * s, v * s, 3.4 * s] };
    if ok {
        p.seg(m(30.0, 33.0), m(42.0, 45.0));
        p.seg(m(42.0, 45.0), m(66.0, 19.0));
        p.ink(0x43A047);
    } else {
        p.seg(m(34.0, 19.0), m(62.0, 45.0));
        p.seg(m(62.0, 19.0), m(34.0, 45.0));
        p.ink(0xE53935);
    }
}

pub fn paint(l: &Layout, st: &State, buf: &mut [u32]) {
    let s = l.s;
    buf.fill(0);
    let mut p = Painter { buf, mask: vec![0; (l.w * l.h) as usize], dirty: Rect::EMPTY, w: l.w, h: l.h };
    p.round_rect(Rect { x0: 0, y0: 0, x1: l.w, y1: l.h }, 10.0 * s, PANEL, 242, Some(EDGE));
    for &y in &l.seps {
        let (a, b) = ([PAD * s + 4.0 * s, y as f32, 0.5 * s], [l.w as f32 - PAD * s - 4.0 * s, y as f32, 0.5 * s]);
        p.seg(a, b);
        p.ink(EDGE);
    }

    for &(item, r) in &l.items {
        let selected = match item {
            Item::Mouse => !st.draw,
            Item::Tool(t) => st.draw && st.tool == t,
            Item::Size(i) => (st.size - SIZES[i]).abs() < 0.01,
            Item::Hide => st.hidden,
            Item::Guides => st.guides,
            Item::Fade => st.fade,
            _ => false,
        };
        let inset = Rect { x0: r.x0 + (2.0 * s) as i32, y0: r.y0 + (2.0 * s) as i32, x1: r.x1 - (2.0 * s) as i32, y1: r.y1 - (2.0 * s) as i32 };
        let has_bg = !matches!(item, Item::Grip | Item::Color(_));
        if has_bg && selected {
            p.round_rect(inset, 6.0 * s, SELECTED, 255, None);
        } else if has_bg && st.hover == Some(item) {
            p.round_rect(inset, 6.0 * s, HOVER, 255, None);
        }
        let (cx, cy) = ((r.x0 + r.x1) as f32 / 2.0, (r.y0 + r.y1) as f32 / 2.0);
        let enabled = match item {
            Item::Undo => st.can_undo,
            Item::Redo => st.can_redo,
            _ => true,
        };
        icon(&mut p, item, cx, cy, s, if enabled { FG } else { DIM }, st);
    }
}

/// Ícone do programa (bandeja, atalho), `n`x`n`, BGRA pré-multiplicado:
/// quadrado escuro com a caneta da barra e um rabisco vermelho.
pub fn app_icon(n: i32) -> Vec<u32> {
    let mut buf = vec![0u32; (n * n) as usize];
    let s = n as f32 / 24.0;
    let mut p = Painter { buf: &mut buf, mask: vec![0; (n * n) as usize], dirty: Rect::EMPTY, w: n, h: n };
    p.round_rect(Rect { x0: 0, y0: 0, x1: n, y1: n }, 5.0 * s, PANEL, 255, None);
    // Traço mais grosso que na barra para ler em 16 px.
    let r = if n <= 24 { 1.6 } else { 1.3 };
    let m = |u: f32, v: f32| -> Pt { [u * s, v * s, r * s] };
    let pen = [(15.5, 3.5), (20.5, 8.5), (10.0, 19.0), (5.0, 19.0), (5.0, 14.0)];
    for i in 0..pen.len() {
        let (a, b) = (pen[i], pen[(i + 1) % pen.len()]);
        p.seg(m(a.0, a.1), m(b.0, b.1));
    }
    p.ink(FG);
    let wave = [(3.5, 21.5), (8.0, 20.5), (12.5, 21.5), (17.0, 20.5), (20.5, 21.0)];
    for w in wave.windows(2) {
        p.seg(m(w[0].0, w[0].1), m(w[1].0, w[1].1));
    }
    p.ink(0xE53935);
    buf
}

/// Desenha o ícone de `item` centrado em (cx, cy), num quadrado de 24 unidades.
fn icon(p: &mut Painter, item: Item, cx: f32, cy: f32, s: f32, fg: u32, st: &State) {
    let m = |u: f32, v: f32, r: f32| -> Pt { [cx + (u - 12.0) * s, cy + (v - 12.0) * s, r * s] };
    let line = |p: &mut Painter, pts: &[(f32, f32)], closed: bool, r: f32| {
        for w in pts.windows(2) {
            p.seg(m(w[0].0, w[0].1, r), m(w[1].0, w[1].1, r));
        }
        if closed && pts.len() > 2 {
            let (a, b) = (pts[pts.len() - 1], pts[0]);
            p.seg(m(a.0, a.1, r), m(b.0, b.1, r));
        }
    };
    let dot = |p: &mut Painter, u: f32, v: f32, r: f32| p.seg(m(u, v, r), m(u, v, r));
    const R: f32 = 0.95;
    match item {
        Item::Grip => {
            for u in [8.0, 12.0, 16.0] {
                for v in [10.0, 14.0] {
                    dot(p, u, v, 1.1);
                }
            }
            p.ink(GRIP);
        }
        Item::Close => {
            line(p, &[(8.0, 8.0), (16.0, 16.0)], false, 0.9);
            line(p, &[(16.0, 8.0), (8.0, 16.0)], false, 0.9);
            p.ink(GRIP);
        }
        Item::Mouse => {
            line(p, &[(8.0, 4.0), (8.0, 19.0), (11.5, 15.5), (14.0, 21.0), (16.5, 20.0), (14.0, 14.5), (19.0, 14.5)], true, R);
            p.ink(fg);
        }
        Item::Tool(Tool::Pen) => {
            line(p, &[(15.0, 5.0), (19.0, 9.0), (9.0, 19.0), (5.0, 19.0), (5.0, 15.0)], true, R);
            line(p, &[(12.5, 7.5), (16.5, 11.5)], false, R);
            p.ink(fg);
        }
        Item::Tool(Tool::Marker) => {
            line(p, &[(10.0, 14.0), (17.0, 7.0)], false, 3.2);
            line(p, &[(6.5, 17.5), (9.0, 15.0)], false, 1.4);
            p.ink(fg);
            line(p, &[(4.0, 21.0), (20.0, 21.0)], false, 1.4);
            p.ink(st.rgb);
        }
        Item::Tool(Tool::Eraser) => {
            line(p, &[(4.0, 15.0), (13.0, 6.0), (20.0, 13.0), (11.0, 22.0)], true, R);
            line(p, &[(8.5, 10.5), (15.5, 17.5)], false, R);
            p.ink(fg);
        }
        Item::Tool(Tool::Line) => {
            line(p, &[(5.0, 19.0), (19.0, 5.0)], false, R);
            p.ink(fg);
        }
        Item::Tool(Tool::Arrow) => {
            line(p, &[(5.0, 19.0), (19.0, 5.0)], false, R);
            line(p, &[(11.0, 5.0), (19.0, 5.0), (19.0, 13.0)], false, R);
            p.ink(fg);
        }
        Item::Tool(Tool::Rect) => {
            line(p, &[(4.0, 6.0), (20.0, 6.0), (20.0, 18.0), (4.0, 18.0)], true, R);
            p.ink(fg);
        }
        Item::Tool(Tool::Number) => {
            let mut segs = Vec::new();
            number_segments(12.0, 12.0, 9.0, R, 1, &mut segs);
            for (a, b) in segs {
                p.seg(m(a[0], a[1], R), m(b[0], b[1], R));
            }
            p.ink(fg);
        }
        Item::Guides => {
            line(p, &[(4.0, 9.0), (20.0, 9.0)], false, R);
            line(p, &[(4.0, 15.0), (20.0, 15.0)], false, R);
            line(p, &[(9.0, 4.0), (9.0, 20.0)], false, R);
            line(p, &[(15.0, 4.0), (15.0, 20.0)], false, R);
            p.ink(fg);
        }
        Item::Rec => {
            // ● parado; ■ gravando. Vermelho, como o ponto de gravando.
            if st.rec {
                for v in [8.5, 10.5, 12.0, 13.5, 15.5] {
                    line(p, &[(8.5, v), (15.5, v)], false, 1.6);
                }
            } else {
                dot(p, 12.0, 12.0, 5.5);
            }
            p.ink(REC);
        }
        Item::Save => {
            // Disquete: contorno com o canto cortado, etiqueta e janelinha.
            line(p, &[(5.0, 5.0), (16.0, 5.0), (19.0, 8.0), (19.0, 19.0), (5.0, 19.0)], true, R);
            line(p, &[(8.0, 5.0), (8.0, 9.0), (15.0, 9.0), (15.0, 5.0)], false, R);
            line(p, &[(8.0, 19.0), (8.0, 14.0), (16.0, 14.0), (16.0, 19.0)], false, R);
            p.ink(fg);
        }
        Item::Tool(Tool::Text) => {
            // "T" com serifa embaixo.
            line(p, &[(5.0, 6.5), (5.0, 5.0), (19.0, 5.0), (19.0, 6.5)], false, R);
            line(p, &[(12.0, 5.0), (12.0, 19.0)], false, R);
            line(p, &[(9.0, 19.0), (15.0, 19.0)], false, R);
            p.ink(fg);
        }
        Item::Fade => {
            // Rabisco que se desfaz em pontos cada vez menores.
            line(p, &[(3.5, 15.0), (6.0, 11.0), (8.5, 15.0), (11.0, 11.0)], false, R);
            p.ink(fg);
            for (u, v, r) in [(14.0, 13.0, 0.95), (17.0, 13.0, 0.75), (19.5, 13.0, 0.55)] {
                dot(p, u, v, r);
            }
            p.ink(GRIP);
        }
        Item::Spot => {
            // Círculo claro com raios em volta.
            let mut segs = Vec::new();
            ellipse(12.0, 12.0, 4.5, 4.5, R, 32, &mut segs);
            for (a, b) in segs {
                p.seg(m(a[0], a[1], R), m(b[0], b[1], R));
            }
            for k in 0..8 {
                let t = std::f32::consts::FRAC_PI_4 * k as f32;
                let (c, s2) = (t.cos(), t.sin());
                line(p, &[(12.0 + 7.5 * c, 12.0 + 7.5 * s2), (12.0 + 9.5 * c, 12.0 + 9.5 * s2)], false, R);
            }
            p.ink(fg);
        }
        Item::Zoom => {
            // Lupa com +.
            let mut segs = Vec::new();
            ellipse(10.5, 10.5, 6.0, 6.0, R, 40, &mut segs);
            for (a, b) in segs {
                p.seg(m(a[0], a[1], R), m(b[0], b[1], R));
            }
            line(p, &[(15.0, 15.0), (20.0, 20.0)], false, 1.4);
            line(p, &[(8.0, 10.5), (13.0, 10.5)], false, R);
            line(p, &[(10.5, 8.0), (10.5, 13.0)], false, R);
            p.ink(fg);
        }
        Item::Tool(Tool::Ellipse) => {
            let mut segs = Vec::new();
            ellipse(12.0, 12.0, 8.5, 6.5, R, 48, &mut segs);
            for (a, b) in segs {
                p.seg(m(a[0], a[1], R), m(b[0], b[1], R));
            }
            p.ink(fg);
        }
        Item::Color(i) => {
            let rgb = st.colors[i];
            if rgb == st.rgb {
                dot(p, 12.0, 12.0, 9.0);
                p.ink(FG);
                dot(p, 12.0, 12.0, 7.6);
                p.ink(PANEL);
            }
            dot(p, 12.0, 12.0, 6.4);
            p.ink(GRIP);
            dot(p, 12.0, 12.0, 5.8);
            p.ink(rgb);
        }
        Item::Size(i) => {
            // Bolinha do tamanho relativo, na cor atual (ou clara, se a cor some no fundo).
            let r = [1.5, 2.6, 4.0, 6.0][i];
            dot(p, 12.0, 12.0, r);
            let lum = ((st.rgb >> 16) & 255) * 3 + ((st.rgb >> 8) & 255) * 6 + (st.rgb & 255);
            p.ink(if lum < 600 { FG } else { st.rgb });
        }
        Item::Undo | Item::Redo => {
            let flip = |u: f32| if item == Item::Redo { 24.0 - u } else { u };
            let mut pts = vec![(flip(19.0), 18.0)];
            for k in 0..=16 {
                let t = -std::f32::consts::PI * k as f32 / 16.0;
                pts.push((flip(13.0 + 6.0 * t.cos()), 13.0 + 6.0 * t.sin()));
            }
            line(p, &pts, false, R);
            line(p, &[(flip(4.0), 10.0), (flip(7.0), 14.0), (flip(10.5), 10.5)], false, R);
            p.ink(fg);
        }
        Item::Clear => {
            line(p, &[(5.0, 7.0), (19.0, 7.0)], false, R);
            line(p, &[(10.0, 7.0), (10.0, 4.5), (14.0, 4.5), (14.0, 7.0)], false, R);
            line(p, &[(7.0, 9.0), (8.0, 20.0), (16.0, 20.0), (17.0, 9.0)], false, R);
            line(p, &[(10.5, 11.0), (10.5, 17.0)], false, R);
            line(p, &[(13.5, 11.0), (13.5, 17.0)], false, R);
            p.ink(fg);
        }
        Item::Hide => {
            let mut top = Vec::new();
            let mut bot = Vec::new();
            for k in 0..=18 {
                let u = 3.0 + k as f32;
                let v = 6.0 * (std::f32::consts::PI * k as f32 / 18.0).sin();
                top.push((u, 12.0 - v));
                bot.push((u, 12.0 + v));
            }
            line(p, &top, false, R);
            line(p, &bot, false, R);
            dot(p, 12.0, 12.0, 2.6);
            if st.hidden {
                line(p, &[(4.0, 20.0), (20.0, 4.0)], false, R);
            }
            p.ink(fg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_cabe_e_acha_os_botoes() {
        let l = layout(1.0);
        assert_eq!(l.w, 80);
        // 461 px a 100%: cabe com folga numa tela de 1080p a 150% (720 px úteis).
        assert!(l.h <= 470, "barra alta demais: {}", l.h);
        for &(item, r) in &l.items {
            assert!(r.x1 <= l.w && r.y1 <= l.h, "{item:?} fora da barra");
            assert_eq!(hit(&l, (r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2), Some(item));
        }
        assert_eq!(hit(&l, 1, 1), None); // canto = arrastar
    }

    #[test]
    fn botao_gravar_e_contador() {
        let l = layout(1.0);
        assert!(l.items.iter().any(|&(i, _)| i == Item::Rec));
        let colors = [0xFF0000u32; 9];
        let r = l.items.iter().find(|x| x.0 == Item::Rec).unwrap().1;
        let centro = ((r.y0 + r.y1) / 2 * l.w + (r.x0 + r.x1) / 2) as usize;
        for rec in [false, true] {
            let mut buf = vec![0u32; (l.w * l.h) as usize];
            let st = State { draw: false, tool: Tool::Pen, rgb: 0, size: 4.0, hidden: false, guides: false, fade: false, hover: None, can_undo: false, can_redo: false, rec, colors: &colors };
            paint(&l, &st, &mut buf);
            // ● e ■ vermelhos no meio do botão.
            assert!(buf[centro] >> 16 & 0xFF > 0xC0 && buf[centro] & 0xFF < 0x40, "{rec}: {:08x}", buf[centro]);
        }
        let (w, h) = badge_size(1.0, 5, 12);
        assert_eq!((w, h), (4 + 38 + 6 + 12, 14));
        let mut b = vec![0u32; (w * h) as usize];
        paint_badge(&mut b, w, h, 1.0, "01:23", 12);
        let brancos = b.iter().filter(|&&p| p == 0xFFFF_FFFF).count();
        assert!(brancos > 40, "{brancos} pixels de texto");
        assert_eq!(b[(h / 2 * w + w - 6) as usize] >> 16 & 0xFF, 230);
        // Cada dígito tem um desenho diferente.
        let um = |t: &str| {
            let mut b = vec![0u32; (w * h) as usize];
            paint_badge(&mut b, w, h, 1.0, t, 12);
            b
        };
        let todos: Vec<Vec<u32>> = (0..10).map(|d| um(&format!("0{d}:00"))).collect();
        for i in 0..10 {
            for j in i + 1..10 {
                assert_ne!(todos[i], todos[j], "{i} e {j}");
            }
        }
    }

    #[test]
    fn pinta_sem_estourar() {
        let colors = [0xFF0000u32; 9];
        for s in [1.0, 1.25, 1.5, 2.0] {
            let l = layout(s);
            let mut buf = vec![0u32; (l.w * l.h) as usize];
            let st = State { draw: true, tool: Tool::Pen, rgb: 0xFF0000, size: 4.0, hidden: false, guides: true, fade: true, hover: Some(Item::Undo), can_undo: true, can_redo: false, rec: false, colors: &colors };
            paint(&l, &st, &mut buf);
            assert!(buf[(l.h / 2 * l.w + l.w / 2) as usize] >> 24 > 0);
        }
    }
}
