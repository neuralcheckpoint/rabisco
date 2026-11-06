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
    pub pts: Vec<Pt>,
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
            Tool::Pen | Tool::Marker | Tool::Eraser => {
                if p.len() == 1 {
                    out.push((p[0], p[0]));
                }
                for w in p.windows(2) {
                    out.push((w[0], w[1]));
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
        self.segments(scratch);
        scratch.iter().any(|(a, b)| {
            let (t, d) = seg_dist(a, b, x, y);
            d < a[2] + (b[2] - a[2]) * t + r
        })
    }

    /// Caixa que o traço ocupa na tela (com a ponta da seta).
    pub fn bbox(&self, scratch: &mut Vec<(Pt, Pt)>) -> Rect {
        self.segments(scratch);
        scratch.iter().fold(Rect::EMPTY, |r, (a, b)| r.union(&seg_box(a, b, i32::MAX, i32::MAX)))
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

/// Fundo do modo desenho: quase invisível, mas a janela pega o clique.
pub const FLOOR: u32 = 0x0100_0000;

const TS: i32 = 64;
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
    pooling: bool,
    pub strokes: Vec<Stroke>,
    segs: Vec<(Pt, Pt)>,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Canvas {
        let (w, h) = (w.max(1), h.max(1));
        let tw = (w + TS - 1) / TS;
        let th = (h + TS - 1) / TS;
        let mut tiles = Vec::new();
        tiles.resize_with((tw * th) as usize, || None);
        Canvas { w, h, tw, tiles, used: Vec::new(), pool: Vec::new(), pooling: false, strokes: Vec::new(), segs: Vec::new() }
    }

    #[cfg(test)]
    pub fn full(&self) -> Rect {
        Rect { x0: 0, y0: 0, x1: self.w, y1: self.h }
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
            for y in 0..(self.h - y0).min(TS) {
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
        let mut segs = std::mem::take(&mut self.segs);
        s.segments(&mut segs);
        let mut r = Rect::EMPTY;
        for &(a, b) in &segs {
            r = r.union(&self.raster_seg_in(out, a, b, clip));
        }
        self.segs = segs;
        r
    }

    /// Zera a cobertura (forma sendo redesenhada enquanto arrasta).
    pub fn clear_live(&mut self) {
        for &i in &self.used {
            self.tiles[i].as_mut().unwrap().mask.fill(0);
        }
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
    pub fn rebuild(&mut self, out: &mut [u32], floor: u32) {
        self.end_live();
        self.pooling = true;
        out.fill(floor);
        let strokes = std::mem::take(&mut self.strokes);
        for s in &strokes {
            let r = self.raster_stroke(out, s);
            self.compose(out, r, s.rgb, s.alpha);
            self.end_live();
        }
        self.strokes = strokes;
        self.pooling = false;
        self.pool = Vec::new();
    }

    /// Redesenha só a área r (desfazer um traço, borracha): pinta o fundo nela
    /// e refaz, recortados, só os traços que passam por ali. Devolve a área.
    pub fn rebuild_region(&mut self, out: &mut [u32], r: Rect, floor: u32) -> Rect {
        let r = r.clip(self.w, self.h);
        if r.is_empty() {
            return r;
        }
        self.end_live();
        for y in r.y0..r.y1 {
        self.pooling = true;
            let row = (y * self.w) as usize;
            out[row + r.x0 as usize..row + r.x1 as usize].fill(floor);
        }
        let strokes = std::mem::take(&mut self.strokes);
        let mut scratch = Vec::new();
        for s in &strokes {
            if s.bbox(&mut scratch).intersect(&r).is_empty() {
                continue;
            }
            let rr = self.raster_stroke_in(out, s, r);
            self.compose(out, rr, s.rgb, s.alpha);
            self.end_live();
        }
        self.strokes = strokes;
        self.pooling = false;
        self.pool = Vec::new();
        r
    }
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

    fn draw(c: &mut Canvas, out: &mut [u32], s: &Stroke) {
        let r = c.raster_stroke(out, s);
        c.compose(out, r, s.rgb, s.alpha);
        c.end_live();
    }

    fn pen(rgb: u32, alpha: u8, pts: Vec<Pt>) -> Stroke {
        Stroke { tool: Tool::Pen, rgb, alpha, pts }
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
    fn refazer_regiao_igual_a_refazer_tudo() {
        let mut c = Canvas::new(300, 200);
        let mut out = vec![FLOOR; c.len()];
        let mk = |tool, rgb, alpha, pts: Vec<Pt>| Stroke { tool, rgb, alpha, pts };
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

    #[test]
    fn troca_de_fundo() {
        let mut out = vec![FLOOR, 0xFF00FF00, FLOOR];
        swap_floor(&mut out, FLOOR, 0);
        assert_eq!(out, vec![0, 0xFF00FF00, 0]);
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    /// `cargo test --release bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn custo_de_refazer_a_tela() {
        let (w, h) = (6400, 2160);
        let mut c = Canvas::new(w, h);
        for i in 0..200 {
            let y = 100.0 + (i * 9 % 1900) as f32;
            let pts = (0..80).map(|k| [300.0 + k as f32 * 20.0, y + (k as f32 / 6.0).sin() * 40.0, 3.0]).collect();
            c.strokes.push(Stroke { tool: Tool::Pen, rgb: 0xFF0000, alpha: 255, pts });
        }
        let t = Instant::now();
        let mut out = vec![0u32; c.len()];
        let alloc = t.elapsed();
        let t = Instant::now();
        out.fill(FLOOR);
        let fill = t.elapsed();
        let t = Instant::now();
        c.rebuild(&mut out, FLOOR);
        let rebuild = t.elapsed();
        println!("alocar {alloc:?} · preencher {fill:?} · refazer com 200 traços {rebuild:?}");
    }
}
