//! Holofote e zoom (estilo ZoomIt): a parte que não precisa de janela.
//!
//! Holofote: a imagem do monitor é escura (60%) e só um círculo em volta do
//! cursor fica claro. A imagem é pintada uma vez; cada movimento repinta só a
//! caixa do furo antigo (volta a escurecer) e a do novo (copia o furo pronto).

use crate::canvas::{Rect, premul};

/// 60% de preto, pré-multiplicado.
pub const DARK: u32 = 153 << 24;
/// Dentro do furo: transparente (o clique passa pela janela inteira de qualquer jeito).
pub const HOLE: u32 = 0;

pub const MIN_RADIUS: f32 = 40.0;
pub const MAX_RADIUS: f32 = 900.0;
pub const MIN_ZOOM: f32 = 1.5;
pub const MAX_ZOOM: f32 = 4.0;

/// Furo de raio r, pronto para copiar: d x d pixels, borda suave de ~2 px.
pub fn hole(r: f32) -> (i32, Vec<u32>) {
    let d = 2 * (r.ceil() as i32 + 2);
    let c = d as f32 / 2.0;
    let mut px = vec![DARK; (d * d) as usize];
    // d é par e o círculo é simétrico: calcula um quadrante e espelha nos
    // outros três (raio 900 levava 23 ms).
    let n = d as usize;
    for y in 0..n / 2 {
        for x in 0..n / 2 {
            let dist = (x as f32 + 0.5 - c).hypot(y as f32 + 0.5 - c);
            // 0 dentro, 1 fora, rampa de 2 px na borda.
            let t = ((dist - r + 1.0) / 2.0).clamp(0.0, 1.0);
            let a = (t * 153.0 + 0.5) as u32;
            let v = if a <= 1 { HOLE } else { premul(0, a) };
            let (xm, ym) = (n - 1 - x, n - 1 - y);
            for i in [y * n + x, y * n + xm, ym * n + x, ym * n + xm] {
                px[i] = v;
            }
        }
    }
    (d, px)
}

/// Caixa do furo de lado d centrado em (cx, cy).
pub fn hole_box(cx: i32, cy: i32, d: i32) -> Rect {
    Rect { x0: cx - d / 2, y0: cy - d / 2, x1: cx - d / 2 + d, y1: cy - d / 2 + d }
}

/// Move o furo: escurece a caixa `old` e copia o furo em `new`. Devolve as
/// duas áreas (recortadas) que mudaram, para o `prcDirty`.
pub fn move_hole(px: &mut [u32], w: i32, h: i32, old: Rect, new: Rect, hole: &[u32]) -> (Rect, Rect) {
    let o = old.clip(w, h);
    if !o.is_empty() {
        for y in o.y0..o.y1 {
            let row = (y * w) as usize;
            px[row + o.x0 as usize..row + o.x1 as usize].fill(DARK);
        }
    }
    let n = new.clip(w, h);
    if !n.is_empty() {
        let d = new.x1 - new.x0;
        for y in n.y0..n.y1 {
            let row = (y * w) as usize;
            let src = ((y - new.y0) * d + n.x0 - new.x0) as usize;
            let len = (n.x1 - n.x0) as usize;
            px[row + n.x0 as usize..row + n.x1 as usize].copy_from_slice(&hole[src..src + len]);
        }
    }
    (o, n)
}

/// Junta as duas áreas num envio só quando a união não é muito maior que as duas
/// (furo andou pouco); senão, manda separado.
pub fn merge_dirty(a: Rect, b: Rect) -> Option<Rect> {
    let area = |r: &Rect| if r.is_empty() { 0i64 } else { (r.x1 - r.x0) as i64 * (r.y1 - r.y0) as i64 };
    let u = a.union(&b);
    (area(&u) <= 2 * (area(&a) + area(&b))).then_some(u)
}

/// Zoom: retângulo da tela (x, y, w, h) que a lupa de `lw` x `lh` amplia `z`
/// vezes em volta do cursor, sem sair da área `screen` (x, y, w, h).
pub fn zoom_src(cx: i32, cy: i32, lw: i32, lh: i32, z: f32, screen: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let (sw, sh) = (((lw as f32 / z).round() as i32).clamp(1, screen.2), ((lh as f32 / z).round() as i32).clamp(1, screen.3));
    let x = (cx - sw / 2).clamp(screen.0, screen.0 + screen.2 - sw);
    let y = (cy - sh / 2).clamp(screen.1, screen.1 + screen.3 - sh);
    (x, y, sw, sh)
}

/// Passo da roda: raio do holofote (x1,15) e zoom (x1,25), dentro dos limites.
pub fn step_radius(r: f32, dir: i32) -> f32 {
    (if dir > 0 { r * 1.15 } else { r / 1.15 }).clamp(MIN_RADIUS, MAX_RADIUS)
}

pub fn step_zoom(z: f32, dir: i32) -> f32 {
    (if dir > 0 { z * 1.25 } else { z / 1.25 }).clamp(MIN_ZOOM, MAX_ZOOM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn furo_claro_no_meio_e_escuro_fora() {
        let (d, px) = hole(50.0);
        let at = |x: i32, y: i32| px[(y * d + x) as usize];
        assert_eq!(at(d / 2, d / 2), HOLE);
        assert_eq!(at(0, 0), DARK);
        assert_eq!(at(d / 2 + 40, d / 2), HOLE);
        assert_eq!(at(d - 1, d / 2), DARK);
        // Borda suave: o alfa cresce sem pular de 1 para 153.
        let row: Vec<u32> = (d / 2..d).map(|x| at(x, d / 2) >> 24).collect();
        assert!(row.windows(2).all(|w| w[0] <= w[1]));
        assert!(row.iter().any(|&a| a > 1 && a < 153));
    }

    /// Pinta do zero: tudo escuro e o furo em (cx, cy).
    fn fresh(w: i32, h: i32, cx: i32, cy: i32, d: i32, hole: &[u32]) -> Vec<u32> {
        let mut px = vec![DARK; (w * h) as usize];
        move_hole(&mut px, w, h, Rect::EMPTY, hole_box(cx, cy, d), hole);
        px
    }

    #[test]
    fn mover_o_furo_da_o_mesmo_que_pintar_de_novo() {
        let (w, h) = (300, 200);
        let (d, hl) = hole(30.0);
        let mut px = fresh(w, h, 20, 20, d, &hl);
        let mut prev = hole_box(20, 20, d);
        // Inclui sair pela borda (recorte) e voltar.
        for (cx, cy) in [(60, 40), (290, 195), (-10, 100), (150, 100)] {
            let next = hole_box(cx, cy, d);
            move_hole(&mut px, w, h, prev, next, &hl);
            prev = next;
            assert!(px == fresh(w, h, cx, cy, d, &hl), "diferente em ({cx}, {cy})");
        }
    }

    #[test]
    fn sujeira_junta_perto_e_separa_longe() {
        let a = Rect { x0: 0, y0: 0, x1: 100, y1: 100 };
        assert!(merge_dirty(a, Rect { x0: 10, y0: 5, x1: 110, y1: 105 }).is_some());
        assert!(merge_dirty(a, Rect { x0: 2000, y0: 900, x1: 2100, y1: 1000 }).is_none());
        assert_eq!(merge_dirty(Rect::EMPTY, a), Some(a));
    }

    #[test]
    fn zoom_pega_a_area_em_volta_sem_sair_da_tela() {
        let screen = (0, 0, 1920, 1080);
        assert_eq!(zoom_src(960, 540, 480, 270, 2.0, screen), (840, 473, 240, 135));
        // No canto: o retângulo encosta na borda em vez de pegar fora da tela.
        assert_eq!(zoom_src(5, 5, 480, 270, 2.0, screen), (0, 0, 240, 135));
        assert_eq!(zoom_src(1919, 1079, 480, 270, 4.0, screen), (1800, 1012, 120, 68));
        // Monitor com origem negativa (à esquerda do principal).
        assert_eq!(zoom_src(-100, 10, 400, 200, 2.0, (-2560, 0, 2560, 1440)), (-200, 0, 200, 100));
    }

    #[test]
    fn roda_respeita_os_limites() {
        let mut z = 2.0;
        for _ in 0..20 {
            z = step_zoom(z, 1);
        }
        assert_eq!(z, MAX_ZOOM);
        for _ in 0..20 {
            z = step_zoom(z, -1);
        }
        assert_eq!(z, MIN_ZOOM);
        assert_eq!(step_radius(MIN_RADIUS, -1), MIN_RADIUS);
        assert!(step_radius(100.0, 1) > 100.0);
    }

    #[test]
    fn furo_espelhado_igual_ao_calculado_ponto_a_ponto() {
        for r in [3.0f32, 37.5, 200.0] {
            let (d, px) = hole(r);
            let c = d as f32 / 2.0;
            for y in 0..d {
                for x in 0..d {
                    let dist = (x as f32 + 0.5 - c).hypot(y as f32 + 0.5 - c);
                    let a = (((dist - r + 1.0) / 2.0).clamp(0.0, 1.0) * 153.0 + 0.5) as u32;
                    assert_eq!(px[(y * d + x) as usize], if a <= 1 { HOLE } else { premul(0, a) }, "r {r} ({x}, {y})");
                }
            }
        }
    }
}
