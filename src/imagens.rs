//! Gera as imagens do manual com o próprio motor de desenho, sem print de tela.
//! `cargo test --release imagens -- --ignored`  →  grava em `imagens/`.

use crate::canvas::{Canvas, Stroke, Tool, over};
use crate::toolbar::{self, Item};

const COLORS: [u32; 9] = [0xE53935, 0xFB8C00, 0xFDD835, 0x43A047, 0x00ACC1, 0x1E88E5, 0xD81B60, 0xFFFFFF, 0x111111];

/// Grava o PNG com o codificador do próprio app.
fn png(path: &str, w: i32, h: i32, bgra: &[u32]) {
    std::fs::write(path, crate::png::encode(w as usize, h as usize, bgra)).unwrap();
}

/// Uma "janela de programa" genérica para o desenho ter onde ficar.
fn fake_desktop(w: i32, h: i32) -> Vec<u32> {
    let mut bg = vec![0u32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let t = (y as f32 / h as f32 * 18.0) as u32;
            let mut c = 0xFF000000 | ((0x2A + t) << 16) | ((0x30 + t) << 8) | (0x3A + t);
            let in_win = x > 150 && x < w - 60 && y > 50 && y < h - 50;
            if in_win {
                c = 0xFF1E2024;
                if y < 86 {
                    c = 0xFF2B2E33; // barra de título
                }
                // "conteúdo": blocos cinza
                let (cx, cy) = (x - 190, y - 120);
                if cy >= 0 && cx >= 0 && (cy % 70) < 46 && cx < (w - 300) && ((cy / 70) % 2 == 0 || cx < 520) {
                    c = 0xFF34383F;
                }
            }
            bg[(y * w + x) as usize] = c;
        }
    }
    bg
}

fn stroke(tool: Tool, rgb: u32, r: f32, pts: Vec<(f32, f32)>) -> Stroke {
    Stroke { tool, rgb, alpha: if tool == Tool::Marker { 90 } else { 255 }, pts: pts.into_iter().map(|(x, y)| [x, y, r]).collect(), num: 0, text: None, area: Default::default() }
}

/// Caixa de texto pronta, como a ferramenta T deixa.
fn text(x: f32, y: f32, s: &str, rgb: u32) -> Stroke {
    let f = crate::texto::Font::new(crate::texto::font_px(4.0)).unwrap();
    let mask = f.render(s, 2000).mask;
    Stroke { tool: Tool::Text, rgb, alpha: 255, pts: vec![[x, y, 0.0]], num: 0, text: Some(std::sync::Arc::new(mask)), area: Default::default() }
}

fn bar_pixels(s: f32, tool: Tool, rgb: u32, draw: bool, hover: Option<Item>) -> (toolbar::Layout, Vec<u32>) {
    let l = toolbar::layout(s);
    let mut buf = vec![0u32; (l.w * l.h) as usize];
    let st = toolbar::State { draw, tool, rgb, size: 4.0, hidden: false, guides: false, fade: false, hover, can_undo: true, can_redo: false, rec: false, colors: &COLORS };
    toolbar::paint(&l, &st, &mut buf);
    (l, buf)
}

fn paste(dst: &mut [u32], dw: i32, src: &[u32], sw: i32, sh: i32, x0: i32, y0: i32) {
    for y in 0..sh {
        for x in 0..sw {
            let i = ((y0 + y) * dw + x0 + x) as usize;
            dst[i] = over(src[(y * sw + x) as usize], dst[i]);
        }
    }
}

#[test]
#[ignore]
fn imagens() {
    std::fs::create_dir_all("imagens").unwrap();

    // 1. A barra sozinha, em tamanho grande.
    let (l, bar) = bar_pixels(2.0, Tool::Pen, COLORS[0], true, None);
    let (w, h) = (l.w + 80, l.h + 80);
    let mut img = vec![0xFF2A2D33u32; (w * h) as usize];
    paste(&mut img, w, &bar, l.w, l.h, 40, 40);
    png("imagens/barra.png", w, h, &img);

    // 2. Anotações por cima de um programa.
    let (w, h) = (1280, 720);
    let mut img = fake_desktop(w, h);
    let mut c = Canvas::new(w, h);
    let mut layer = vec![0u32; (w * h) as usize];
    let wave: Vec<(f32, f32)> = (0..=90).map(|i| (600.0 + i as f32 * 5.0, 560.0 + (i as f32 / 9.0).sin() * 22.0)).collect();
    for s in [
        stroke(Tool::Marker, COLORS[2], 13.0, vec![(200.0, 147.0), (690.0, 147.0)]),
        stroke(Tool::Rect, COLORS[3], 2.5, vec![(760.0, 250.0), (1150.0, 400.0)]),
        stroke(Tool::Arrow, COLORS[0], 3.0, vec![(560.0, 470.0), (745.0, 360.0)]),
        stroke(Tool::Ellipse, COLORS[5], 2.5, vec![(185.0, 255.0), (450.0, 345.0)]),
        stroke(Tool::Pen, COLORS[0], 2.5, wave),
        Stroke::marker(210.0, 205.0, 4.0, 1, COLORS[0]),
        Stroke::marker(730.0, 330.0, 4.0, 2, COLORS[0]),
        Stroke::marker(1170.0, 215.0, 4.0, 3, COLORS[0]),
        text(420.0, 478.0, "clique aqui", COLORS[0]),
    ] {
        let r = c.raster_stroke(&layer, &s);
        c.compose(&mut layer, r, s.rgb, s.alpha);
        c.end_live();
    }
    for (d, s) in img.iter_mut().zip(&layer) {
        *d = over(*s, *d);
    }
    let (l, bar) = bar_pixels(1.25, Tool::Pen, COLORS[0], true, Some(Item::Tool(Tool::Arrow)));
    paste(&mut img, w, &bar, l.w, l.h, 24, (h - l.h) / 2);
    png("imagens/exemplo.png", w, h, &img);
}

/// Prévia do ícone em `target/icone.png`: 16, 32 e 64 px, e o 16 ampliado 8x.
#[test]
#[ignore]
fn icone() {
    let (w, h) = (16 * 8 + 16 + 64 + 32 + 16 + 40, 140);
    let mut img = vec![0xFF5A_5A5Au32; (w * h) as usize];
    let mut x = 8;
    for n in [16, 32, 64] {
        let px = toolbar::app_icon(n);
        for (i, &p) in px.iter().enumerate() {
            let (ix, iy) = (x + i as i32 % n, 8 + i as i32 / n);
            let d = &mut img[(iy * w + ix) as usize];
            *d = over(p, *d);
        }
        x += n + 8;
    }
    let px = toolbar::app_icon(16);
    for y in 0..128 {
        for xx in 0..128 {
            let d = &mut img[((y + 8) * w + x + xx) as usize];
            *d = over(px[((y / 8) * 16 + xx / 8) as usize], *d);
        }
    }
    png("target/icone.png", w, h, &img);
}

/// Prévia dos marcadores 0-9, 10, 12, 99 e 100 em `target/marcadores.png`.
#[test]
#[ignore]
fn marcadores() {
    let (w, h) = (14 * 70, 80);
    let mut c = Canvas::new(w, h);
    let mut out = vec![0xFF202428u32; (w * h) as usize];
    for (k, n) in [0u16, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 99, 100].into_iter().enumerate() {
        let s = Stroke::marker(35.0 + k as f32 * 70.0, 40.0, 4.0, n, COLORS[7]);
        let r = c.raster_stroke(&out, &s);
        c.compose(&mut out, r, s.rgb, s.alpha);
        c.end_live();
    }
    png("target/marcadores.png", w, h, &out);
}
