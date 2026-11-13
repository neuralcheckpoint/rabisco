#![windows_subsystem = "windows"]
//! Rabisco: anotar por cima da tela. Uma janela em camadas, sempre no topo,
//! cobrindo todos os monitores, e uma barra flutuante dona dela.
//!
//! Memória: a imagem da tela (DIB) só existe no modo desenho. Ao voltar para o
//! mouse, a janela recebe o quadro final e a DIB é liberada; o Windows continua
//! mostrando o que já recebeu. Para mudar algo depois, a DIB é refeita a partir
//! dos traços (vetores).

mod bandeja;
mod canvas;
mod foco;
mod gravar;
mod gravar_gpu;
mod gravar_som;
mod guias;
mod instalar;
mod png;
mod texto;
mod toolbar;
#[cfg(test)]
mod imagens;
#[cfg(test)]
mod perf;

use canvas::{Canvas, FLOOR, Fading, Rect, Stroke, Tool};
use core::ptr::{null, null_mut};
use toolbar::{Item, Layout, SIZES};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetTickCount64};
use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW, SetEvent, Sleep};
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Input::Pointer::*;
use windows_sys::Win32::UI::Shell::{NIN_SELECT, NINF_KEY};
use windows_sys::core::BOOL;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const HK_DRAW: i32 = 1;
const HK_HIDE: i32 = 2;
const HK_CLEAR: i32 = 3;
const HK_QUIT: i32 = 4;
const HK_SAVE: i32 = 5;
const HK_REC: i32 = 6;
/// Esc durante a escolha da janela ou da região (só existe nesse meio tempo).
const HK_PICK_ESC: i32 = 7;
const HK_PRINT_REG: i32 = 8;
const HK_BAR: i32 = 9;

/// Outra cópia foi aberta (atalho do Iniciar): mostra a barra desta.
const WM_SHOW_BAR: u32 = WM_APP + 2;
/// A thread do print terminou: lParam = `Box<Saved>`.
const WM_SAVED: u32 = WM_APP + 3;
// Do processo da gravação: começou (wp = x | y << 16, lp = w | h << 16, a área
// gravada) e terminou (wp = 1 se deu certo).
const WM_REC_STARTED: u32 = WM_APP + 4;
const WM_REC_DONE: u32 = WM_APP + 5;
/// Mouse durante a escolha (do gancho WH_MOUSE_LL): wp = 0 move, 1 aperta, 2 solta, 3 cancela.
const WM_PICK: u32 = WM_APP + 6;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

const MENU_BAR: u32 = 1;
const MENU_DRAW: u32 = 2;
const MENU_HIDE: u32 = 3;
const MENU_CLEAR: u32 = 4;
const MENU_AUTOSTART: u32 = 5;
const MENU_QUIT: u32 = 6;
const MENU_SAVE: u32 = 7;
const MENU_REC_MON: u32 = 8;
const MENU_REC_WIN: u32 = 9;
const MENU_REC_REG: u32 = 10;
const MENU_REC_SYS: u32 = 11;
const MENU_REC_MIC: u32 = 12;
const MENU_REC_STOP: u32 = 13;
const MENU_BAR_FADE: u32 = 14;
const MENU_PRINT_REG: u32 = 15;

const CF_DIB: u32 = 8;

const TIP_TIMER: usize = 1;
/// Parado no modo desenho por este tempo: solta a imagem da tela.
const IDLE_TIMER: usize = 2;
const IDLE_MS: u32 = 8000;
/// Esconde o aviso de "print salvo".
const TOAST_TIMER: usize = 3;
const TOAST_MS: u32 = 1000;
/// Cursor da caixa de texto piscando (só existe com a caixa aberta).
const CARET_TIMER: usize = 4;
/// Próximo passo do fade (só existe com traço para sumir).
const FADE_TIMER: usize = 5;
/// Zoom: junta os movimentos do mouse num quadro a cada ZOOM_MS.
const ZOOM_TIMER: usize = 6;
/// Contador do tempo gravado: 1 s, só enquanto grava.
const REC_TIMER: usize = 7;
/// Barra transparente: confere se o mouse saiu dela (só roda com o mouse em cima).
const BAR_TIMER: usize = 8;
/// Opacidade da barra transparente longe do mouse (de 255).
const BAR_FADED: u8 = 80;
const ZOOM_MS: u32 = 20;

/// Já tem um print sendo codificado: o próximo pedido espera.

/// Teclas 1 a 9 (mesma ordem de `toolbar::COLOR_NAMES`).
const COLORS: [u32; 9] = [
    0xE53935, // vermelho
    0xFB8C00, // laranja
    0xFDD835, // amarelo
    0x43A047, // verde
    0x00ACC1, // ciano
    0x1E88E5, // azul
    0xD81B60, // magenta
    0xFFFFFF, // branco
    0x111111, // preto
];

const MARKER_ALPHA: u8 = 90;

enum Action {
    Add(Stroke),
    /// Traços apagados, na ordem da remoção (índice no momento em que saiu).
    Erase(Vec<(usize, Stroke)>),
    Clear(Vec<Stroke>),
}

struct Sample {
    x: f32,
    y: f32,
    pressure: f32,
    eraser: bool,
}

/// Uma DIB 32 bits num DC de memória, para `UpdateLayeredWindowIndirect`.
struct Surface {
    dc: HDC,
    dib: HBITMAP,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Surface {
    fn new(w: i32, h: i32) -> Option<Surface> {
        unsafe {
            let mut bmi: BITMAPINFO = core::mem::zeroed();
            bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h; // de cima para baixo
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB as u32;
            let mut bits = null_mut();
            let dib = CreateDIBSection(null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if dib.is_null() {
                return None;
            }
            let dc = CreateCompatibleDC(null_mut());
            SelectObject(dc, dib);
            Some(Surface { dc, dib, bits: bits as *mut u32, w, h })
        }
    }

    fn pixels(&mut self) -> &'static mut [u32] {
        unsafe { core::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) }
    }

    /// Manda r para a janela. `pos` = canto na tela, ou None para não mover.
    fn present(&self, hwnd: HWND, pos: Option<(i32, i32)>, r: Option<Rect>) {
        self.update(hwnd, pos, r, AC_SRC_ALPHA as u8, 255);
    }

    /// Como `present`, com a janela inteira a `alpha` de 255.
    fn present_alpha(&self, hwnd: HWND, alpha: u8) {
        self.update(hwnd, None, None, AC_SRC_ALPHA as u8, alpha);
    }

    /// Como `present`, mas opaca: o alfa dos pixels é ignorado (o GDI escreve 0 nele).
    fn present_opaque(&self, hwnd: HWND, pos: Option<(i32, i32)>) {
        self.update(hwnd, pos, None, 0, 255);
    }

    fn update(&self, hwnd: HWND, pos: Option<(i32, i32)>, r: Option<Rect>, format: u8, alpha: u8) {
        let dst = pos.map(|(x, y)| POINT { x, y });
        let size = SIZE { cx: self.w, cy: self.h };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: format };
        let dirty = r.map(|r| RECT { left: r.x0, top: r.y0, right: r.x1, bottom: r.y1 });
        let info = UPDATELAYEREDWINDOWINFO {
            cbSize: size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
            hdcDst: null_mut(),
            pptDst: dst.as_ref().map_or(null(), |p| p),
            psize: &size,
            hdcSrc: self.dc,
            pptSrc: &src,
            crKey: 0,
            pblend: &blend,
            dwFlags: ULW_ALPHA,
            prcDirty: dirty.as_ref().map_or(null(), |r| r),
        };
        unsafe { UpdateLayeredWindowIndirect(hwnd, &info) };
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            DeleteDC(self.dc);
            DeleteObject(self.dib);
        }
    }
}

struct Bar {
    hwnd: HWND,
    layout: Layout,
    surface: Option<Surface>,
    hover: Option<Item>,
    tip: HWND,
    tip_text: Vec<u16>,
    tip_shown: bool,
    /// Falso = guardada na bandeja.
    shown: bool,
    /// Modo barra transparente (fica quase invisível longe do mouse).
    faded: bool,
    /// Mouse em cima da barra agora.
    near: bool,
    /// O ● foi apertado: o menu abre ao soltar o botão.
    rec_menu: bool,
}

/// Caixa de texto aberta (ferramenta T). O texto fica em `App::live` como traço.
struct TextEdit {
    editor: texto::Editor,
    font: texto::Font,
    /// Canto da caixa, em pixels da tela virtual.
    x: f32,
    y: f32,
    /// Largura máxima: até a borda direita do monitor.
    max_w: i32,
    /// Altura máxima: até a borda de baixo do monitor. Passando disso, a
    /// digitação para (a caixa não cresce para fora da tela).
    max_h: i32,
    /// Linhas da tecla anterior: a próxima refaz só o que mudou.
    laid: texto::Laid,
    /// Cursor relativo ao canto da caixa.
    caret: (i32, i32),
    caret_on: bool,
}

enum LookKind {
    /// Furo pronto (`d` x `d`) e onde ele está agora (pixels do monitor).
    Spot { d: i32, hole: Vec<u32>, at: Rect },
    /// `src`: pedaço da tela copiado antes de ampliar (lado = lupa / zoom).
    Zoom { src: Option<Surface> },
}

/// Holofote ou zoom: janela própria com WS_EX_TRANSPARENT por cima de tudo. O
/// movimento chega pela janela da tela (modo desenho), que continua embaixo.
struct Look {
    hwnd: HWND,
    surface: Surface,
    /// Monitor (x, y, w, h) do holofote.
    mon: (i32, i32, i32, i32),
    kind: LookKind,
}

impl Drop for Look {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.hwnd) };
    }
}

struct App {
    hwnd: HWND,
    surface: Option<Surface>,
    visible: bool,
    ox: i32,
    oy: i32,
    canvas: Canvas,
    tool: Tool,
    rgb: u32,
    size: f32,
    draw: bool,
    hidden: bool,
    cursor: HCURSOR,
    /// Ponteiro que está desenhando agora.
    pointer: Option<u32>,
    live: Option<Stroke>,
    live_box: Rect,
    smooth_p: f32,
    /// Apagando neste arrasto (ferramenta borracha ou ponta de trás da caneta).
    erasing: Option<Vec<(usize, Stroke)>>,
    undo: Vec<Action>,
    redo: Vec<Action>,
    samples: Vec<Sample>,
    scratch: Vec<(canvas::Pt, canvas::Pt)>,
    bar: Bar,
    /// Guias de enquadramento: janela própria (clique passa), no monitor do cursor.
    guides: guias::Mode,
    guide_hwnds: [HWND; guias::MAX_LINES],
    /// Aviso curto de "print salvo".
    toast_hwnd: HWND,
    /// PNG do último print, entregue à área de transferência sob demanda
    /// (CF_DIB e "PNG" são renderizados quando alguém cola).
    clip_png: Option<Vec<u8>>,
    print: PrintQueue<Surface>,
    /// Zoom: último ponto ainda não desenhado (Some = timer armado).
    zoom_at: Option<(i32, i32)>,
    png_format: u32,
    tray_icon: HICON,
    /// `TaskbarCreated`: o Explorer reiniciou e o ícone da bandeja sumiu.
    taskbar_created: u32,
    /// Modo fade: traço novo some sozinho (fica fora do desfazer).
    fade: bool,
    text: Option<TextEdit>,
    look: Option<Look>,
    /// Raio do holofote (0 = ainda não escolhido: 1/8 da altura do monitor).
    spot_r: f32,
    zoom: f32,
    rec: gravar::State,
    /// Evento que pede para o processo da gravação parar.
    rec_stop: HANDLE,
    rec_child: Option<std::process::Child>,
    /// O `.part` reservado aqui: se o filho morrer, quem apaga é o pai.
    rec_part: Option<std::path::PathBuf>,
    /// Espera do fim do processo no pool do Windows (sem thread nossa).
    rec_wait: HANDLE,
    /// Ponto vermelho no canto da área gravada, fora da captura.
    rec_dot: HWND,
    /// Moldura vermelha em volta do que grava (fora do vídeo) e o que ela cerca.
    rec_frame: [HWND; 4],
    rec_rect: (i32, i32, i32, i32),
    /// Gravando uma janela: ela (a moldura e o contador a seguem).
    rec_target: HWND,
    /// Área gravada (x, y, w) e o tamanho do ponto (d, m), para o contador.
    rec_area: (i32, i32, i32, i32, i32),
    rec_t0: Option<std::time::Instant>,
    rec_prefs: gravar::Prefs,
    /// Escolha da janela ou da região em andamento.
    pick: Option<Picking>,
}

static mut APP: *mut App = null_mut();

fn app() -> &'static mut App {
    // Só a thread da janela mexe aqui.
    unsafe { &mut *APP }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn lparam_point(lp: LPARAM) -> (i32, i32) {
    ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32)
}

fn pointer_id(wp: WPARAM) -> u32 {
    (wp & 0xFFFF) as u32
}

fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        )
    }
}

impl App {
    fn floor(&self) -> u32 {
        if self.draw { FLOOR } else { 0 }
    }

    /// Tem algo na tela (traços fixos ou sumindo)?
    fn has_content(&self) -> bool {
        !self.canvas.strokes.is_empty() || !self.canvas.fading.is_empty()
    }

    // ---------- superfície da tela ----------

    /// Garante a DIB; se foi criada agora, já vem com os traços desenhados.
    fn ensure_surface(&mut self) -> bool {
        if self.surface.is_some() {
            return true;
        }
        let Some(mut s) = Surface::new(self.canvas.w, self.canvas.h) else { return false };
        let floor = self.floor();
        self.canvas.rebuild(s.pixels(), floor);
        self.surface = Some(s);
        true
    }

    fn present(&mut self, r: Rect) {
        let r = r.clip(self.canvas.w, self.canvas.h);
        if let (Some(s), false) = (&self.surface, r.is_empty()) {
            s.present(self.hwnd, Some((self.ox, self.oy)), Some(r));
        }
    }

    fn present_all(&mut self) {
        if let Some(s) = &self.surface {
            s.present(self.hwnd, Some((self.ox, self.oy)), None);
        }
    }

    /// Mostra a janela da tela só se ela tem algo para mostrar ou pegar.
    fn sync_visibility(&mut self) {
        let want = !self.hidden && (self.draw || self.has_content());
        if want != self.visible {
            self.visible = want;
            unsafe { ShowWindow(self.hwnd, if want { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
        }
        if !self.draw && self.canvas.fading.is_empty() {
            self.surface = None; // o Windows já tem o último quadro
        }
    }

    /// Algo mudou na lista de traços: redesenha a área (ou tudo) e manda.
    fn redraw(&mut self, area: Option<Rect>) {
        let fresh = self.surface.is_none();
        if !self.hidden && (self.draw || self.has_content()) && self.ensure_surface() {
            let floor = self.floor();
            let px = self.surface.as_mut().unwrap().pixels();
            match area {
                _ if fresh => {} // acabou de ser refeita inteira
                Some(r) => {
                    self.canvas.rebuild_region(px, r, floor);
                }
                None => self.canvas.rebuild(px, floor),
            }
            match area {
                Some(r) if !fresh => self.present(r),
                _ => self.present_all(),
            }
        }
        self.sync_visibility();
        self.arm_idle();
        self.paint_bar();
    }

    fn redraw_all(&mut self) {
        self.redraw(None);
    }

    /// No modo desenho, depois de um tempo parado a imagem da tela é solta
    /// (o Windows continua mostrando e pegando o clique). O próximo toque refaz.
    fn arm_idle(&mut self) {
        if self.draw {
            unsafe { SetTimer(self.hwnd, IDLE_TIMER, IDLE_MS, None) };
        }
    }

    fn idle(&mut self) {
        unsafe { KillTimer(self.hwnd, IDLE_TIMER) };
        if self.pointer.is_none() && self.live.is_none() && self.erasing.is_none() && self.canvas.fading.is_empty() {
            self.surface = None;
        }
    }

    // ---------- fade ----------

    fn toggle_fade(&mut self) {
        self.fade = !self.fade;
        self.paint_bar();
    }

    /// Passo do fade: refaz só a caixa do que mudou de opacidade e marca o
    /// próximo passo (3 s parado, depois um quadro a cada 16 ms). Sem nada
    /// para sumir, o timer morre.
    fn fade_tick(&mut self) {
        if self.live.is_some() {
            // Traço (ou caixa de texto) em andamento usa os blocos que a
            // reconstrução descartaria: o fade espera ele terminar.
            unsafe { SetTimer(self.hwnd, FADE_TIMER, canvas::FADE_FRAME_MS as u32, None) };
            return;
        }
        let (now, floor) = (unsafe { GetTickCount64() }, self.floor());
        let (dirty, next) = match self.surface.as_mut() {
            Some(s) if !self.hidden => self.canvas.fade_frame(s.pixels(), now, floor),
            _ => self.canvas.fade_tick(now),
        };
        if !dirty.is_empty() && self.hidden {
            self.surface = None; // ficou velha: mostrar de novo refaz do zero
        } else if !dirty.is_empty() {
            if self.surface.is_some() {
                self.present(dirty);
            } else if self.ensure_surface() {
                self.present_all();
            }
        }
        match next {
            Some(ms) => unsafe {
                SetTimer(self.hwnd, FADE_TIMER, ms.clamp(10, 60_000) as u32, None);
            },
            None => {
                unsafe { KillTimer(self.hwnd, FADE_TIMER) };
                self.sync_visibility();
                self.arm_idle();
            }
        }
    }

    // ---------- texto ----------

    /// Abre a caixa com o clique na altura do meio da primeira linha.
    fn text_open(&mut self, x: f32, y: f32) {
        let Some(font) = texto::Font::new(texto::font_px(self.size)) else { return };
        let (mx, my, mw, mh) = cursor_monitor();
        let pad = texto::pad(font.px);
        let (x, y) = (x - pad as f32, y - (pad + font.line_h / 2) as f32);
        let max_w = ((mx + mw - self.ox) as f32 - x) as i32 - 8;
        // Uma linha cabe sempre, mesmo com o clique colado na borda de baixo.
        let max_h = (((my + mh - self.oy) as f32 - y) as i32).max(font.line_h + 2 * pad);
        let (max_w, laid) = (max_w.max(120), texto::Laid::default());
        self.text = Some(TextEdit { editor: texto::Editor::default(), font, x, y, max_w, max_h, laid, caret: (0, 0), caret_on: true });
        self.text_update(true, true);
        self.arm_caret();
    }

    fn arm_caret(&mut self) {
        let t = unsafe { GetCaretBlinkTime() };
        if t != 0 && t != u32::MAX {
            unsafe { SetTimer(self.hwnd, CARET_TIMER, t, None) };
        }
    }

    fn text_char(&mut self, u: u16) {
        let Some(ed) = self.text.as_mut() else { return };
        if ed.editor.input(u) {
            ed.caret_on = true;
            self.text_update(true, true);
            self.arm_caret();
        }
    }

    /// Redesenha a caixa aberta. `gdi`: o texto ou a fonte mudou (refaz pelo
    /// GDI); `all`: manda a caixa toda (senão só o cursor, que piscou).
    fn text_update(&mut self, gdi: bool, all: bool) {
        if self.text.is_none() || !self.ensure_surface() {
            return;
        }
        let ed = self.text.as_mut().unwrap();
        let px = self.surface.as_mut().unwrap().pixels();
        let mut live = match self.live.take() {
            Some(l) if l.tool == Tool::Text => l,
            _ => {
                ed.laid = texto::Laid::default();
                let mask = canvas::TextMask { w: 0, h: 0, cov: Vec::new() };
                Stroke { tool: Tool::Text, rgb: self.rgb, alpha: 255, pts: vec![[ed.x, ed.y, 0.0]], num: 0, text: Some(std::sync::Arc::new(mask)), area: Default::default() }
            }
        };
        // Primeira linha (na caixa) que mudou: o resto fica como está.
        let mut from = ed.caret.1; // o cursor piscou: só a linha dele
        if gdi || live.text.as_ref().is_some_and(|m| m.w == 0) {
            let m = std::sync::Arc::make_mut(live.text.as_mut().unwrap());
            (from, ed.caret) = ed.font.update(&ed.editor.text, ed.max_w, &mut ed.laid, m);
            if m.h > ed.max_h && ed.editor.backspace() {
                // Passou da borda de baixo do monitor: a tecla não entra.
                let (f, caret) = ed.font.update(&ed.editor.text, ed.max_w, &mut ed.laid, m);
                (from, ed.caret) = (from.min(f), caret);
            }
        }
        live.rgb = self.rgb;
        let w = ed.font.caret_w() as f32 * 0.5;
        let lh = ed.font.line_h as f32;
        let cx = (live.pts[0][0].round() as i32 + ed.caret.0) as f32 + w;
        let cy = (live.pts[0][1].round() as i32 + ed.caret.1) as f32;
        let (a, b) = ([cx, cy + lh * 0.1 + w, w], [cx, cy + lh * 0.9 - w, w]);
        let old = self.live_box;
        // Só da linha que mudou para baixo: com 2.000 letras, refazer a caixa
        // toda a cada tecla levava 12 ms.
        let y = live.pts[0][1].round() as i32 + from;
        self.canvas.clear_live_from(y);
        let mut r = self.canvas.raster_stroke_from(px, &live, y);
        let caret = canvas::seg_box(&a, &b, self.canvas.w, self.canvas.h);
        if ed.caret_on {
            self.canvas.raster_seg(px, a, b);
        }
        r = r.union(&caret);
        let full = live.text_box().map_or(r, |b| b.clip(self.canvas.w, self.canvas.h).union(&caret));
        self.live_box = full;
        let below = Rect { x0: 0, y0: y.max(0), x1: self.canvas.w, y1: self.canvas.h };
        let dirty = if all { old.union(&full).intersect(&below).union(&caret) } else { caret };

        self.canvas.compose(px, dirty, live.rgb, 255);
        self.live = Some(live);
        self.present(dirty);
    }

    /// Fecha a caixa: sem o cursor, o texto vira item (ou some, se ficou vazio).
    fn finish_text(&mut self, live: Stroke) {
        unsafe { KillTimer(self.hwnd, CARET_TIMER) };
        let empty = self.text.take().is_none_or(|e| e.editor.text.trim().is_empty());
        let Some(px) = self.surface.as_mut().map(|s| s.pixels()) else {
            if !empty {
                self.commit(live); // sem imagem agora: entra na lista e aparece no próximo redesenho
            }
            return;
        };
        if empty {
            let r = self.canvas.cancel_live(px);
            self.present(r);
            return;
        }
        self.canvas.clear_live();
        self.canvas.raster_stroke(px, &live);
        let r = self.canvas.live_area();
        self.canvas.compose(px, r, live.rgb, live.alpha);
        self.canvas.end_live();
        self.present(r);
        self.commit(live);
    }

    /// Traço pronto entra na lista e no desfazer, ou no fade.
    fn commit(&mut self, live: Stroke) {
        if self.fade {
            self.canvas.fading.push(Fading { stroke: live, born: unsafe { GetTickCount64() }, k: 255 });
            self.fade_tick();
        } else {
            self.canvas.strokes.push(live.clone());
            self.push_action(Action::Add(live));
        }
    }

    // ---------- holofote e zoom ----------

    /// S (holofote) ou Z (zoom): liga; a mesma tecla de novo desliga.
    fn toggle_look(&mut self, zoom: bool) {
        let same = self.look.as_ref().is_some_and(|l| matches!(l.kind, LookKind::Zoom { .. }) == zoom);
        self.look = None;
        if same {
            return;
        }
        self.finish();
        let was_drawing = self.draw;
        self.set_draw(true); // o movimento do mouse chega pela janela da tela
        if !was_drawing {
            // Fora do desenho só o fundo de clique importa, e a janela já guarda
            // o quadro: a imagem da tela inteira sai agora, sem esperar o IDLE_TIMER.
            self.idle();
        }
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        self.look_open(zoom, pt.x, pt.y);
    }

    fn look_open(&mut self, zoom: bool, x: i32, y: i32) {
        let mon = monitor_at(x, y);
        let hwnd = aux_window();
        if hwnd.is_null() {
            return;
        }
        let look = if zoom {
            unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) };
            let Some(s) = Surface::new((mon.2 / 4).max(160), (mon.3 / 4).max(90)) else { return };
            Look { hwnd, surface: s, mon, kind: LookKind::Zoom { src: None } }
        } else {
            if self.spot_r <= 0.0 {
                self.spot_r = (mon.3 as f32 / 8.0).clamp(foco::MIN_RADIUS, foco::MAX_RADIUS);
            }
            let Some(mut s) = Surface::new(mon.2, mon.3) else { return };
            s.pixels().fill(foco::DARK);
            let (d, hole) = foco::hole(self.spot_r);
            Look { hwnd, surface: s, mon, kind: LookKind::Spot { d, hole, at: Rect::EMPTY } }
        };
        self.look = Some(look);
        self.look_move(x, y);
        unsafe {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    /// Cursor em (x, y) da tela: move o furo ou a lupa.
    /// Movimento do ponteiro: o holofote anda na hora; o zoom junta os
    /// movimentos e captura uma vez a cada ZOOM_MS (o BitBlt custa ~16 ms e
    /// roda nesta thread). O timer só existe com a lupa aberta e se mata sozinho.
    fn look_pointer(&mut self, x: i32, y: i32) {
        if !self.look.as_ref().is_some_and(|l| matches!(l.kind, LookKind::Zoom { .. })) {
            return self.look_move(x, y);
        }
        if self.zoom_at.replace((x, y)).is_none() {
            unsafe { SetTimer(self.hwnd, ZOOM_TIMER, ZOOM_MS, None) };
        }
    }

    fn zoom_tick(&mut self) {
        unsafe { KillTimer(self.hwnd, ZOOM_TIMER) };
        if let Some((x, y)) = self.zoom_at.take()
            && self.look.is_some()
        {
            self.look_move(x, y);
        }
    }

    fn look_move(&mut self, x: i32, y: i32) {
        let Some(l) = self.look.as_mut() else { return };
        let (mx, my, mw, mh) = l.mon;
        let spot = matches!(l.kind, LookKind::Spot { .. });
        if spot && (x < mx || y < my || x >= mx + mw || y >= my + mh) {
            // Outro monitor: o holofote vai junto.
            self.look = None;
            return self.look_open(false, x, y);
        }
        let first = matches!(&l.kind, LookKind::Spot { at, .. } if at.is_empty());
        match &mut l.kind {
            LookKind::Spot { d, hole, at } => {
                let next = foco::hole_box(x - mx, y - my, *d);
                if next == *at {
                    return;
                }
                let (o, n) = foco::move_hole(l.surface.pixels(), mw, mh, *at, next, hole);
                *at = next;
                let pos = first.then_some((mx, my));
                match foco::merge_dirty(o, n) {
                    _ if first => l.surface.present(l.hwnd, pos, None),
                    Some(u) => l.surface.present(l.hwnd, None, Some(u)),
                    None => {
                        l.surface.present(l.hwnd, None, Some(o));
                        l.surface.present(l.hwnd, None, Some(n));
                    }
                }
            }
            LookKind::Zoom { src } => {
                let (lw, lh) = (l.surface.w, l.surface.h);
                let (sx, sy, sw, sh) = foco::zoom_src(x, y, lw, lh, self.zoom, virtual_screen());
                if src.as_ref().is_none_or(|s| s.w != sw || s.h != sh) {
                    *src = Surface::new(sw, sh);
                }
                let Some(s) = src.as_ref() else { return };
                unsafe {
                    // CAPTUREBLT pega as janelas em camadas (as anotações); a lupa
                    // está fora da captura, então não aparece dentro dela mesma.
                    let screen = GetDC(null_mut());
                    BitBlt(s.dc, 0, 0, sw, sh, screen, sx, sy, SRCCOPY | CAPTUREBLT);
                    ReleaseDC(null_mut(), screen);
                    SetStretchBltMode(l.surface.dc, COLORONCOLOR);
                    StretchBlt(l.surface.dc, 0, 0, lw, lh, s.dc, 0, 0, sw, sh, SRCCOPY);
                    GdiFlush();
                }
                let px = l.surface.pixels();
                let b = (lh / 180).max(2);
                for yy in 0..lh {
                    let row = &mut px[(yy * lw) as usize..((yy + 1) * lw) as usize];
                    if yy < b || yy >= lh - b {
                        row.fill(0xFFE8_EAED);
                    } else {
                        row[..b as usize].fill(0xFFE8_EAED);
                        row[(lw - b) as usize..].fill(0xFFE8_EAED);
                    }
                }
                l.surface.present_opaque(l.hwnd, Some((x - lw / 2, y - lh / 2)));
            }
        }
    }

    /// Roda do mouse com o holofote ou o zoom ligado.
    fn look_wheel(&mut self, dir: i32) {
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        let Some(l) = self.look.as_mut() else { return };
        match &mut l.kind {
            // O próximo movimento escurece a caixa velha inteira e copia o furo novo.
            LookKind::Spot { d, hole, .. } => {
                self.spot_r = foco::step_radius(self.spot_r, dir);
                (*d, *hole) = foco::hole(self.spot_r);
            }
            LookKind::Zoom { .. } => self.zoom = foco::step_zoom(self.zoom, dir),
        }
        self.look_move(pt.x, pt.y);
    }

    fn wheel(&mut self, dir: i32) {
        if self.look.is_some() {
            self.look_wheel(dir);
        } else {
            self.resize_pen(dir);
        }
    }

    fn set_draw(&mut self, on: bool) {
        if on == self.draw {
            if on {
                unsafe { SetForegroundWindow(self.hwnd) };
            }
            self.paint_bar();
            return;
        }
        if !on {
            self.finish();
            self.look = None;
        }
        self.draw = on;
        if on {
            self.hidden = false;
            if self.surface.is_none() {
                self.ensure_surface();
            } else {
                let px = self.surface.as_mut().unwrap().pixels();
                canvas::swap_floor(px, 0, FLOOR);
            }
        } else if let Some(s) = self.surface.as_mut() {
            canvas::swap_floor(s.pixels(), FLOOR, 0);
        } else {
            self.ensure_surface(); // foi solta por estar parado: refaz sem o fundo de clique
        }
        self.present_all();
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32;
            let ex = if on { ex & !WS_EX_TRANSPARENT } else { ex | WS_EX_TRANSPARENT };
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex as isize);
        }
        self.sync_visibility();
        if on {
            unsafe {
                SetForegroundWindow(self.hwnd);
                SetCursor(self.cursor);
            }
            self.arm_idle();
        }
        self.paint_bar();
    }

    fn set_hidden(&mut self, hidden: bool) {
        if hidden && self.draw {
            self.set_draw(false);
        }
        self.hidden = hidden;
        if !hidden && !self.visible && self.has_content() {
            // A janela ficou sem o último quadro? Refaz antes de mostrar.
            self.ensure_surface();
            self.present_all();
        }
        self.sync_visibility();
        self.paint_bar();
    }

    /// Monitores mudaram: os traços ficam no mesmo lugar da tela.
    fn display_changed(&mut self) {
        self.finish();
        self.look = None;
        let (ox, oy, w, h) = virtual_screen();
        let (dx, dy) = ((self.ox - ox) as f32, (self.oy - oy) as f32);
        let mut strokes = std::mem::take(&mut self.canvas.strokes);
        let mut fading = std::mem::take(&mut self.canvas.fading);
        for p in strokes.iter_mut().chain(fading.iter_mut().map(|f| &mut f.stroke)).flat_map(|s| s.pts.iter_mut()) {
            p[0] += dx;
            p[1] += dy;
        }
        (self.ox, self.oy) = (ox, oy);
        self.canvas = Canvas::new(w, h);
        self.canvas.strokes = strokes;
        self.canvas.fading = fading;
        self.surface = None;
        unsafe { SetWindowPos(self.hwnd, null_mut(), ox, oy, w, h, SWP_NOACTIVATE | SWP_NOZORDER) };
        self.redraw_all();
        self.show_guides();
    }

    // ---------- ferramenta, cor, tamanho ----------

    fn set_tool(&mut self, t: Tool) {
        self.finish();
        self.tool = t;
        self.refresh_cursor();
        self.set_draw(true);
    }

    fn set_color(&mut self, rgb: u32) {
        self.rgb = rgb;
        if self.tool == Tool::Eraser {
            self.tool = Tool::Pen;
        }
        self.refresh_cursor();
        self.set_draw(true);
        self.text_update(false, true);
    }

    fn set_size(&mut self, size: f32) {
        self.size = size.clamp(1.0, 200.0);
        self.refresh_cursor();
        self.paint_bar();
        // Caixa aberta: a fonte acompanha a espessura.
        let px = texto::font_px(self.size);
        if let Some(ed) = self.text.as_mut().filter(|e| e.font.px != px)
            && let Some(f) = texto::Font::new(px)
        {
            ed.font = f;
            ed.laid = texto::Laid::default(); // outra fonte: refaz tudo
            self.text_update(true, true);
        }
    }

    fn resize_pen(&mut self, dir: i32) {
        let s = self.size;
        self.set_size(if dir > 0 { s * 1.2 + 0.5 } else { (s - 0.5) / 1.2 });
    }

    fn refresh_cursor(&mut self) {
        unsafe {
            let old = self.cursor;
            let diam = match self.tool {
                Tool::Marker => self.size * 3.0,
                Tool::Eraser => self.eraser_radius() * 2.0,
                Tool::Number => canvas::marker_dims(self.size).0 * 2.0,
                _ => self.size,
            };
            let rgb = if self.tool == Tool::Eraser { 0xBBBBBB } else { self.rgb };
            self.cursor = make_cursor(diam, rgb, !self.tool.is_freehand() && self.tool != Tool::Eraser);
            if self.draw {
                SetCursor(self.cursor);
            }
            if !old.is_null() {
                DestroyIcon(old);
            }
        }
    }

    fn eraser_radius(&self) -> f32 {
        (self.size * 2.0).max(12.0)
    }

    fn radius(&self, tool: Tool, pressure: f32) -> f32 {
        match tool {
            Tool::Pen => (self.size * 0.5 * (0.15 + 0.85 * pressure)).max(0.6),
            Tool::Marker => self.size * 1.5,
            _ => (self.size * 0.5).max(0.6),
        }
    }

    // ---------- entrada ----------

    fn read_samples(&mut self, id: u32) {
        self.samples.clear();
        unsafe {
            let mut kind = 0;
            GetPointerType(id, &mut kind);
            let (ox, oy) = (self.ox as f32, self.oy as f32);
            if kind == PT_PEN {
                let mut info: POINTER_PEN_INFO = core::mem::zeroed();
                if GetPointerPenInfo(id, &mut info) == 0 {
                    return;
                }
                let mut n = info.pointerInfo.historyCount.max(1);
                let mut hist = vec![POINTER_PEN_INFO::default(); n as usize];
                if GetPointerPenInfoHistory(id, &mut n, hist.as_mut_ptr()) == 0 {
                    hist[0] = info;
                    n = 1;
                }
                // O histórico vem do mais novo para o mais velho.
                for e in hist[..n as usize].iter().rev() {
                    self.samples.push(Sample {
                        x: e.pointerInfo.ptPixelLocation.x as f32 - ox,
                        y: e.pointerInfo.ptPixelLocation.y as f32 - oy,
                        pressure: if e.penMask & PEN_MASK_PRESSURE != 0 { e.pressure as f32 / 1024.0 } else { 1.0 },
                        eraser: e.penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0,
                    });
                }
            } else {
                let mut info: POINTER_INFO = core::mem::zeroed();
                if GetPointerInfo(id, &mut info) == 0 {
                    return;
                }
                let mut n = info.historyCount.max(1);
                let mut hist = vec![POINTER_INFO::default(); n as usize];
                if GetPointerInfoHistory(id, &mut n, hist.as_mut_ptr()) == 0 {
                    hist[0] = info;
                    n = 1;
                }
                for e in hist[..n as usize].iter().rev() {
                    self.samples.push(Sample {
                        x: e.ptPixelLocation.x as f32 - ox,
                        y: e.ptPixelLocation.y as f32 - oy,
                        pressure: 1.0,
                        eraser: false,
                    });
                }
            }
        }
    }

    fn pointer_down(&mut self, id: u32) {
        if self.look.as_ref().is_some_and(|l| matches!(l.kind, LookKind::Zoom { .. })) {
            return; // a lupa cobre o lugar do traço
        }
        if self.text.is_some() {
            // Caixa aberta: clicar dentro não faz nada; fora confirma o texto (e só).
            self.read_samples(id);
            if !self.samples.last().is_some_and(|s| self.live_box.contains(s.x as i32, s.y as i32)) {
                self.finish();
            }
            return;
        }
        self.finish();
        if !self.ensure_surface() {
            return;
        }
        self.read_samples(id);
        let Some(s) = self.samples.pop() else { return };
        if self.tool == Tool::Text && !s.eraser {
            self.text_open(s.x, s.y);
            return;
        }
        self.pointer = Some(id);
        if self.tool == Tool::Eraser || s.eraser {
            self.erasing = Some(Vec::new());
            self.erase_at(s.x, s.y);
            return;
        }
        let tool = self.tool;
        if tool == Tool::Number {
            // Um clique = um marcador; arrastar antes de soltar move o marcador.
            let num = canvas::next_number(self.canvas.strokes.iter().chain(self.canvas.fading.iter().map(|f| &f.stroke)));
            let live = Stroke::marker(s.x, s.y, self.size, num, self.rgb);
            let px = self.surface.as_mut().unwrap().pixels();
            self.live_box = self.canvas.raster_stroke(px, &live);
            self.canvas.compose(px, self.live_box, live.rgb, live.alpha);
            self.live = Some(live);
            let r = self.live_box;
            self.present(r);
            return;
        }
        self.smooth_p = s.pressure;
        let pt = [s.x, s.y, self.radius(tool, s.pressure)];
        let alpha = if tool == Tool::Marker { MARKER_ALPHA } else { 255 };
        let pts = if tool.is_freehand() { vec![pt] } else { vec![pt, pt] };
        let live = Stroke { tool, rgb: self.rgb, alpha, pts, num: 0, text: None, area: Default::default() };
        self.live_box = Rect::EMPTY;
        if tool.is_freehand() {
            let px = self.surface.as_mut().unwrap().pixels();
            self.live_box = self.canvas.raster_seg(px, pt, pt);
            self.canvas.compose(px, self.live_box, live.rgb, live.alpha);
        }
        self.live = Some(live);
        let r = self.live_box;
        self.present(r);
    }

    fn pointer_move(&mut self, id: u32) {
        if self.pointer != Some(id) {
            return;
        }
        self.read_samples(id);
        let samples = std::mem::take(&mut self.samples);
        for s in &samples {
            self.add_sample(s);
        }
        self.samples = samples;
    }

    fn add_sample(&mut self, s: &Sample) {
        if self.erasing.is_some() {
            self.erase_at(s.x, s.y);
            return;
        }
        let Some(mut live) = self.live.take() else { return };
        let Some(px) = self.surface.as_mut().map(|s| s.pixels()) else { return };
        if live.tool.is_freehand() {
            self.smooth_p = self.smooth_p * 0.6 + s.pressure * 0.4;
            let pt = [s.x, s.y, self.radius(live.tool, self.smooth_p)];
            let last = *live.pts.last().unwrap();
            if (pt[0] - last[0]).hypot(pt[1] - last[1]) >= 0.75 {
                live.pts.push(pt);
                let r = self.canvas.raster_seg(px, last, pt);
                self.canvas.compose(px, r, live.rgb, live.alpha);
                self.live_box = self.live_box.union(&r);
                self.live = Some(live);
                self.present(r);
                return;
            }
        } else {
            let a = live.pts[0];
            if live.tool == Tool::Number {
                let r = live.pts[1][0] - a[0];
                live.pts[0] = [s.x, s.y, a[2]];
                live.pts[1] = [s.x + r, s.y, a[2]];
            } else {
                let (mut x, mut y) = (s.x, s.y);
                if unsafe { GetKeyState(VK_SHIFT as i32) } < 0 {
                    (x, y) = constrain(live.tool, a[0], a[1], x, y);
                }
                live.pts[1] = [x, y, a[2]];
            }
            let old = self.live_box;
            self.canvas.clear_live();
            self.live_box = self.canvas.raster_stroke(px, &live);
            let r = old.union(&self.live_box);
            self.canvas.compose(px, r, live.rgb, live.alpha);
            self.live = Some(live);
            self.present(r);
            return;
        }
        self.live = Some(live);
    }

    fn pointer_up(&mut self, id: u32) {
        if self.pointer != Some(id) {
            return;
        }
        self.pointer_move(id);
        self.finish();
    }

    /// Fecha o traço ou o arrasto da borracha em andamento.
    fn finish(&mut self) {
        self.pointer = None;
        if let Some(erased) = self.erasing.take() {
            if !erased.is_empty() {
                self.push_action(Action::Erase(erased));
            }
        }
        let Some(live) = self.live.take() else { return };
        self.live_box = Rect::EMPTY;
        if live.tool == Tool::Text {
            self.finish_text(live);
            self.sync_visibility();
            self.arm_idle();
            return;
        }
        let (a, b) = (live.pts[0], live.pts[live.pts.len() - 1]);
        if !live.tool.is_freehand() && live.tool != Tool::Number && (b[0] - a[0]).hypot(b[1] - a[1]) < 2.0 {
            // Clique sem arrastar numa forma: devolve o que estava embaixo.
            if let Some(px) = self.surface.as_mut().map(|s| s.pixels()) {
                let r = self.canvas.cancel_live(px);
                self.present(r);
            }
        } else {
            self.canvas.end_live();
            self.commit(live);
        }
        self.sync_visibility();
        self.arm_idle();
    }

    fn erase_at(&mut self, x: f32, y: f32) {
        let er = self.eraser_radius();
        let mut area = Rect::EMPTY;
        let mut i = self.canvas.strokes.len();
        while i > 0 {
            i -= 1;
            if self.canvas.strokes[i].hit(x, y, er, &mut self.scratch) {
                let s = self.canvas.strokes.remove(i);
                area = area.union(&s.bbox(&mut self.scratch));
                if let Some(list) = self.erasing.as_mut() {
                    list.push((i, s));
                }
            }
        }
        if !area.is_empty() {
            self.redraw(Some(area));
        }
    }

    // ---------- desfazer ----------

    fn push_action(&mut self, a: Action) {
        self.undo.push(a);
        self.redo.clear();
        self.paint_bar();
    }

    fn clear_all(&mut self) {
        self.finish();
        // O que está sumindo vai junto, sem desfazer (nunca entrou nele).
        let fading = !self.canvas.fading.is_empty();
        self.canvas.fading.clear();
        if self.canvas.strokes.is_empty() {
            if fading {
                self.redraw_all();
            }
            return;
        }
        let all = std::mem::take(&mut self.canvas.strokes);
        self.undo.push(Action::Clear(all));
        self.redo.clear();
        self.redraw_all();
    }

    fn undo(&mut self, redo: bool) {
        self.finish();
        let Some(a) = (if redo { self.redo.pop() } else { self.undo.pop() }) else { return };
        // Só a área dos traços que entram ou saem precisa ser refeita.
        let sc = &mut self.scratch;
        let area = match &a {
            Action::Add(s) => Some(s.bbox(sc)),
            Action::Erase(list) => Some(list.iter().fold(Rect::EMPTY, |r, (_, s)| r.union(&s.bbox(sc)))),
            Action::Clear(_) => None,
        };
        let strokes = &mut self.canvas.strokes;
        let back = match a {
            Action::Add(s) if redo => {
                strokes.push(s.clone());
                Action::Add(s)
            }
            Action::Add(_) => Action::Add(strokes.pop().unwrap()),
            Action::Erase(list) => {
                if redo {
                    for (i, _) in &list {
                        strokes.remove(*i);
                    }
                } else {
                    for (i, s) in list.iter().rev() {
                        strokes.insert(*i, s.clone());
                    }
                }
                Action::Erase(list)
            }
            Action::Clear(list) => {
                if redo {
                    strokes.clear();
                } else {
                    *strokes = list.clone();
                }
                Action::Clear(list)
            }
        };
        if redo { self.undo.push(back) } else { self.redo.push(back) }
        self.redraw(area);
    }

    fn key(&mut self, vk: u16) {
        let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT as i32) } < 0;
        if self.text.is_some() {
            // Digitando: letras, Enter e Backspace chegam por WM_CHAR, então as
            // teclas de ferramenta não disparam. Esc confirma; Ctrl+algo confirma
            // e segue o atalho. AltGr é Ctrl+Alt no ABNT2: é letra, não atalho.
            let alt = unsafe { GetKeyState(VK_MENU as i32) } < 0;
            match vk {
                VK_ESCAPE => return self.finish(),
                _ if ctrl && !alt => self.finish(),
                _ => return,
            }
        }
        if self.look.is_some() && vk == VK_ESCAPE {
            self.look = None;
            return;
        }
        match vk {
            VK_ESCAPE => self.set_draw(false),
            0x5A if ctrl && shift => self.undo(true), // Ctrl+Shift+Z
            0x5A if ctrl => self.undo(false),         // Ctrl+Z
            0x59 if ctrl => self.undo(true),          // Ctrl+Y
            VK_DELETE | VK_BACK => self.clear_all(),
            0x43 if !ctrl => self.clear_all(),     // C
            0x50 => self.set_tool(Tool::Pen),     // P
            0x4D => self.set_tool(Tool::Marker),  // M
            0x4C => self.set_tool(Tool::Line),    // L
            0x41 => self.set_tool(Tool::Arrow),   // A
            0x52 => self.set_tool(Tool::Rect),    // R
            0x4F => self.set_tool(Tool::Ellipse), // O
            0x45 => self.set_tool(Tool::Eraser),  // E
            0x4E => self.set_tool(Tool::Number),  // N
            0x54 => self.set_tool(Tool::Text),    // T
            0x47 => self.cycle_guides(),          // G
            0x46 => self.toggle_fade(),           // F
            0x53 if ctrl => self.save_print(),    // Ctrl+S
            0x53 => self.toggle_look(false),      // S: holofote
            0x5A => self.toggle_look(true),       // Z: zoom
            0x48 => self.set_hidden(true),        // H
            0x31..=0x39 => self.set_color(COLORS[(vk - 0x31) as usize]),
            VK_OEM_PLUS | VK_ADD => self.resize_pen(1),
            VK_OEM_MINUS | VK_SUBTRACT => self.resize_pen(-1),
            _ => {}
        }
    }

    fn hotkey(&mut self, id: i32) {
        match id {
            HK_DRAW => self.set_draw(!self.draw),
            HK_HIDE => self.set_hidden(!self.hidden),
            HK_CLEAR => self.clear_all(),
            HK_SAVE => {
                // Como o Ctrl+S local: confirma a caixa de texto antes (sem o
                // cursor no print). O atalho chega mesmo sem o foco do teclado.
                if self.text.is_some() {
                    self.finish();
                }
                self.save_print()
            }
            HK_REC => self.toggle_record(),
            HK_PRINT_REG => self.print_region(),
            HK_BAR => {
                // Guardada na bandeja: só aparece. Visível: liga/desliga a transparência.
                if self.bar.shown {
                    self.toggle_bar_fade();
                } else {
                    self.set_bar(true);
                }
            }
            HK_PICK_ESC => self.pick_end(None),
            HK_QUIT => unsafe {
                DestroyWindow(self.hwnd);
            },
            _ => {}
        }
    }

    // ---------- gravação ----------

    /// Ctrl+Shift+F e o ■: gravando, para; senão grava o modo salvo (janela e
    /// região são escolhidas antes). Durante a escolha, cancela.
    fn toggle_record(&mut self) {
        if self.pick.is_some() {
            return self.pick_end(None);
        }
        match (self.rec, self.rec_prefs.pick) {
            (gravar::State::Recording | gravar::State::Stopping, _) | (_, gravar::Pick::Monitor) => self.press_record(gravar::Mode::Monitor),
            (_, p) => self.pick_start(p, false),
        }
    }

    fn press_record(&mut self, mode: gravar::Mode) {
        let (s, act) = gravar::press(self.rec);
        self.rec = s;
        match act {
            gravar::Act::Start => self.start_record(mode),
            gravar::Act::Stop => unsafe {
                SetEvent(self.rec_stop);
            },
            gravar::Act::Nothing => {}
        }
        self.paint_bar();
    }

    /// Itens de gravação do menu do botão Gravar e da bandeja.
    fn rec_entries(&self, v: &mut Vec<bandeja::Entry<'static>>) {
        use bandeja::Entry::{Item as It, Sep};
        if matches!(self.rec, gravar::State::Recording | gravar::State::Stopping) {
            v.push(It { id: MENU_REC_STOP, text: "Parar a gravação\tCtrl+Shift+F", checked: false });
            return;
        }
        let p = self.rec_prefs;
        // O modo marcado é o do Ctrl+Shift+F.
        let modes = [
            (MENU_REC_MON, gravar::Pick::Monitor, "Gravar o monitor\tCtrl+Shift+F", "Gravar o monitor"),
            (MENU_REC_WIN, gravar::Pick::Window, "Gravar uma janela (clique nela)\tCtrl+Shift+F", "Gravar uma janela (clique nela)"),
            (MENU_REC_REG, gravar::Pick::Region, "Gravar uma região (arraste)\tCtrl+Shift+F", "Gravar uma região (arraste)"),
        ];
        for (id, pick, with_key, plain) in modes {
            v.push(It { id, text: if p.pick == pick { with_key } else { plain }, checked: p.pick == pick });
        }
        v.push(Sep);
        v.push(It { id: MENU_REC_SYS, text: "Som do sistema", checked: p.system });
        v.push(It { id: MENU_REC_MIC, text: "Microfone", checked: p.mic });
    }

    /// Um item de gravação escolhido no menu: o modo fica salvo e começa; o som só fica salvo.
    fn rec_command(&mut self, id: u32) {
        let mut p = self.rec_prefs;
        match id {
            MENU_REC_STOP => return self.toggle_record(),
            MENU_REC_MON => p.pick = gravar::Pick::Monitor,
            MENU_REC_WIN => p.pick = gravar::Pick::Window,
            MENU_REC_REG => p.pick = gravar::Pick::Region,
            MENU_REC_SYS => p.system = !p.system,
            MENU_REC_MIC => p.mic = !p.mic,
            _ => return,
        }
        self.rec_prefs = p;
        instalar::set_pref("Gravacao", p.bits());
        if matches!(id, MENU_REC_MON | MENU_REC_WIN | MENU_REC_REG) {
            self.toggle_record();
        }
    }

    fn rec_menu(&mut self) {
        let mut v = Vec::new();
        self.rec_entries(&mut v);
        // Dono = a barra (visível): com a janela do desenho, escondida no modo
        // mouse, o menu não ganhava o primeiro plano e fechava na hora.
        let id = bandeja::menu(self.bar.hwnd, &v);
        self.rec_command(id);
    }

    fn pick_start(&mut self, kind: gravar::Pick, print: bool) {
        let mon = cursor_monitor();
        let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(pick_hook), GetModuleHandleW(null()), 0) };
        if hook.is_null() {
            return self.show_toast(false);
        }
        unsafe { RegisterHotKey(self.hwnd, HK_PICK_ESC, 0, VK_ESCAPE as u32) };
        // A janela pode estar em qualquer tela; a região fica no monitor do cursor.
        let area = if kind == gravar::Pick::Window { virtual_screen() } else { mon };
        let mut pk = Picking {
            kind,
            hook,
            mon,
            area,
            dim: core::array::from_fn(|_| solid_box(DARK_CLASS, PICK_DIM)),
            edge: core::array::from_fn(|_| solid_box(EDGE_CLASS, 255)),
            label: aux_window(),
            from: None,
            target: null_mut(),
            print,
        };
        // Fora do print da região (que copia a tela logo depois de fechar a escolha).
        unsafe { SetWindowDisplayAffinity(pk.label, WDA_EXCLUDEFROMCAPTURE) };
        pick_show(&mut pk, None, mon);
        self.pick = Some(pk);
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        self.pick_event(0, pt.x, pt.y);
    }

    fn pick_event(&mut self, ev: usize, x: i32, y: i32) {
        let Some(pk) = self.pick.as_mut() else { return };
        let mon = pk.mon;
        let mut done = None;
        match (pk.kind, ev) {
            (_, 3) => return self.pick_end(None),
            (gravar::Pick::Window, 0) => {
                let t = window_at(x, y);
                if t != pk.target {
                    pk.target = t;
                    pick_show(pk, window_bounds(t), monitor_at(x, y));
                }
            }
            (gravar::Pick::Window, 2) if !pk.target.is_null() => done = Some(gravar::Mode::Window(pk.target as isize)),
            (gravar::Pick::Region, 1) => pk.from = Some((x, y)),
            (gravar::Pick::Region, 0 | 2) if pk.from.is_some() => {
                let a = pk.from.unwrap_or_default();
                // Dentro do monitor do começo, em coordenadas dele.
                let c = |v: (i32, i32)| ((v.0 - mon.0).clamp(0, mon.2), (v.1 - mon.1).clamp(0, mon.3));
                let sel = gravar::drag_rect(c(a), c((x, y)));
                pick_show(pk, Some((mon.0 + sel.0, mon.1 + sel.1, sel.2, sel.3)), mon);
                if ev == 2 {
                    pk.from = None;
                    if sel.2 >= 16 && sel.3 >= 16 {
                        done = Some(gravar::Mode::Region(mon.0 + sel.0, mon.1 + sel.1, sel.2, sel.3));
                    }
                }
            }
            _ => {}
        }
        if done.is_some() {
            self.pick_end(done);
        }
    }

    /// Fim da escolha: solta o gancho, o Esc, as janelas e a imagem; com um modo, grava.
    fn pick_end(&mut self, mode: Option<gravar::Mode>) {
        let Some(pk) = self.pick.take() else { return };
        unsafe {
            UnhookWindowsHookEx(pk.hook);
            UnregisterHotKey(self.hwnd, HK_PICK_ESC);
            for h in pk.dim.into_iter().chain(pk.edge).chain([pk.label]) {
                if !h.is_null() {
                    DestroyWindow(h);
                }
            }
        }
        match mode {
            Some(gravar::Mode::Region(x, y, w, h)) if pk.print => self.save_print_area(x, y, w, h),
            Some(m) => self.press_record(m),
            None => {}
        }
    }

    /// Ctrl+Shift+A: arrasta um retângulo e salva o print só dele (a mesma escolha
    /// da gravação de região). De novo durante a escolha, cancela.
    fn print_region(&mut self) {
        if self.pick.is_some() {
            return self.pick_end(None);
        }
        self.pick_start(gravar::Pick::Region, true);
    }

    /// A gravação roda em outro processo (`rabisco.exe --gravar`): D3D11, WGC e
    /// Media Foundation deixam ~60 MB de privado no processo mesmo depois de
    /// soltos (medido), e assim tudo volta ao sistema quando ele sai. O fim do
    /// processo chega por WM_REC_DONE (RegisterWaitForSingleObject): uma thread
    /// nossa só para esperar custava KB no exe.
    fn start_record(&mut self, mode: gravar::Mode) {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = format!("Local\\RabiscoParar{}_{n}", unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() });
        self.rec_stop = unsafe { CreateEventW(null(), 1, 0, wide(&name).as_ptr()) };
        let hwnd = self.hwnd as isize;
        let mut t: SYSTEMTIME = unsafe { core::mem::zeroed() };
        unsafe { GetLocalTime(&mut t) };
        let file = gravar::file_name(t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
        let Some((fin, part)) = gravar::videos_dir().filter(|d| std::fs::create_dir_all(d).is_ok()).and_then(|d| gravar::reserve(&d, &file)) else { return self.record_done() };
        self.rec_part = Some(part);
        self.rec_target = if let gravar::Mode::Window(h) = mode { h as HWND } else { null_mut() };
        let pid = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() }.to_string();
        let child = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).args(["--gravar", &hwnd.to_string(), &name, &gravar::mode_arg(mode), self.rec_prefs.sound_arg()]).arg(&fin).arg(&pid).spawn());
        let Ok(child) = child else { return self.record_done() };
        unsafe extern "system" fn exited(hwnd: *mut core::ffi::c_void, _: bool) {
            unsafe { PostMessageW(hwnd as HWND, WM_REC_DONE, 0, 0) };
        }
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Threading::{INFINITE, RegisterWaitForSingleObject, WT_EXECUTEONLYONCE};
        unsafe { RegisterWaitForSingleObject(&mut self.rec_wait, child.as_raw_handle() as HANDLE, Some(exited), self.hwnd as _, INFINITE, WT_EXECUTEONLYONCE) };
        self.rec_child = Some(child);
    }

    /// Moldura de 3 px em volta do que grava: por fora da janela ou da região,
    /// por dentro do monitor (por fora não apareceria).
    fn rec_frame_place(&self) {
        let r = self.rec_rect;
        let t = (3 * cursor_dpi().max(96) as i32 + 48) / 96;
        let r = if monitor_at(r.0, r.1) == r { (r.0 + t, r.1 + t, r.2 - 2 * t, r.3 - 2 * t) } else { r };
        for (h, b) in self.rec_frame.iter().zip(gravar::ring(r, t)) {
            place(*h, b);
        }
    }

    /// O processo da gravação saiu (ou nem abriu): deu certo se saiu com 0; 2 =
    /// parou antes do primeiro quadro (sem vídeo e sem aviso de erro).
    fn record_done(&mut self) {
        let code = self.rec_child.take().and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
        let (ok, canceled) = (code == Some(0), code == Some(2));
        // Filho morto (crash, taskkill) não apaga o próprio .part.
        if let Some(p) = self.rec_part.take().filter(|_| !ok) {
            let _ = std::fs::remove_file(p);
        }
        self.rec = gravar::ended(ok || canceled);
        if !self.rec_wait.is_null() {
            unsafe { windows_sys::Win32::System::Threading::UnregisterWait(self.rec_wait) };
            self.rec_wait = null_mut();
        }
        if !self.rec_stop.is_null() {
            unsafe { CloseHandle(self.rec_stop) };
            self.rec_stop = null_mut();
        }
        for h in core::mem::take(&mut self.rec_frame).into_iter().chain([core::mem::replace(&mut self.rec_dot, null_mut())]) {
            if !h.is_null() {
                unsafe { DestroyWindow(h) };
            }
        }
        self.rec_target = null_mut();
        unsafe { KillTimer(self.hwnd, REC_TIMER) };
        self.rec_t0 = None;
        self.paint_bar();
        if !canceled {
            self.show_toast(ok);
        }
    }

    // ---------- barra ----------

    fn paint_bar(&mut self) {
        let st = toolbar::State {
            draw: self.draw,
            tool: self.tool,
            rgb: self.rgb,
            size: self.size,
            hidden: self.hidden,
            guides: self.guides != guias::Mode::Off,
            fade: self.fade,
            hover: self.bar.hover,
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            rec: matches!(self.rec, gravar::State::Recording | gravar::State::Stopping),
            colors: &COLORS,
        };
        let bar = &mut self.bar;
        let Some(s) = bar.surface.as_mut() else { return };
        toolbar::paint(&bar.layout, &st, s.pixels());
        s.present_alpha(bar.hwnd, if bar.faded && !bar.near { BAR_FADED } else { 255 });
    }

    fn toggle_bar_fade(&mut self) {
        self.bar.faded = !self.bar.faded;
        instalar::set_pref("BarraTransparente", self.bar.faded as u32);
        self.paint_bar();
    }

    /// Mouse entrou na barra (ou o timer viu que saiu): opaca perto, transparente longe.
    fn bar_near(&mut self, near: bool) {
        if near == self.bar.near {
            return;
        }
        self.bar.near = near;
        unsafe {
            if near {
                SetTimer(self.bar.hwnd, BAR_TIMER, 150, None);
            } else {
                KillTimer(self.bar.hwnd, BAR_TIMER);
            }
        }
        if self.bar.faded {
            self.paint_bar();
        }
    }

    fn cursor_on_bar(&self) -> bool {
        let (mut r, mut pt) = (RECT { left: 0, top: 0, right: 0, bottom: 0 }, POINT { x: 0, y: 0 });
        unsafe { GetWindowRect(self.bar.hwnd, &mut r) != 0 && GetCursorPos(&mut pt) != 0 && pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom }
    }

    /// Recalcula a barra para o DPI do monitor onde ela está.
    fn layout_bar(&mut self) {
        let dpi = unsafe { GetDpiForWindow(self.bar.hwnd) }.max(96);
        self.bar.layout = toolbar::layout(dpi as f32 / 96.0);
        let l = &self.bar.layout;
        self.bar.surface = Surface::new(l.w, l.h);
        unsafe { SetWindowPos(self.bar.hwnd, null_mut(), 0, 0, l.w, l.h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE) };
        self.paint_bar();
    }

    fn bar_point(&self, sx: i32, sy: i32) -> (i32, i32) {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetWindowRect(self.bar.hwnd, &mut r) };
        (sx - r.left, sy - r.top)
    }

    fn bar_hover(&mut self, item: Option<Item>) {
        let item = item.filter(|i| *i != Item::Grip).or(item); // grip também tem dica
        if item == self.bar.hover {
            return;
        }
        self.bar.hover = item;
        self.hide_tip();
        if item.is_some() {
            unsafe { SetTimer(self.bar.hwnd, TIP_TIMER, 450, None) };
        }
        self.paint_bar();
    }

    fn bar_click(&mut self, item: Item) {
        self.hide_tip();
        match item {
            Item::Grip => {}
            Item::Close => self.set_bar(false),
            Item::Mouse => self.set_draw(false),
            Item::Tool(t) => self.set_tool(t),
            Item::Color(i) => self.set_color(COLORS[i]),
            Item::Size(i) => self.set_size(SIZES[i]),
            Item::Undo => self.undo(false),
            Item::Redo => self.undo(true),
            Item::Clear => self.clear_all(),
            Item::Hide => self.set_hidden(!self.hidden),
            Item::Guides => self.cycle_guides(),
            Item::Save => self.save_print(),
            Item::Rec if self.rec == gravar::State::Idle || self.rec == gravar::State::Failed => self.bar.rec_menu = true,
            Item::Rec => self.toggle_record(),
            Item::Fade => self.toggle_fade(),
            Item::Spot => self.toggle_look(false),
            Item::Zoom => self.toggle_look(true),
        }
    }

    /// Mostra a barra ou guarda na bandeja (sai do modo desenho junto).
    fn set_bar(&mut self, show: bool) {
        if !show && self.draw {
            self.set_draw(false);
        }
        self.hide_tip();
        self.bar.hover = None;
        self.bar.shown = show;
        unsafe { ShowWindow(self.bar.hwnd, if show { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
        self.paint_bar();
    }

    fn tray(&mut self, event: u32) {
        match event {
            NIN_SELECT | NIN_KEYSELECT => self.set_bar(!self.bar.shown),
            WM_CONTEXTMENU => {
                use bandeja::Entry::{Item as It, Sep};
                let auto = instalar::autostart();
                let mut v = vec![
                    It { id: MENU_BAR, text: if self.bar.shown { "Guardar a barra na bandeja" } else { "Mostrar a barra" }, checked: false },
                    It { id: MENU_DRAW, text: "Desenhar\tCtrl+Shift+D", checked: self.draw },
                    It { id: MENU_HIDE, text: "Esconder desenhos\tCtrl+Shift+H", checked: self.hidden },
                    It { id: MENU_CLEAR, text: "Apagar tudo\tCtrl+Shift+X", checked: false },
                    It { id: MENU_SAVE, text: "Salvar print\tCtrl+Shift+S", checked: false },
                    It { id: MENU_PRINT_REG, text: "Print de uma região (arraste)\tCtrl+Shift+A", checked: false },
                    Sep,
                ];
                self.rec_entries(&mut v);
                v.extend([Sep, It { id: MENU_BAR_FADE, text: "Barra transparente\tCtrl+Shift+B", checked: self.bar.faded }, It { id: MENU_AUTOSTART, text: "Abrir com o Windows", checked: auto }, Sep, It { id: MENU_QUIT, text: "Sair\tCtrl+Shift+Q", checked: false }]);
                let id = bandeja::menu(self.hwnd, &v);
                match id {
                    MENU_BAR => self.set_bar(!self.bar.shown),
                    MENU_DRAW => self.set_draw(!self.draw),
                    MENU_HIDE => self.set_hidden(!self.hidden),
                    MENU_CLEAR => self.clear_all(),
                    MENU_SAVE => self.save_print(),
                    MENU_PRINT_REG => self.print_region(),
                    MENU_AUTOSTART => {
                        instalar::set_autostart(!auto);
                    }
                    MENU_BAR_FADE => self.toggle_bar_fade(),
                    MENU_QUIT => unsafe {
                        DestroyWindow(self.hwnd);
                    },
                    id => self.rec_command(id),
                }
            }
            _ => {}
        }
    }

    fn show_tip(&mut self) {
        unsafe { KillTimer(self.bar.hwnd, TIP_TIMER) };
        let Some(item) = self.bar.hover else { return };
        let Some(&(_, r)) = self.bar.layout.items.iter().find(|(i, _)| *i == item) else { return };
        let mut wr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetWindowRect(self.bar.hwnd, &mut wr) };
        self.bar.tip_text = wide(&toolbar::label(item));
        let mut ti = self.tool_info();
        ti.lpszText = self.bar.tip_text.as_mut_ptr();
        let (x, y) = (wr.right + 6, wr.top + r.y0 + 4);
        // Barra encostada na direita da tela: dica do lado esquerdo.
        let x = if x + 260 > self.ox + self.canvas.w { wr.left - 260 } else { x };
        unsafe {
            SendMessageW(self.bar.tip, TTM_UPDATETIPTEXTW, 0, &ti as *const _ as LPARAM);
            SendMessageW(self.bar.tip, TTM_TRACKPOSITION, 0, (((y as u16 as u32) << 16) | x as u16 as u32) as LPARAM);
            SendMessageW(self.bar.tip, TTM_TRACKACTIVATE, 1, &ti as *const _ as LPARAM);
        }
        self.bar.tip_shown = true;
    }

    fn hide_tip(&mut self) {
        unsafe { KillTimer(self.bar.hwnd, TIP_TIMER) };
        if self.bar.tip_shown {
            let ti = self.tool_info();
            unsafe { SendMessageW(self.bar.tip, TTM_TRACKACTIVATE, 0, &ti as *const _ as LPARAM) };
            self.bar.tip_shown = false;
        }
    }

    // ---------- guias ----------

    /// Próximo modo das guias (desligado → terços → 16:9 → terços no 16:9).
    fn cycle_guides(&mut self) {
        self.guides = self.guides.next();
        self.show_guides();
        self.paint_bar();
    }

    /// Desenha as guias no monitor do cursor, uma janela fina por linha: cada
    /// imagem tem só a faixa da linha (2 espessuras) e é solta logo depois,
    /// porque o Windows guarda o quadro. Uma janela do monitor inteiro custaria
    /// 33 MB por troca (4K) e outro tanto no DWM enquanto as guias ficam ligadas.
    fn show_guides(&mut self) {
        unsafe {
            let (mx, my, w, h) = cursor_monitor();
            let lines = guias::lines(self.guides, w, h);
            let thick = guias::thickness(cursor_dpi());
            for (i, &hw) in self.guide_hwnds.iter().enumerate() {
                let Some(l) = lines.get(i) else {
                    ShowWindow(hw, SW_HIDE);
                    continue;
                };
                let b = guias::line_box(l, thick, w, h);
                let Some(mut s) = Surface::new(b.2 - b.0, b.3 - b.1) else { continue };
                guias::paint_line(s.pixels(), b, l, thick, w, h);
                s.present(hw, Some((mx + b.0, my + b.1)), None);
                ShowWindow(hw, SW_SHOWNOACTIVATE);
            }
            if !lines.is_empty() {
                // A barra fica por cima das linhas.
                SetWindowPos(self.bar.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    // ---------- print ----------

    /// Captura a tela inteira (com as anotações e as guias visíveis; a barra e
    /// o aviso ficam de fora) e codifica numa thread: a interface não trava.
    /// Com um print em andamento, a tela do aperto é copiada já (o BitBlt é
    /// rápido; o lento é o PNG) e espera a vez.
    fn save_print(&mut self) {
        let (ox, oy, w, h) = (self.ox, self.oy, self.canvas.w, self.canvas.h);
        match self.print.press(|| capture(ox, oy, w, h)) {
            Press::Start => self.start_print(None),
            Press::Queued => {}
            Press::Full => self.show_toast(false),
        }
    }

    /// Print de uma área: a cópia sai já (o BitBlt é rápido) e entra na fila do print.
    fn save_print_area(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let mut shot = capture(x, y, w, h);
        if shot.is_none() {
            return self.show_toast(false);
        }
        match self.print.press(|| shot.take()) {
            Press::Start => self.start_print(shot),
            Press::Queued => {}
            Press::Full => self.show_toast(false),
        }
    }

    /// Thread do print (`shot` = tela já copiada). Sem thread, roda aqui mesmo.
    fn start_print(&mut self, shot: Option<Surface>) {
        let (hwnd, ox, oy, w, h) = (self.hwnd as isize, self.ox, self.oy, self.canvas.w, self.canvas.h);
        // Por ponteiro: se a thread não abrir, o trabalho volta inteiro para cá.
        let job = Box::into_raw(Box::new(shot)) as usize;
        let run = move |job: usize| match *unsafe { Box::from_raw(job as *mut Option<Surface>) } {
            Some(s) => encode_and_save(s),
            None => capture_and_save(ox, oy, w, h),
        };
        let spawned = std::thread::Builder::new().spawn(move || {
            let saved = Box::into_raw(Box::new(run(job)));
            if unsafe { PostMessageW(hwnd as HWND, WM_SAVED, 0, saved as LPARAM) } == 0 {
                drop(unsafe { Box::from_raw(saved) });
            }
        });
        if spawned.is_err() {
            let r = run(job);
            self.saved(r);
        }
    }

    /// A thread terminou: entrega à área de transferência, avisa e começa o
    /// print que estava esperando.
    fn saved(&mut self, r: Saved) {
        let copied = !r.png.is_empty() && self.set_clipboard(r.png);
        self.show_toast(copied && r.files);
        if let Some(s) = self.print.done() {
            self.start_print(Some(s));
        }
    }

    /// Dono da área de transferência com renderização adiada: nada de 55 MB
    /// parados na memória; só o PNG (pequeno) fica guardado.
    fn set_clipboard(&mut self, png: Vec<u8>) -> bool {
        unsafe {
            let mut opened = false;
            for _ in 0..5 {
                opened = OpenClipboard(self.hwnd) != 0;
                if opened {
                    break;
                }
                Sleep(10);
            }
            if !opened {
                return false;
            }
            EmptyClipboard(); // avisa o dono antigo (nós): clip_png = None
            self.clip_png = Some(png);
            SetClipboardData(CF_DIB, null_mut());
            SetClipboardData(self.png_format, null_mut());
            CloseClipboard();
            true
        }
    }

    /// Alguém colou: gera o formato pedido a partir do PNG guardado.
    fn render_clipboard(&mut self, fmt: u32) {
        let Some(png) = &self.clip_png else { return };
        let mem = if fmt == CF_DIB {
            dib_from_png(png)
        } else if fmt == self.png_format {
            global_copy(png)
        } else {
            null_mut()
        };
        if !mem.is_null() {
            unsafe { SetClipboardData(fmt, mem) };
        }
    }

    /// Antes de sair (ou a pedido do Windows): entrega tudo que faltava.
    fn render_all_clipboard(&mut self) {
        if self.clip_png.is_none() || unsafe { OpenClipboard(self.hwnd) } == 0 {
            return;
        }
        for fmt in [CF_DIB, self.png_format] {
            self.render_clipboard(fmt);
        }
        unsafe { CloseClipboard() };
    }

    /// ✓ (ou ✗) por 1 s no meio embaixo do monitor do cursor.
    fn show_toast(&mut self, ok: bool) {
        unsafe {
            let sc = cursor_dpi().max(96) as f32 / 96.0;
            let (w, h) = toolbar::toast_size(sc);
            let Some(mut s) = Surface::new(w, h) else { return };
            toolbar::paint_toast(s.pixels(), sc, ok);
            let (mx, my, mw, mh) = cursor_monitor();
            s.present(self.toast_hwnd, Some((mx + (mw - w) / 2, my + mh - h - (90.0 * sc) as i32)), None);
            ShowWindow(self.toast_hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(self.toast_hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            SetTimer(self.hwnd, TOAST_TIMER, TOAST_MS, None);
        }
    }

    fn tool_info(&self) -> TTTOOLINFOW {
        TTTOOLINFOW {
            // Sem manifesto o Windows carrega o comctl32 v5, que recusa o
            // tamanho novo (com lpReserved): usa o TTTOOLINFOW_V2_SIZE.
            cbSize: core::mem::offset_of!(TTTOOLINFOW, lpReserved) as u32,
            uFlags: TTF_TRACK | TTF_ABSOLUTE,
            hwnd: self.bar.hwnd,
            uId: 1,
            ..Default::default()
        }
    }
}

/// Retângulo (x, y, largura, altura) do monitor onde o cursor está.
fn cursor_monitor() -> (i32, i32, i32, i32) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    monitor_at(pt.x, pt.y)
}

/// DPI do monitor onde o cursor está (as guias e o aviso vão para ele).
fn cursor_dpi() -> u32 {
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let (mut x, mut y) = (96, 96);
        if GetDpiForMonitor(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), MDT_EFFECTIVE_DPI, &mut x, &mut y) != 0 {
            return GetDpiForSystem();
        }
        x
    }
}

/// Escolha da janela ou da região. O mouse chega por um gancho WH_MOUSE_LL (o
/// clique não passa para o programa embaixo) e só existe enquanto escolhe.
struct Picking {
    kind: gravar::Pick,
    hook: HHOOK,
    /// Monitor do cursor no começo: a região fica dentro dele.
    mon: (i32, i32, i32, i32),
    /// O que escurece: o monitor (região) ou todas as telas (janela).
    area: (i32, i32, i32, i32),
    /// Escurecido fora da seleção, moldura dela e a etiqueta (caixas sólidas).
    dim: [HWND; 4],
    edge: [HWND; 4],
    label: HWND,
    from: Option<(i32, i32)>,
    /// Janela: a que está sob o cursor.
    target: HWND,
    /// Região para print (Ctrl+Shift+A), não para gravar.
    print: bool,
}

/// Classes das caixas sólidas: o escurecido (preto) e a moldura (vermelha).
const DARK_CLASS: &str = "RabiscoEscuro";
const EDGE_CLASS: &str = "RabiscoBorda";
/// Escurecido da escolha (preto a 45%).
const PICK_DIM: u8 = 115;

unsafe extern "system" fn pick_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 && app().pick.is_some() {
            let m = &*(lp as *const MSLLHOOKSTRUCT);
            let ev = match wp as u32 {
                WM_MOUSEMOVE => Some(0),
                WM_LBUTTONDOWN => Some(1),
                WM_LBUTTONUP => Some(2),
                WM_RBUTTONUP => Some(3),
                WM_RBUTTONDOWN => Some(4),
                _ => None,
            };
            if let Some(ev) = ev {
                let xy = (m.pt.x as u16 as isize) | (m.pt.y as u16 as isize) << 16;
                PostMessageW(app().hwnd, WM_PICK, ev, xy);
                // O clique é da escolha, não do programa embaixo; o movimento passa.
                if ev != 0 {
                    return 1;
                }
            }
        }
        CallNextHookEx(null_mut(), code, wp, lp)
    }
}

/// Janela de cima sob o ponto (GA_ROOT), pulando as do Rabisco (a tela no modo
/// desenho, a barra) e as em camadas que deixam o clique passar. A área de
/// trabalho não conta.
fn window_at(x: i32, y: i32) -> HWND {
    unsafe {
        let pt = POINT { x, y };
        let me = windows_sys::Win32::System::Threading::GetCurrentProcessId();
        let mut h = GetAncestor(WindowFromPoint(pt), GA_ROOT);
        while !h.is_null() {
            let mut pid = 0;
            GetWindowThreadProcessId(h, &mut pid);
            let mut wr: RECT = core::mem::zeroed();
            GetWindowRect(h, &mut wr);
            let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            let skip = pid == me || IsWindowVisible(h) == 0 || IsIconic(h) != 0 || PtInRect(&wr, pt) == 0 || ex & (WS_EX_LAYERED | WS_EX_TRANSPARENT) == WS_EX_LAYERED | WS_EX_TRANSPARENT;
            if !skip {
                break;
            }
            h = GetWindow(h, GW_HWNDNEXT);
        }
        if h == GetShellWindow() || h == GetDesktopWindow() { null_mut() } else { h }
    }
}

/// Mostra a escolha: escurece `area` fora de `sel`, põe a moldura nela e a
/// etiqueta em cima (nome e tamanho, ou o que fazer, sem seleção). `mon` = o
/// monitor onde a etiqueta fica.
fn pick_show(pk: &mut Picking, sel: Option<(i32, i32, i32, i32)>, mon: (i32, i32, i32, i32)) {
    let r = sel.unwrap_or((pk.area.0, pk.area.1, 0, 0));
    for (h, b) in pk.dim.iter().zip(gravar::around(r, pk.area)) {
        place(*h, b);
    }
    let t = (3 * cursor_dpi().max(96) as i32 + 48) / 96;
    for (h, b) in pk.edge.iter().zip(gravar::ring(r, t)) {
        place(*h, if sel.is_some() { b } else { (0, 0, 0, 0) });
    }
    let text = match (sel, pk.kind, pk.print) {
        (Some(s), gravar::Pick::Window, _) => format!("{}   {} × {}", window_title(pk.target), s.2, s.3),
        (Some(s), ..) => format!("{} × {}", s.2, s.3),
        (None, gravar::Pick::Window, _) => "Clique na janela que vai ser gravada   ·   Esc cancela".into(),
        (None, _, false) => "Arraste a área que vai ser gravada   ·   Esc cancela".into(),
        (None, _, true) => "Arraste a área do print   ·   Esc cancela".into(),
    };
    let sc = cursor_dpi().max(96) as f32 / 96.0;
    let Some(sf) = label_surface(&text, sc, 0) else { return };
    let gap = (8.0 * sc) as i32;
    let (x, y) = match sel {
        // Em cima da seleção; sem espaço, dentro dela.
        Some(s) if s.1 - sf.h - gap >= mon.1 => (s.0, s.1 - sf.h - gap),
        Some(s) => (s.0 + gap, s.1 + gap),
        None => (mon.0 + (mon.2 - sf.w) / 2, mon.1 + 6 * gap),
    };
    let x = x.clamp(mon.0, (mon.0 + mon.2 - sf.w).max(mon.0));
    sf.present(pk.label, Some((x, y)), None);
    unsafe {
        ShowWindow(pk.label, SW_SHOWNOACTIVATE);
        SetWindowPos(pk.label, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Caixa de cor sólida (o fundo da classe), meio transparente por `alpha`,
/// fora da captura e com o clique passando. Sem imagem: custa só a janela.
fn solid_box(class: &str, alpha: u8) -> HWND {
    let (class, title) = (wide(class), wide("Rabisco"));
    unsafe {
        let h = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        SetLayeredWindowAttributes(h, 0, alpha, LWA_ALPHA);
        SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE);
        h
    }
}

/// Põe a caixa em (x, y, w, h), por cima de tudo; vazia, esconde.
fn place(h: HWND, (x, y, w, hh): (i32, i32, i32, i32)) {
    unsafe {
        if w <= 0 || hh <= 0 {
            ShowWindow(h, SW_HIDE);
        } else {
            SetWindowPos(h, HWND_TOPMOST, x, y, w, hh, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
    }
}

/// Contorno visível da janela (sem a sombra e a borda invisível do Windows 10/11).
fn window_bounds(h: HWND) -> Option<(i32, i32, i32, i32)> {
    if h.is_null() {
        return None;
    }
    let mut r: RECT = unsafe { core::mem::zeroed() };
    let ok = unsafe {
        windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute(h, windows_sys::Win32::Graphics::Dwm::DWMWA_EXTENDED_FRAME_BOUNDS as u32, &mut r as *mut RECT as _, size_of::<RECT>() as u32) == 0
            || GetWindowRect(h, &mut r) != 0
    };
    (ok && r.right > r.left && r.bottom > r.top).then_some((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

/// Título da janela (cortado em 50 letras) ou o nome do programa sem título.
fn window_title(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32) }.max(0) as usize;
    let t = String::from_utf16_lossy(&buf[..n]);
    let t = t.trim();
    match t.chars().count() {
        0 => "Janela".into(),
        c if c > 50 => t.chars().take(49).chain(Some('…')).collect(),
        _ => t.into(),
    }
}

/// Etiqueta: fundo escuro opaco, texto branco em Segoe UI (GDI, antialias em
/// cinza) e, com `dot` > 0, a bola vermelha desse diâmetro na direita.
fn label_surface(text: &str, sc: f32, dot: i32) -> Option<Surface> {
    const BG: u32 = 0xFF1C_1C1E;
    let px = (14.0 * sc).round() as i32;
    let pad = (10.0 * sc).round() as i32;
    let u: Vec<u16> = text.encode_utf16().collect();
    let face = wide("Segoe UI");
    unsafe {
        let font = CreateFontW(-px, 0, 0, 0, FW_SEMIBOLD as i32, 0, 0, 0, DEFAULT_CHARSET as u32, OUT_TT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32, ANTIALIASED_QUALITY as u32, (DEFAULT_PITCH | FF_SWISS) as u32, face.as_ptr());
        if font.is_null() {
            return None;
        }
        let mdc = CreateCompatibleDC(null_mut());
        let old = SelectObject(mdc, font);
        let mut sz = SIZE { cx: 0, cy: 0 };
        GetTextExtentPoint32W(mdc, u.as_ptr(), u.len() as i32, &mut sz);
        SelectObject(mdc, old);
        DeleteDC(mdc);
        let dot_w = if dot > 0 { dot + pad * 6 / 10 } else { 0 };
        let (w, h) = (pad + sz.cx + dot_w + pad, sz.cy.max(dot) + pad);
        let Some(mut s) = Surface::new(w, h) else {
            DeleteObject(font);
            return None;
        };
        s.pixels().fill(BG);
        let old = SelectObject(s.dc, font);
        SetBkMode(s.dc, TRANSPARENT as i32);
        SetTextColor(s.dc, 0x00FF_FFFF);
        TextOutW(s.dc, pad, (h - sz.cy) / 2, u.as_ptr(), u.len() as i32);
        GdiFlush();
        SelectObject(s.dc, old);
        DeleteObject(font);
        // O GDI zera o alfa onde escreve; o fundo é opaco, então tudo volta a 255.
        for p in s.pixels() {
            *p |= 0xFF00_0000;
        }
        if dot > 0 {
            let (cx, cy, r) = ((w - pad) as f32 - dot as f32 / 2.0, h as f32 / 2.0, dot as f32 / 2.0);
            let red = toolbar::REC;
            let px = s.pixels();
            for y in 0..h {
                for x in (w - pad - dot).max(0)..(w - pad) {
                    let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                    let a = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
                    let mix = |sh: u32| ((red >> sh & 0xFF) as f32 * a + (BG >> sh & 0xFF) as f32 * (1.0 - a)) as u32;
                    px[(y * w + x) as usize] = 0xFF00_0000 | mix(16) << 16 | mix(8) << 8 | mix(0);
                }
            }
        }
        Some(s)
    }
}

/// Janela em camadas sem dono, por cima de tudo e com o clique passando
/// (guias, aviso, holofote, zoom).
fn aux_window() -> HWND {
    let class = wide("RabiscoAviso");
    let title = wide("Rabisco");
    unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    }
}

/// Retângulo (x, y, largura, altura) do monitor que tem o ponto (ou o mais perto).
fn monitor_at(x: i32, y: i32) -> (i32, i32, i32, i32) {
    unsafe {
        let pt = POINT { x, y };
        let mut mi: MONITORINFO = core::mem::zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut mi) == 0 {
            return virtual_screen();
        }
        let r = mi.rcMonitor;
        (r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1))
    }
}

/// Fila do print: um sendo gravado e no máximo um esperando, já capturado.
struct PrintQueue<T> {
    busy: bool,
    pending: Option<T>,
}

impl<T> Default for PrintQueue<T> {
    fn default() -> Self {
        PrintQueue { busy: false, pending: None }
    }
}

enum Press {
    /// Livre: começa agora (a thread captura).
    Start,
    /// Ocupado: a tela foi capturada e espera.
    Queued,
    /// Já tem um esperando (ou a captura falhou): este se perde.
    Full,
}

impl<T> PrintQueue<T> {
    fn press(&mut self, capture: impl FnOnce() -> Option<T>) -> Press {
        if !self.busy {
            self.busy = true;
            return Press::Start;
        }
        if self.pending.is_some() {
            return Press::Full;
        }
        match capture() {
            Some(t) => {
                self.pending = Some(t);
                Press::Queued
            }
            None => Press::Full,
        }
    }

    /// O print terminou: devolve o próximo (que já começa).
    fn done(&mut self) -> Option<T> {
        let next = self.pending.take();
        self.busy = next.is_some();
        next
    }
}

/// Resultado da thread do print.
struct Saved {
    /// Vazio = a captura falhou.
    png: Vec<u8>,
    /// Os dois arquivos foram gravados.
    files: bool,
}

/// `rabisco_AAAA-MM-DD_HHhMMmSS.png`
fn file_name(y: u16, mo: u16, d: u16, h: u16, mi: u16, s: u16) -> String {
    format!("rabisco_{y:04}-{mo:02}-{d:02}_{h:02}h{mi:02}m{s:02}.png")
}

/// Grava o print com data e, por cima, `ultimo.png` (caminho fixo para outra
/// ferramenta ler). `ultimo.png` vai por arquivo temporário + rename: quem lê
/// nunca pega meio arquivo.
fn write_files(dir: &std::path::Path, name: &str, png: &[u8]) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let tmp = dir.join("ultimo.png.tmp");
    write_new(dir, name, png)
        && std::fs::write(&tmp, png).is_ok()
        && std::fs::rename(&tmp, dir.join("ultimo.png")).is_ok()
}

/// Grava num arquivo novo (`create_new`): se o nome já existe (dois prints no
/// mesmo segundo), tenta `_2`, `_3`... em vez de sobrescrever.
fn write_new(dir: &std::path::Path, name: &str, png: &[u8]) -> bool {
    use std::io::Write;
    let stem = name.strip_suffix(".png").unwrap_or(name);
    for n in 1..1000 {
        let path = if n == 1 { dir.join(name) } else { dir.join(format!("{stem}_{n}.png")) };
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => return f.write_all(png).is_ok(),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return false,
        }
    }
    false
}

fn pictures_dir() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(std::env::var_os("USERPROFILE")?).join("Pictures").join("Rabisco"))
}

/// Copia a área (x, y, w, h) da tela para uma DIB.
fn capture(ox: i32, oy: i32, w: i32, h: i32) -> Option<Surface> {
    let s = Surface::new(w, h)?;
    unsafe {
        let screen = GetDC(null_mut());
        let ok = BitBlt(s.dc, 0, 0, w, h, screen, ox, oy, SRCCOPY | CAPTUREBLT);
        ReleaseDC(null_mut(), screen);
        (ok != 0).then_some(s)
    }
}

/// Roda na thread: copia a tela virtual (CAPTUREBLT pega as janelas em
/// camadas, ou seja, as anotações), codifica e grava.
fn capture_and_save(ox: i32, oy: i32, w: i32, h: i32) -> Saved {
    match capture(ox, oy, w, h) {
        Some(s) => encode_and_save(s),
        None => Saved { png: Vec::new(), files: false },
    }
}

/// Codifica a tela copiada e grava.
fn encode_and_save(mut s: Surface) -> Saved {
    let png = png::encode(s.w as usize, s.h as usize, s.pixels());
    drop(s);
    let t = unsafe {
        let mut t = core::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    let name = file_name(t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
    let files = pictures_dir().is_some_and(|dir| write_files(&dir, &name, &png));
    Saved { png, files }
}

fn global_copy(bytes: &[u8]) -> HANDLE {
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if mem.is_null() {
            return null_mut();
        }
        let p = GlobalLock(mem) as *mut u8;
        if p.is_null() {
            GlobalFree(mem);
            return null_mut();
        }
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        GlobalUnlock(mem);
        mem
    }
}

/// CF_DIB (32 bits, de baixo para cima) a partir do PNG guardado.
fn dib_from_png(png: &[u8]) -> HANDLE {
    let Some((w, h, px)) = png::decode(png) else { return null_mut() };
    let head = size_of::<BITMAPINFOHEADER>();
    let mut buf = vec![0u8; head + w * h * 4];
    let bih = BITMAPINFOHEADER {
        biSize: head as u32,
        biWidth: w as i32,
        biHeight: h as i32,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB as u32,
        biSizeImage: (w * h * 4) as u32,
        ..unsafe { core::mem::zeroed() }
    };
    unsafe { core::ptr::copy_nonoverlapping(&bih as *const _ as *const u8, buf.as_mut_ptr(), head) };
    for (y, row) in px.chunks_exact(w).enumerate() {
        let at = head + (h - 1 - y) * w * 4;
        for (x, p) in row.iter().enumerate() {
            buf[at + x * 4..at + x * 4 + 4].copy_from_slice(&p.to_le_bytes());
        }
    }
    global_copy(&buf)
}

/// Shift: linha e seta em passos de 45 graus, retângulo e elipse quadrados.
fn constrain(tool: Tool, ax: f32, ay: f32, x: f32, y: f32) -> (f32, f32) {
    let (dx, dy) = (x - ax, y - ay);
    match tool {
        Tool::Line | Tool::Arrow => {
            let step = std::f32::consts::FRAC_PI_4;
            let ang = (dy.atan2(dx) / step).round() * step;
            let len = dx.hypot(dy);
            (ax + len * ang.cos(), ay + len * ang.sin())
        }
        _ => {
            let m = dx.abs().max(dy.abs());
            (ax + m.copysign(dx), ay + m.copysign(dy))
        }
    }
}

/// Cursor redondo do tamanho e da cor do pincel, com contorno escuro para
/// aparecer em fundo claro. `cross` põe uma cruz no meio (formas).
unsafe fn make_cursor(diam: f32, rgb: u32, cross: bool) -> HCURSOR {
    let s = ((diam + 6.0).ceil() as i32).clamp(17, 256) | 1;
    let c = s as f32 / 2.0;
    let r = (diam / 2.0).max(1.5);
    let mut px = vec![0u32; (s * s) as usize];
    let put = |px: &mut Vec<u32>, i: usize, a: f32, col: u32| {
        let a = (a.clamp(0.0, 1.0) * 255.0) as u32;
        if a > px[i] >> 24 {
            px[i] = (a << 24) | col; // ícone ARGB: não pré-multiplicado
        }
    };
    for y in 0..s {
        for x in 0..s {
            let i = (y * s + x) as usize;
            let d = (x as f32 + 0.5 - c).hypot(y as f32 + 0.5 - c);
            put(&mut px, i, 0.9 - ((d - r - 1.0).abs() - 0.6), 0x000000);
            put(&mut px, i, 1.0 - ((d - r).abs() - 0.4), rgb);
            if d < 1.2 {
                put(&mut px, i, 1.0, rgb);
            }
            let (ax, ay) = ((x - s / 2).abs(), (y - s / 2).abs());
            if cross && ((ax == 0 && ay <= 5) || (ay == 0 && ax <= 5)) {
                put(&mut px, i, 1.0, rgb);
            }
        }
    }
    unsafe { argb_icon(&px, s, Some((s / 2) as u32)) }
}

/// Ícone (`hot` = None) ou cursor a partir de pixels ARGB `s`x`s`.
unsafe fn argb_icon(px: &[u32], s: i32, hot: Option<u32>) -> HICON {
    unsafe {
        let mut bmi: BITMAPINFO = core::mem::zeroed();
        bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = s;
        bmi.bmiHeader.biHeight = -s;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB as u32;
        let mut bits = null_mut();
        let color = CreateDIBSection(null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        core::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len());
        let mask = CreateBitmap(s, s, 1, 1, null());
        let h = hot.unwrap_or(0);
        let info = ICONINFO { fIcon: hot.is_none() as BOOL, xHotspot: h, yHotspot: h, hbmMask: mask, hbmColor: color };
        let cur = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        cur
    }
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if APP.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let a = app();
        match msg {
            WM_POINTERDOWN => {
                if a.draw && (wp >> 16) as u32 & POINTER_FLAG_FIRSTBUTTON != 0 {
                    a.pointer_down(pointer_id(wp));
                }
                0
            }
            WM_POINTERUPDATE => {
                if a.look.is_some() {
                    let (x, y) = lparam_point(lp);
                    a.look_pointer(x, y);
                }
                a.pointer_move(pointer_id(wp));
                0
            }
            WM_POINTERUP | WM_POINTERCAPTURECHANGED => {
                a.pointer_up(pointer_id(wp));
                0
            }
            WM_POINTERWHEEL | WM_MOUSEWHEEL => {
                a.wheel(((wp >> 16) as u16 as i16).signum() as i32);
                0
            }
            WM_SETCURSOR if a.draw => {
                SetCursor(a.cursor);
                1
            }
            WM_KEYDOWN if a.draw => {
                a.key(wp as u16);
                0
            }
            WM_CHAR if a.text.is_some() => {
                a.text_char(wp as u16);
                0
            }
            WM_TIMER if wp == CARET_TIMER => {
                if let Some(ed) = a.text.as_mut() {
                    ed.caret_on = !ed.caret_on;
                    a.text_update(false, false);
                }
                0
            }
            WM_TIMER if wp == ZOOM_TIMER => {
                a.zoom_tick();
                0
            }
            WM_TIMER if wp == FADE_TIMER => {
                a.fade_tick();
                0
            }
            WM_HOTKEY => {
                a.hotkey(wp as i32);
                0
            }
            WM_TIMER if wp == IDLE_TIMER => {
                a.idle();
                0
            }
            WM_TIMER if wp == TOAST_TIMER => {
                KillTimer(hwnd, TOAST_TIMER);
                ShowWindow(a.toast_hwnd, SW_HIDE);
                0
            }
            WM_REC_STARTED => {
                let (x, y) = (wp as u16 as i16 as i32, (wp >> 16) as u16 as i16 as i32);
                let (w, h) = ((lp & 0xFFFF) as i32, (lp >> 16 & 0xFFFF) as i32);
                if a.rec != gravar::State::Idle && a.rec_dot.is_null() {
                    let (d, m) = dot_size();
                    a.rec_area = (x, y, w, d, m);
                    a.rec_rect = (x, y, w, h);
                    a.rec_t0 = Some(std::time::Instant::now());
                    a.rec_dot = rec_dot(null_mut(), a.rec_area, 0);
                    a.rec_frame = core::array::from_fn(|_| solid_box(EDGE_CLASS, 255));
                    a.rec_frame_place();
                    SetTimer(hwnd, REC_TIMER, 1000, None);
                }
                0
            }
            WM_TIMER if wp == REC_TIMER => {
                // Janela gravada que mudou de lugar: a moldura e o contador vão junto.
                if let Some(r) = window_bounds(a.rec_target).filter(|r| *r != a.rec_rect) {
                    a.rec_rect = r;
                    a.rec_area = (r.0, r.1, r.2, a.rec_area.3, a.rec_area.4);
                    a.rec_frame_place();
                }
                if let Some(t0) = a.rec_t0 {
                    rec_dot(a.rec_dot, a.rec_area, t0.elapsed().as_secs());
                }
                0
            }
            WM_PICK => {
                a.pick_event(wp, lp as u16 as i16 as i32, (lp >> 16) as u16 as i16 as i32);
                0
            }
            WM_REC_DONE => {
                a.record_done();
                0
            }
            WM_SAVED => {
                a.saved(*Box::from_raw(lp as *mut Saved));
                0
            }
            WM_RENDERFORMAT => {
                a.render_clipboard(wp as u32);
                0
            }
            WM_RENDERALLFORMATS => {
                a.render_all_clipboard();
                0
            }
            WM_DESTROYCLIPBOARD => {
                a.clip_png = None;
                0
            }
            WM_DISPLAYCHANGE => {
                a.display_changed();
                0
            }
            bandeja::WM_TRAY => {
                a.tray(lp as u32 & 0xFFFF);
                0
            }
            WM_SHOW_BAR => {
                a.set_bar(true);
                0
            }
            m if m == a.taskbar_created && m != 0 => {
                bandeja::add(hwnd, a.tray_icon);
                0
            }
            WM_DESTROY => {
                // Sair gravando fecha o vídeo antes (senão sobra só o .part).
                if !a.rec_stop.is_null() {
                    SetEvent(a.rec_stop);
                }
                if let Some(c) = a.rec_child.as_mut() {
                    // Até 5 s para fechar o MP4: um filho travado no Finalize não segura o app.
                    use std::os::windows::io::AsRawHandle;
                    if windows_sys::Win32::System::Threading::WaitForSingleObject(c.as_raw_handle() as HANDLE, 5000) != 0 {
                        let _ = c.kill();
                    }
                    if !c.wait().is_ok_and(|s| s.success())
                        && let Some(p) = &a.rec_part
                    {
                        let _ = std::fs::remove_file(p);
                    }
                }
                bandeja::remove(hwnd);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

unsafe extern "system" fn bar_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if APP.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let a = app();
        let item_at = |a: &App, id: u32| {
            let mut info: POINTER_INFO = core::mem::zeroed();
            GetPointerInfo(id, &mut info);
            let (x, y) = a.bar_point(info.ptPixelLocation.x, info.ptPixelLocation.y);
            toolbar::hit(&a.bar.layout, x, y)
        };
        match msg {
            WM_NCHITTEST => {
                // Fundo e alça arrastam a barra; botões são área de cliente.
                let (sx, sy) = lparam_point(lp);
                let (x, y) = a.bar_point(sx, sy);
                match toolbar::hit(&a.bar.layout, x, y) {
                    Some(Item::Grip) | None => HTCAPTION as LRESULT,
                    Some(_) => HTCLIENT as LRESULT,
                }
            }
            WM_POINTERUPDATE => {
                a.bar_near(true);
                let item = item_at(a, pointer_id(wp));
                a.bar_hover(item);
                0
            }
            WM_POINTERDOWN => {
                a.bar_near(true);
                if (wp >> 16) as u32 & POINTER_FLAG_FIRSTBUTTON != 0 {
                    if let Some(item) = item_at(a, pointer_id(wp)) {
                        a.bar_click(item);
                    }
                }
                0
            }
            WM_POINTERUP if a.bar.rec_menu => {
                // Aberto no aperto, o menu recebia o soltar do botão e fechava.
                a.bar.rec_menu = false;
                a.rec_menu();
                0
            }
            WM_POINTERLEAVE => {
                a.bar_hover(None);
                0
            }
            WM_NCPOINTERUPDATE => {
                a.bar_near(true);
                // Em cima da alça ou do fundo: só a dica da alça.
                let (sx, sy) = lparam_point(lp);
                let (x, y) = a.bar_point(sx, sy);
                let item = toolbar::hit(&a.bar.layout, x, y).filter(|i| *i == Item::Grip);
                a.bar_hover(item);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_NCMOUSELEAVE | WM_MOUSELEAVE => {
                a.bar_hover(None);
                0
            }
            WM_POINTERWHEEL | WM_MOUSEWHEEL => {
                a.resize_pen(((wp >> 16) as u16 as i16).signum() as i32);
                0
            }
            WM_TIMER if wp == BAR_TIMER => {
                if !a.cursor_on_bar() {
                    a.bar_near(false);
                }
                0
            }
            WM_TIMER if wp == TIP_TIMER => {
                a.show_tip();
                0
            }
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_DPICHANGED => {
                let r = &*(lp as *const RECT);
                SetWindowPos(hwnd, null_mut(), r.left, r.top, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                a.layout_bar();
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// Desktop novo só para esta thread: as janelas não aparecem nem pegam o mouse.
unsafe fn perf_desktop() -> bool {
    use windows_sys::Win32::System::StationsAndDesktops::{CreateDesktopW, SetThreadDesktop};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    let name = wide(&format!("RabiscoPerf{}", unsafe { GetCurrentProcessId() }));
    let d = unsafe { CreateDesktopW(name.as_ptr(), null(), null(), 0, GENERIC_ALL, null()) };
    !d.is_null() && unsafe { SetThreadDesktop(d) } != 0
}

/// (WorkingSet, PrivateUsage) do processo, em bytes.
fn process_memory() -> (usize, usize) {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut m: PROCESS_MEMORY_COUNTERS_EX = unsafe { core::mem::zeroed() };
    m.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut m as *mut _ as *mut _, m.cb) };
    (m.WorkingSetSize, m.PrivateUsage)
}

/// Passa pelos estados do app e grava "estado working_set privado" por linha
/// em `file` (arquivo, não stdout: o std::io da saída custaria ~6 KB no exe).
unsafe fn perf_memoria(file: &str) {
    let a = app();
    let mut out = String::new();
    let mut mark = |name: &str| {
        for _ in 0..5 {
            let mut msg: MSG = unsafe { core::mem::zeroed() };
            while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
                unsafe { DispatchMessageW(&msg) };
            }
            unsafe { Sleep(20) };
        }
        let (ws, private) = process_memory();
        out += &format!("{name} {ws} {private}\n");
    };
    mark("mouse_parado");
    // Gravação (Ctrl+Shift+F): o motor roda em outro processo, então este não
    // pode crescer nem gravando nem depois (logo depois do "mouse_parado", que
    // é o estado mais estável: o privado varia ~10 KB entre execuções). O vídeo vai para uma pasta
    // temporária (o USERPROFILE do filho), não para os Vídeos do Rafa.
    let tmp = std::env::temp_dir().join(format!("rabisco-perf-video-{}", std::process::id()));
    unsafe { std::env::set_var("USERPROFILE", &tmp) };
    let pump_until = |ms: u64, done: &dyn Fn() -> bool| {
        let t = std::time::Instant::now();
        while !done() && t.elapsed().as_millis() < ms as u128 {
            let mut msg: MSG = unsafe { core::mem::zeroed() };
            while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
                unsafe { DispatchMessageW(&msg) };
            }
            unsafe { Sleep(10) };
        }
    };
    a.toggle_record();
    pump_until(20_000, &|| !app().rec_dot.is_null() || app().rec != gravar::State::Recording);
    pump_until(2_000, &|| false);
    mark("gravando");
    a.toggle_record();
    pump_until(20_000, &|| app().rec == gravar::State::Idle || app().rec == gravar::State::Failed);
    let videos = std::fs::read_dir(tmp.join("Videos").join("Rabisco")).map_or(0, |d| d.filter(|e| e.as_ref().is_ok_and(|e| e.path().extension().is_some_and(|x| x == "mp4"))).count());
    mark(if a.rec == gravar::State::Idle && videos == 1 { "gravou" } else { "gravou_falhou" });
    // Região pelo menu: a tela escurecida só existe enquanto escolhe. O mouse
    // vem direto para o pick_event (o gancho não vê o mouse do desktop do Rafa).
    let (mx, my, _, _) = cursor_monitor();
    a.pick_start(gravar::Pick::Region, false);
    a.pick_event(1, mx + 101, my + 101);
    a.pick_event(0, mx + 400, my + 300);
    mark("escolhendo_regiao");
    a.pick_event(2, mx + 742, my + 462);
    pump_until(20_000, &|| !app().rec_dot.is_null() || app().rec != gravar::State::Recording);
    pump_until(1_000, &|| false);
    a.toggle_record();
    pump_until(20_000, &|| app().rec == gravar::State::Idle || app().rec == gravar::State::Failed);
    let videos = std::fs::read_dir(tmp.join("Videos").join("Rabisco")).map_or(0, |d| d.filter(|e| e.as_ref().is_ok_and(|e| e.path().extension().is_some_and(|x| x == "mp4"))).count());
    mark(if a.rec == gravar::State::Idle && videos == 2 && a.pick.is_none() { "gravou_regiao" } else { "gravou_regiao_falhou" });
    let _ = std::fs::remove_dir_all(&tmp);
    a.set_draw(true);
    mark("desenho_vazio");
    a.canvas.strokes = canvas::sample_strokes(200);
    a.redraw_all();
    mark("desenho_200");
    a.idle(); // o que o IDLE_TIMER faz depois de 8 s parado
    mark("desenho_8s");
    a.set_draw(false);
    mark("mouse_200");
    a.toggle_look(false);
    let (mx, my, mw, mh) = a.look.as_ref().map_or((0, 0, 0, 0), |l| l.mon);
    for i in 0..50 {
        a.look_move(mx + mw / 4 + i * 13, my + mh / 3 + i * 5);
    }
    mark("holofote_ligado");
    a.toggle_look(false);
    mark("holofote_desligado");
    a.idle();
    a.set_draw(false);
    mark("mouse_fim");
    // Mais três voltas: se o "mouse_fim" crescer de novo, é vazamento.
    for _ in 0..3 {
        a.toggle_look(false);
        a.toggle_look(false);
        a.idle();
        a.set_draw(false);
    }
    mark("mouse_fim_3x");
    let _ = std::fs::write(file, out);
}

/// `--gravar <hwnd> <evento> <modo> <som> <final.mp4> <pid do pai>`: o processo
/// da gravação. Grava no `.part` que o pai reservou até o evento ser sinalizado
/// ou o pai morrer, e avisa a janela. Sai com 0 (gravou), 1 (falhou) ou 2
/// (parou antes do primeiro quadro).
fn record_child(a: &[String]) -> i32 {
    use windows_sys::Win32::System::Threading::{OpenEventW, OpenProcess, WaitForSingleObject};
    let (Some(hwnd), Some(name), Some(fin), Some(pid)) = (a.first().and_then(|h| h.parse().ok()), a.get(1), a.get(4), a.get(5).and_then(|p| p.parse().ok())) else { return 1 };
    // SYNCHRONIZE nos dois: o evento e o processo do pai.
    let (ev, parent) = unsafe { (OpenEventW(0x0010_0000, 0, wide(name).as_ptr()), OpenProcess(0x0010_0000, 0, pid)) };
    if ev.is_null() || parent.is_null() {
        return 1;
    }
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    // O motor olha a cada quadro (WAIT_OBJECT_0 = pediram para parar, ou o pai
    // morreu: crash ou taskkill, e ninguém mais pararia o filho).
    let mode = a.get(2).and_then(|m| gravar::parse_mode(m)).unwrap_or(gravar::Mode::Monitor);
    let som = a.get(3).map_or("sis", |s| s.as_str());
    let stop = || unsafe { WaitForSingleObject(ev, 0) == 0 || WaitForSingleObject(parent, 0) == 0 };
    record_to_videos(&stop, hwnd, mode, (som.contains("sis"), som.contains("mic")), std::path::Path::new(fin))
}

/// Grava em `fin` (pelo `.part` dele) e publica sem sobrescrever.
fn record_to_videos(stop: &dyn Fn() -> bool, hwnd: isize, mode: gravar::Mode, sound: (bool, bool), fin: &std::path::Path) -> i32 {
    let part = gravar::part_of(fin);
    let started = |x: i32, y: i32, w: i32, h: i32| {
        let wp = (x as u16 as usize) | (y as u16 as usize) << 16;
        unsafe { PostMessageW(hwnd as HWND, WM_REC_STARTED, wp, (w | h << 16) as LPARAM) };
    };
    let r = gravar::record(mode, gravar::FPS, sound, &part, stop, None, &started);
    if r.is_ok() && gravar::publish(&part, fin, false) {
        return 0;
    }
    // Sem o fim do MP4 o arquivo não abre: não deixa um vídeo quebrado com cara de bom.
    let _ = std::fs::remove_file(&part);
    if r.is_err_and(|e| e == gravar::CANCELED) { 2 } else { 1 }
}

/// (diâmetro, margem) do ponto de "gravando", pelo DPI do monitor do cursor.
fn dot_size() -> (i32, i32) {
    let sc = cursor_dpi().max(96) as f32 / 96.0;
    ((12.0 * sc).round() as i32, (10.0 * sc).round() as i32)
}

/// Ponto vermelho no canto de cima à direita da área (x, y, w; ponto d, margem
/// m) com o tempo gravado (mm:ss) à esquerda, excluído da captura: aparece na
/// tela e não no vídeo. `reuse` nulo cria a janela; senão só repinta.
fn rec_dot(reuse: HWND, (x, y, w, d, m): (i32, i32, i32, i32, i32), secs: u64) -> HWND {
    let sc = d as f32 / 12.0;
    let hwnd = if reuse.is_null() { aux_window() } else { reuse };
    let Some(sf) = label_surface(&format!("REC  {}", gravar::clock(secs)), sc, d) else { return hwnd };
    let pad = (10.0 * sc).round() as i32;
    unsafe {
        if reuse.is_null() {
            SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        }
        // A bola fica onde sempre esteve (canto, margem m); a etiqueta cresce para a esquerda.
        sf.present(hwnd, Some((x + w - m + pad - sf.w, y + m + d / 2 - sf.h / 2)), None);
        if reuse.is_null() {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
    hwnd
}

/// `--gravar-teste`: grava `segundos` no `modo` e escreve "chave valor" por
/// linha em `<saida>.txt` (memória antes, no pico e depois de soltar tudo).
unsafe fn gravar_teste(a: &[String]) -> bool {
    let mode = a.first().and_then(|m| gravar::parse_mode(m));
    // Segundos inteiros: ler f64 puxaria ~10 KB de parser para o exe.
    let secs = a.get(1).and_then(|s| s.parse::<u64>().ok());
    let (Some(mode), Some(secs), Some(out)) = (mode, secs, a.get(2)) else { return false };
    let fps = a.get(3).and_then(|f| f.parse().ok()).unwrap_or(gravar::FPS);
    let before = process_memory().1;
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let class = wide("RabiscoAviso");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: GetModuleHandleW(null()),
            lpszClassName: class.as_ptr(),
            ..core::mem::zeroed()
        };
        RegisterClassExW(&wc);
    }
    let fin = std::path::PathBuf::from(out);
    let part = gravar::part_of(&fin);
    let dot = core::cell::Cell::new(null_mut());
    // 5º argumento: "sis", "mic", "sis,mic" ou "-" (sem som, o padrão aqui).
    let som = a.get(4).map_or("-", |s| s.as_str());
    let sound = (som.contains("sis"), som.contains("mic"));
    let r = gravar::record(mode, fps, sound, &part, &|| false, Some(std::time::Duration::from_secs(secs)), &|x, y, w, _| {
        let (d, m) = dot_size();
        dot.set(rec_dot(null_mut(), (x, y, w, d, m), 0))
    });
    if !dot.get().is_null() {
        unsafe { DestroyWindow(dot.get()) };
    }
    let ok = r.is_ok() && gravar::publish(&part, &fin, true);
    let after = process_memory().1;
    let (d, m) = dot_size();
    let mut txt = format!("ok {}\nprivado_antes {before}\nprivado_depois {after}\nponto_d {d}\nponto_m {m}\n", ok as u8);
    match &r {
        Ok(st) => {
            let us = |d: std::time::Duration| d.as_micros();
            txt += &format!(
                "w {}\nh {}\nquadros {}\nrepetidos {}\nperdidos {}\ninicio_us {}\nparada_us {}\ncpu_us {}\nrelogio_us {}\nprivado_pico {}\nsom {}\nsom_desvio_us {}\nnotas {}\n",
                st.w, st.h, st.frames, st.repeated, st.lost, us(st.start), us(st.stop), us(st.cpu), us(st.wall), st.private_peak, st.audio, st.audio_drift * 1_000_000 / gravar::RATE as u64, st.notes
            );
        }
        Err(e) => txt += &format!("erro {e}\n"),
    }
    let mut txt_path = fin.into_os_string();
    txt_path.push(".txt");
    let _ = std::fs::write(txt_path, txt);
    ok
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    if flag("--desinstalar") {
        return instalar::uninstall();
    }
    if instalar::is_installer(&args) {
        return instalar::install();
    }
    // `--bandeja`: começa só com o ícone (é assim que o Windows abre).
    let start_hidden = flag("--bandeja");
    // `--perf-memoria <arquivo>` (escondido, para tests/perf_processo.rs): roda num
    // desktop próprio e invisível, sem instância única, bandeja nem atalhos,
    // mede a memória em cada estado e sai. A cópia aberta não fica sabendo.
    let perf_file = args.iter().position(|a| a == "--perf-memoria").and_then(|i| args.get(i + 1)).cloned();
    let perf = perf_file.is_some();
    // `--gravar-teste <modo> <segundos> <saida.mp4> [fps]` (escondido, para os
    // testes): grava sem interface e escreve os números em <saida.mp4>.txt.
    if let Some(i) = args.iter().position(|a| a == "--gravar-teste") {
        let ok = unsafe { gravar_teste(&args[i + 1..]) };
        std::process::exit(if ok { 0 } else { 1 });
    }
    if let Some(i) = args.iter().position(|a| a == "--gravar") {
        std::process::exit(record_child(&args[i + 1..]));
    }
    unsafe {
        if perf && !perf_desktop() {
            return;
        }
        // Uma instância só. A segunda mostra a barra da primeira e sai.
        let name = wide("Local\\RabiscoOverlay");
        if !perf && {
            CreateMutexW(null(), 1, name.as_ptr());
            GetLastError() == ERROR_ALREADY_EXISTS
        } {
            let class = wide("RabiscoOverlay");
            let other = FindWindowW(class.as_ptr(), null());
            if !other.is_null() && !start_hidden {
                PostMessageW(other, WM_SHOW_BAR, 0, 0);
            }
            return;
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        EnableMouseInPointer(1);
        let icc = INITCOMMONCONTROLSEX { dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_WIN95_CLASSES };
        InitCommonControlsEx(&icc);

        let hinst = GetModuleHandleW(null());
        let arrow = LoadCursorW(null_mut(), IDC_ARROW);
        let overlay_class = wide("RabiscoOverlay");
        let bar_class = wide("RabiscoBarra");
        let aux_class = wide("RabiscoAviso");
        let (dark_class, edge_class) = (wide(DARK_CLASS), wide(EDGE_CLASS));
        // As caixas sólidas pintam só o fundo da classe: nada de imagem na memória.
        let red = toolbar::REC;
        let classes: [(&Vec<u16>, WNDPROC, HBRUSH); 5] = [
            (&overlay_class, Some(overlay_proc), null_mut()),
            (&bar_class, Some(bar_proc), null_mut()),
            (&aux_class, Some(DefWindowProcW), null_mut()),
            (&dark_class, Some(DefWindowProcW), GetStockObject(BLACK_BRUSH) as HBRUSH),
            (&edge_class, Some(DefWindowProcW), CreateSolidBrush((red >> 16) | (red & 0xFF00) | (red & 0xFF) << 16)),
        ];
        for (class, proc_, brush) in classes {
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: proc_,
                hInstance: hinst,
                hCursor: arrow,
                hbrBackground: brush,
                lpszClassName: class.as_ptr(),
                ..core::mem::zeroed()
            };
            RegisterClassExW(&wc);
        }
        let title = wide("Rabisco");
        let (ox, oy, w, h) = virtual_screen();
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
            overlay_class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            ox,
            oy,
            w,
            h,
            null_mut(),
            null_mut(),
            hinst,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        // Dona = a janela da tela: a barra fica sempre por cima dela.
        let bar_y = (GetSystemMetrics(SM_CYSCREEN) - 400) / 2;
        let bar_hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            bar_class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            16,
            bar_y.max(0),
            80,
            400,
            hwnd,
            null_mut(),
            hinst,
            null(),
        );
        // Barra e aviso não entram no print; as guias entram (se estiverem visíveis).
        SetWindowDisplayAffinity(bar_hwnd, WDA_EXCLUDEFROMCAPTURE);
        // Guias e aviso: sem dono (esconder a janela da tela não pode escondê-los) e
        // com WS_EX_TRANSPARENT: o clique atravessa.
        let guide_hwnds = core::array::from_fn(|_| aux_window());
        let toast_hwnd = aux_window();
        SetWindowDisplayAffinity(toast_hwnd, WDA_EXCLUDEFROMCAPTURE);
        let tip = CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            null(),
            WS_POPUP | TTS_NOPREFIX | TTS_ALWAYSTIP,
            0,
            0,
            0,
            0,
            bar_hwnd,
            null_mut(),
            hinst,
            null(),
        );

        let boxed = Box::new(App {
            hwnd,
            surface: None,
            visible: false,
            ox,
            oy,
            canvas: Canvas::new(w, h),
            tool: Tool::Pen,
            rgb: COLORS[0],
            size: 4.0,
            draw: false,
            hidden: false,
            cursor: null_mut(),
            pointer: None,
            live: None,
            live_box: Rect::EMPTY,
            smooth_p: 1.0,
            erasing: None,
            undo: Vec::new(),
            redo: Vec::new(),
            samples: Vec::new(),
            scratch: Vec::new(),
            guides: guias::Mode::Off,
            guide_hwnds,
            toast_hwnd,
            clip_png: None,
            print: PrintQueue::default(),
            zoom_at: None,
            png_format: RegisterClipboardFormatW(wide("PNG").as_ptr()),
            bar: Bar {
                hwnd: bar_hwnd,
                layout: toolbar::layout(1.0),
                surface: None,
                hover: None,
                tip,
                tip_text: wide(""),
                tip_shown: false,
                shown: !start_hidden,
                faded: perf || instalar::pref("BarraTransparente").unwrap_or(1) != 0,
                near: false,
                rec_menu: false,
            },
            tray_icon: {
                let n = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem());
                let px: Vec<u32> = toolbar::app_icon(n).into_iter().map(instalar::unpremul).collect();
                argb_icon(&px, n, None)
            },
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            fade: false,
            text: None,
            look: None,
            spot_r: 0.0,
            zoom: 2.0,
            rec: gravar::State::Idle,
            rec_stop: null_mut(),
            rec_child: None,
            rec_part: None,
            rec_wait: null_mut(),
            rec_dot: null_mut(),
            rec_frame: [null_mut(); 4],
            rec_rect: (0, 0, 0, 0),
            rec_target: null_mut(),
            rec_area: (0, 0, 0, 0, 0),
            rec_t0: None,
            // O --perf-memoria não lê o registro: o padrão (monitor, som do sistema).
            rec_prefs: gravar::Prefs::from_bits(if perf { None } else { instalar::pref("Gravacao") }),
            pick: None,
        });
        APP = Box::into_raw(boxed);
        let a = app();
        let mut ti = a.tool_info();
        ti.lpszText = a.bar.tip_text.as_mut_ptr();
        SendMessageW(tip, TTM_ADDTOOLW, 0, &ti as *const _ as LPARAM);
        SendMessageW(tip, TTM_SETMAXTIPWIDTH, 0, 320);
        a.refresh_cursor();
        a.layout_bar();
        if !start_hidden {
            ShowWindow(bar_hwnd, SW_SHOWNOACTIVATE);
        }
        if perf {
            return perf_memoria(perf_file.as_deref().unwrap_or_default());
        }
        bandeja::add(hwnd, a.tray_icon);
        if flag("--boas-vindas") {
            bandeja::balloon(hwnd, "Rabisco instalado", "Ele abre com o Windows e fica aqui na bandeja. Clique no ícone para mostrar ou guardar a barra.");
        }

        let mods = MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT;
        let mut failed = Vec::new();
        for (id, vk, label) in [(HK_DRAW, 'D', "Ctrl+Shift+D"), (HK_HIDE, 'H', "Ctrl+Shift+H"), (HK_CLEAR, 'X', "Ctrl+Shift+X"), (HK_SAVE, 'S', "Ctrl+Shift+S"), (HK_REC, 'F', "Ctrl+Shift+F"), (HK_PRINT_REG, 'A', "Ctrl+Shift+A"), (HK_BAR, 'B', "Ctrl+Shift+B"), (HK_QUIT, 'Q', "Ctrl+Shift+Q")] {
            if RegisterHotKey(hwnd, id, mods, vk as u32) == 0 {
                failed.push(label);
            }
        }
        if !failed.is_empty() {
            let text = wide(&format!("Outro programa já usa: {}.\nEsses atalhos não vão funcionar.", failed.join(", ")));
            MessageBoxW(null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING);
        }

        let mut msg: MSG = core::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nome_do_arquivo_com_data_e_hora() {
        assert_eq!(file_name(2026, 11, 7, 9, 5, 3), "rabisco_2026-11-07_09h05m03.png");
        assert_eq!(file_name(2026, 12, 31, 23, 59, 59), "rabisco_2026-12-31_23h59m59.png");
    }

    #[test]
    fn grava_o_arquivo_datado_e_sobrescreve_o_ultimo() {
        let dir = std::env::temp_dir().join(format!("rabisco-teste-{}", std::process::id())).join("Pictures").join("Rabisco");
        assert!(write_files(&dir, "rabisco_a.png", b"primeiro"));
        assert!(write_files(&dir, "rabisco_b.png", b"segundo"));
        assert_eq!(std::fs::read(dir.join("rabisco_a.png")).unwrap(), b"primeiro");
        assert_eq!(std::fs::read(dir.join("rabisco_b.png")).unwrap(), b"segundo");
        assert_eq!(std::fs::read(dir.join("ultimo.png")).unwrap(), b"segundo");
        assert!(!dir.join("ultimo.png.tmp").exists(), "sobrou o temporário");
        std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap()).unwrap();
    }

    /// R-2: Ctrl+S durante um print fica na fila (um só), com a tela do aperto.
    #[test]
    fn print_durante_print_fica_na_fila() {
        let mut q = PrintQueue::<u8>::default();
        assert!(matches!(q.press(|| Some(1)), Press::Start));
        assert!(matches!(q.press(|| Some(2)), Press::Queued));
        assert!(matches!(q.press(|| Some(3)), Press::Full), "no máximo 1 pendente");
        assert_eq!(q.done(), Some(2), "o pendente é a tela do 2º aperto e já começa");
        assert!(matches!(q.press(|| Some(4)), Press::Queued));
        assert_eq!(q.done(), Some(4));
        assert_eq!(q.done(), None);
        assert!(matches!(q.press(|| Some(5)), Press::Start), "livre de novo");
        q.done();
        assert!(matches!(q.press(|| None), Press::Start));
        assert!(matches!(q.press(|| None), Press::Full), "captura falhou: não enfileira nada");
    }

    /// R-1: vários Ctrl+S no mesmo segundo têm o mesmo nome; nenhum pode sumir.
    #[test]
    fn prints_no_mesmo_segundo_nao_se_sobrescrevem() {
        let dir = std::env::temp_dir().join(format!("rabisco-mesmo-seg-{}", std::process::id()));
        for i in 0..4u8 {
            assert!(write_files(&dir, "rabisco_x.png", &[i]));
        }
        let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        assert_eq!(names, ["rabisco_x.png", "rabisco_x_2.png", "rabisco_x_3.png", "rabisco_x_4.png", "ultimo.png"]);
        assert_eq!(std::fs::read(dir.join("rabisco_x_3.png")).unwrap(), [2]);
        assert_eq!(std::fs::read(dir.join("ultimo.png")).unwrap(), [3]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dib_da_area_de_transferencia_vem_de_baixo_para_cima() {
        // 2x2: cima = vermelho, verde; baixo = azul, branco.
        let px = [0xFFFF0000u32, 0xFF00FF00, 0xFF0000FF, 0xFFFFFFFF];
        let mem = dib_from_png(&png::encode(2, 2, &px));
        assert!(!mem.is_null());
        unsafe {
            assert_eq!(GlobalSize(mem), size_of::<BITMAPINFOHEADER>() + 16);
            let p = GlobalLock(mem) as *const u8;
            let head = &*(p as *const BITMAPINFOHEADER);
            assert_eq!((head.biWidth, head.biHeight, head.biBitCount, head.biCompression), (2, 2, 32, BI_RGB as u32));
            let data = core::slice::from_raw_parts(p.add(size_of::<BITMAPINFOHEADER>()) as *const u32, 4);
            assert_eq!(data, [0xFF0000FF, 0xFFFFFFFF, 0xFFFF0000, 0xFF00FF00]);
            GlobalUnlock(mem);
            GlobalFree(mem);
        }
    }

    /// Captura de verdade: precisa de uma área de trabalho (o sandbox sem janelas pode falhar).
    /// `cargo test --release captura -- --ignored`
    #[test]
    #[ignore]
    fn captura_da_tela_codifica_e_decodifica() {
        let mut s = capture(0, 0, 320, 200).expect("BitBlt da tela");
        let png = png::encode(320, 200, s.pixels());
        let (w, h, back) = png::decode(&png).unwrap();
        assert_eq!((w, h), (320, 200));
        assert!(back.iter().zip(s.pixels().iter()).all(|(a, b)| a & 0xFFFFFF == b & 0xFFFFFF));
        println!("captura 320x200 -> PNG de {} bytes", png.len());
    }

    /// R-1 de ponta a ponta: 4 prints seguidos (mesmo segundo) = 4 arquivos.
    /// Muda o USERPROFILE do processo: rode sozinho (`captura -- --ignored`).
    #[test]
    #[ignore]
    fn captura_quatro_prints_seguidos_quatro_arquivos() {
        let home = std::env::temp_dir().join(format!("rabisco-quatro-{}", std::process::id()));
        unsafe { std::env::set_var("USERPROFILE", &home) };
        for _ in 0..4 {
            assert!(capture_and_save(0, 0, 320, 200).files);
        }
        let dir = home.join("Pictures").join("Rabisco");
        let n = std::fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("rabisco_")).count();
        std::fs::remove_dir_all(&home).unwrap();
        assert_eq!(n, 4);
    }
}
