//! Guias de enquadramento: terços, retângulo 16:9 com área segura e terços
//! dentro do 16:9. Só geometria e pintura; a janela fica em `main.rs`.
//! Linhas de 1 px (escaladas pelo DPI), brancas e semitransparentes, com uma
//! sombra escura para aparecer em fundo claro.

use crate::canvas::{over, premul};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Thirds,
    /// 16:9 centralizado + área segura (90% ação, 80% título).
    Frame,
    /// Terços dentro do 16:9.
    FrameThirds,
}

impl Mode {
    pub fn next(self) -> Mode {
        match self {
            Mode::Off => Mode::Thirds,
            Mode::Thirds => Mode::Frame,
            Mode::Frame => Mode::FrameThirds,
            Mode::FrameThirds => Mode::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Linha principal (terços, moldura 16:9).
    Main,
    /// Área segura de ação (90%).
    Action,
    /// Área segura de título (80%): tracejada.
    Title,
}

/// Linha horizontal (y0 == y1, de x0 a x1) ou vertical (x0 == x1, de y0 a y1),
/// em pixels do monitor. Os extremos são exclusivos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub kind: Kind,
}

/// Retângulo 16:9 de maior tamanho que cabe em w x h, centralizado:
/// (x0, y0, x1, y1).
pub fn frame_rect(w: i32, h: i32) -> (i32, i32, i32, i32) {
    let (fw, fh) = if w * 9 >= h * 16 { (h * 16 / 9, h) } else { (w, w * 9 / 16) };
    let (x0, y0) = ((w - fw) / 2, (h - fh) / 2);
    (x0, y0, x0 + fw, y0 + fh)
}

/// Retângulo `pct`% do tamanho de r, com o mesmo centro.
fn scaled(r: (i32, i32, i32, i32), pct: i32) -> (i32, i32, i32, i32) {
    let (w, h) = (r.2 - r.0, r.3 - r.1);
    let (dx, dy) = ((w - w * pct / 100) / 2, (h - h * pct / 100) / 2);
    (r.0 + dx, r.1 + dy, r.2 - dx, r.3 - dy)
}

fn outline(r: (i32, i32, i32, i32), kind: Kind, out: &mut Vec<Line>) {
    let (x0, y0, x1, y1) = r;
    out.push(Line { x0, y0, x1, y1: y0, kind });
    out.push(Line { x0, y0: y1 - 1, x1, y1: y1 - 1, kind });
    out.push(Line { x0, y0, x1: x0, y1, kind });
    out.push(Line { x0: x1 - 1, y0, x1: x1 - 1, y1, kind });
}

fn thirds(r: (i32, i32, i32, i32), kind: Kind, out: &mut Vec<Line>) {
    let (x0, y0, x1, y1) = r;
    let (w, h) = (x1 - x0, y1 - y0);
    for k in 1..3 {
        let x = x0 + w * k / 3;
        out.push(Line { x0: x, y0, x1: x, y1, kind });
        let y = y0 + h * k / 3;
        out.push(Line { x0, y0: y, x1, y1: y, kind });
    }
}

/// Linhas do modo, em pixels de um monitor w x h.
pub fn lines(mode: Mode, w: i32, h: i32) -> Vec<Line> {
    let mut v = Vec::new();
    let full = (0, 0, w, h);
    let frame = frame_rect(w, h);
    match mode {
        Mode::Off => {}
        Mode::Thirds => thirds(full, Kind::Main, &mut v),
        Mode::Frame => {
            outline(frame, Kind::Main, &mut v);
            outline(scaled(frame, 90), Kind::Action, &mut v);
            outline(scaled(frame, 80), Kind::Title, &mut v);
        }
        Mode::FrameThirds => {
            outline(frame, Kind::Main, &mut v);
            thirds(frame, Kind::Action, &mut v);
        }
    }
    v
}

/// Mais linhas que um modo desenha (a moldura com as duas áreas seguras).
pub const MAX_LINES: usize = 12;

/// Onde a linha e a sombra ficam, no eixo perpendicular: (linha, sombra).
/// A linha na borda do monitor entra para dentro, para não sumir.
fn offsets(l: &Line, t: i32, w: i32, h: i32) -> (i32, i32) {
    let (fixed, limit) = if l.y0 == l.y1 { (l.y0, h) } else { (l.x0, w) };
    (fixed.min(limit - t).max(0), fixed.min(limit - 2 * t).max(0) + t)
}

/// Caixa (x0, y0, x1, y1) que a linha ocupa com a sombra, em pixels do monitor.
/// Cada linha vira uma janela desse tamanho: uma faixa de 2 espessuras.
pub fn line_box(l: &Line, t: i32, w: i32, h: i32) -> (i32, i32, i32, i32) {
    let t = t.max(1);
    let (fl, fs) = offsets(l, t, w, h);
    let (a, b) = (fl.min(fs), fl.max(fs) + t);
    if l.y0 == l.y1 { (l.x0, a, l.x1, b) } else { (a, l.y0, b, l.y1) }
}

/// Pinta a linha (sombra embaixo, branco por cima) em `buf`, que cobre a caixa
/// `bx` do monitor w x h (BGRA pré-multiplicado, já zerado).
/// `t` = espessura em pixels (1 em 96 dpi).
pub fn paint_line(buf: &mut [u32], bx: (i32, i32, i32, i32), l: &Line, t: i32, w: i32, h: i32) {
    let t = t.max(1);
    let (bw, bh) = (bx.2 - bx.0, bx.3 - bx.1);
    let mut put = |x: i32, y: i32, px: u32| {
        let (x, y) = (x - bx.0, y - bx.1);
        if x >= 0 && y >= 0 && x < bw && y < bh {
            let i = (y * bw + x) as usize;
            buf[i] = over(px, buf[i]);
        }
    };
    let horizontal = l.y0 == l.y1;
    let (from, to) = if horizontal { (l.x0, l.x1) } else { (l.y0, l.y1) };
    let (fl, fs) = offsets(l, t, w, h);
    let alpha = if l.kind == Kind::Main { 190 } else { 130 };
    for (fixed, px) in [(fs, premul(0x000000, alpha * 2 / 5)), (fl, premul(0xFFFFFF, alpha))] {
        for pos in from..to {
            if l.kind == Kind::Title && (pos / (8 * t)) % 2 == 1 {
                continue;
            }
            for k in 0..t {
                if horizontal { put(pos, fixed + k, px) } else { put(fixed + k, pos, px) }
            }
        }
    }
}

/// O monitor inteiro de uma vez (testes e imagens do manual).
#[cfg(test)]
pub fn paint(buf: &mut [u32], w: i32, h: i32, mode: Mode, t: i32) {
    for l in lines(mode, w, h) {
        paint_line(buf, (0, 0, w, h), &l, t, w, h);
    }
}

/// Espessura da linha (px) para o DPI do monitor: escala arredondada, pelo
/// menos 1 (125% = 1, 150% = 2). A divisão inteira dava 1 px até 199%.
pub fn thickness(dpi: u32) -> i32 {
    ((dpi as f32 / 96.0).round() as i32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn o_ciclo_volta_ao_desligado_em_quatro_passos() {
        let mut m = Mode::Off;
        let mut seen = Vec::new();
        for _ in 0..4 {
            m = m.next();
            seen.push(m);
        }
        assert_eq!(seen, [Mode::Thirds, Mode::Frame, Mode::FrameThirds, Mode::Off]);
    }

    #[test]
    fn desligado_nao_tem_linha() {
        assert!(lines(Mode::Off, 1920, 1080).is_empty());
    }

    #[test]
    fn tercos_dividem_o_monitor() {
        let l = lines(Mode::Thirds, 1920, 1080);
        assert_eq!(l.len(), 4);
        let verticais: Vec<i32> = l.iter().filter(|l| l.x0 == l.x1).map(|l| l.x0).collect();
        let horizontais: Vec<i32> = l.iter().filter(|l| l.y0 == l.y1).map(|l| l.y0).collect();
        assert_eq!(verticais, [640, 1280]);
        assert_eq!(horizontais, [360, 720]);
        assert!(l.iter().filter(|l| l.x0 == l.x1).all(|l| (l.y0, l.y1) == (0, 1080)));
    }

    #[test]
    fn moldura_16x9_centralizada_em_varios_monitores() {
        assert_eq!(frame_rect(1920, 1080), (0, 0, 1920, 1080));
        assert_eq!(frame_rect(3840, 2160), (0, 0, 3840, 2160));
        assert_eq!(frame_rect(3440, 1440), (440, 0, 3000, 1440)); // ultrawide: faixas dos lados
        assert_eq!(frame_rect(1600, 1200), (0, 150, 1600, 1050)); // 4:3: faixas em cima e embaixo
        assert_eq!(frame_rect(1080, 1920), (0, 656, 1080, 1263)); // retrato
        for (w, h) in [(2560, 1440), (3440, 1440), (1600, 1200), (1280, 1024)] {
            let (x0, y0, x1, y1) = frame_rect(w, h);
            assert_eq!(x0, w - x1, "centro em x");
            assert!((y0 - (h - y1)).abs() <= 1, "centro em y");
            let (fw, fh) = (x1 - x0, y1 - y0);
            assert!((fw * 9 - fh * 16).abs() <= 16, "proporção {fw}x{fh}");
        }
    }

    #[test]
    fn area_segura_90_e_80_por_cento_com_o_mesmo_centro() {
        let l = lines(Mode::Frame, 1920, 1080);
        assert_eq!(l.len(), 12);
        let box_of = |k: Kind| {
            let v: Vec<&Line> = l.iter().filter(|l| l.kind == k).collect();
            let x0 = v.iter().map(|l| l.x0).min().unwrap();
            let y0 = v.iter().map(|l| l.y0).min().unwrap();
            let x1 = v.iter().map(|l| l.x1).max().unwrap();
            let y1 = v.iter().map(|l| l.y1).max().unwrap();
            (x0, y0, x1, y1)
        };
        assert_eq!(box_of(Kind::Main), (0, 0, 1920, 1080));
        assert_eq!(box_of(Kind::Action), (96, 54, 1824, 1026)); // 1728 x 972 = 90%
        assert_eq!(box_of(Kind::Title), (192, 108, 1728, 972)); // 1536 x 864 = 80%
    }

    #[test]
    fn tercos_dentro_do_16x9_no_monitor_ultrawide() {
        let l = lines(Mode::FrameThirds, 3440, 1440);
        let t: Vec<&Line> = l.iter().filter(|l| l.kind == Kind::Action).collect();
        let mut xs: Vec<i32> = t.iter().filter(|l| l.x0 == l.x1).map(|l| l.x0).collect();
        xs.sort();
        assert_eq!(xs, [440 + 853, 440 + 1706]); // terços de 2560
        let ys: Vec<i32> = t.iter().filter(|l| l.y0 == l.y1).map(|l| l.y0).collect();
        assert_eq!(ys, [480, 960]);
        // Nada passa da moldura.
        assert!(t.iter().all(|l| l.x0 >= 440 && l.x1 <= 3000));
    }

    #[test]
    fn janela_por_linha_da_a_mesma_imagem_e_custa_pouco() {
        // Monitor 4K com escala 200% (linha de 2 px).
        let (w, h, t) = (3840, 2160, 2);
        for mode in [Mode::Thirds, Mode::Frame, Mode::FrameThirds] {
            let ls = lines(mode, w, h);
            assert!(ls.len() <= MAX_LINES, "{mode:?}: {} linhas", ls.len());
            let mut full = vec![0u32; (w * h) as usize];
            paint(&mut full, w, h, mode, t);
            // Cada janela pinta só a sua faixa; juntas dão a mesma imagem.
            let mut joined = vec![0u32; (w * h) as usize];
            let mut area = 0;
            for l in &ls {
                let b = line_box(l, t, w, h);
                assert!(b.0 >= 0 && b.1 >= 0 && b.2 <= w && b.3 <= h, "{l:?} sai do monitor: {b:?}");
                let (bw, bh) = (b.2 - b.0, b.3 - b.1);
                // Na borda do monitor a sombra cai embaixo da linha: faixa de 1 espessura.
                assert!((t..=2 * t).contains(&bw.min(bh)), "{l:?}: faixa de {bw}x{bh}");
                area += bw * bh;
                let mut small = vec![0u32; (bw * bh) as usize];
                paint_line(&mut small, b, l, t, w, h);
                for y in 0..bh {
                    for x in 0..bw {
                        let i = ((b.1 + y) * w + b.0 + x) as usize;
                        joined[i] = over(small[(y * bw + x) as usize], joined[i]);
                    }
                }
            }
            assert_eq!(joined, full, "{mode:?}");
            // Todas as janelas juntas: menos de 2% do monitor (~560 KB contra 33 MB da 4K inteira).
            assert!(area * 50 < w * h, "{mode:?}: {area} px");
        }
    }

    #[test]
    fn pinta_so_perto_das_linhas_e_dentro_do_monitor() {
        let (w, h) = (320, 180);
        for mode in [Mode::Thirds, Mode::Frame, Mode::FrameThirds] {
            let mut buf = vec![0u32; (w * h) as usize];
            paint(&mut buf, w, h, mode, 1);
            let pintados = buf.iter().filter(|&&p| p != 0).count();
            assert!(pintados > 300, "{mode:?}: {pintados}");
            assert!(pintados < (w * h / 10) as usize, "{mode:?} pintou demais: {pintados}");
        }
        // O centro de um monitor 16:9 fica limpo nos terços e na moldura.
        let mut buf = vec![0u32; (w * h) as usize];
        paint(&mut buf, w, h, Mode::Frame, 1);
        assert_eq!(buf[(h / 2 * w + w / 2) as usize], 0);
        // A moldura 16:9 de um monitor 16:9 encosta na borda e continua visível.
        assert_ne!(buf[(h / 2 * w) as usize], 0);
        assert_ne!(buf[(h / 2 * w + w - 1) as usize], 0);
    }

    #[test]
    fn espessura_pelo_dpi_do_monitor_arredondada() {
        assert_eq!(thickness(96), 1);
        assert_eq!(thickness(120), 1); // 125%
        assert_eq!(thickness(144), 2); // 150%
        assert_eq!(thickness(168), 2); // 175%
        assert_eq!(thickness(192), 2); // 200%
        assert_eq!(thickness(288), 3); // 300%
        assert_eq!(thickness(0), 1);
    }
}
