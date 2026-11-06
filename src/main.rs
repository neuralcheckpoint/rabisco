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
mod instalar;
mod toolbar;
#[cfg(test)]
mod imagens;

use canvas::{Canvas, FLOOR, Rect, Stroke, Tool};
use core::ptr::{null, null_mut};
use toolbar::{Item, Layout, SIZES};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
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

/// Outra cópia foi aberta (atalho do Iniciar): mostra a barra desta.
const WM_SHOW_BAR: u32 = WM_APP + 2;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

const MENU_BAR: u32 = 1;
const MENU_DRAW: u32 = 2;
const MENU_HIDE: u32 = 3;
const MENU_CLEAR: u32 = 4;
const MENU_AUTOSTART: u32 = 5;
const MENU_QUIT: u32 = 6;

const TIP_TIMER: usize = 1;
/// Parado no modo desenho por este tempo: solta a imagem da tela.
const IDLE_TIMER: usize = 2;
const IDLE_MS: u32 = 8000;

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
        let dst = pos.map(|(x, y)| POINT { x, y });
        let size = SIZE { cx: self.w, cy: self.h };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
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
    tray_icon: HICON,
    /// `TaskbarCreated`: o Explorer reiniciou e o ícone da bandeja sumiu.
    taskbar_created: u32,
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
        let want = !self.hidden && (self.draw || !self.canvas.strokes.is_empty());
        if want != self.visible {
            self.visible = want;
            unsafe { ShowWindow(self.hwnd, if want { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
        }
        if !self.draw {
            self.surface = None; // o Windows já tem o último quadro
        }
    }

    /// Algo mudou na lista de traços: redesenha a área (ou tudo) e manda.
    fn redraw(&mut self, area: Option<Rect>) {
        let fresh = self.surface.is_none();
        if !self.hidden && (self.draw || !self.canvas.strokes.is_empty()) && self.ensure_surface() {
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
        if self.pointer.is_none() && self.live.is_none() && self.erasing.is_none() {
            self.surface = None;
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
        if !hidden && !self.visible && !self.canvas.strokes.is_empty() {
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
        let (ox, oy, w, h) = virtual_screen();
        let (dx, dy) = ((self.ox - ox) as f32, (self.oy - oy) as f32);
        let mut strokes = std::mem::take(&mut self.canvas.strokes);
        for p in strokes.iter_mut().flat_map(|s| s.pts.iter_mut()) {
            p[0] += dx;
            p[1] += dy;
        }
        (self.ox, self.oy) = (ox, oy);
        self.canvas = Canvas::new(w, h);
        self.canvas.strokes = strokes;
        self.surface = None;
        unsafe { SetWindowPos(self.hwnd, null_mut(), ox, oy, w, h, SWP_NOACTIVATE | SWP_NOZORDER) };
        self.redraw_all();
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
    }

    fn set_size(&mut self, size: f32) {
        self.size = size.clamp(1.0, 200.0);
        self.refresh_cursor();
        self.paint_bar();
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
        self.finish();
        if !self.ensure_surface() {
            return;
        }
        self.read_samples(id);
        let Some(s) = self.samples.pop() else { return };
        self.pointer = Some(id);
        if self.tool == Tool::Eraser || s.eraser {
            self.erasing = Some(Vec::new());
            self.erase_at(s.x, s.y);
            return;
        }
        let tool = self.tool;
        self.smooth_p = s.pressure;
        let pt = [s.x, s.y, self.radius(tool, s.pressure)];
        let alpha = if tool == Tool::Marker { MARKER_ALPHA } else { 255 };
        let pts = if tool.is_freehand() { vec![pt] } else { vec![pt, pt] };
        let live = Stroke { tool, rgb: self.rgb, alpha, pts };
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
            let (mut x, mut y) = (s.x, s.y);
            if unsafe { GetKeyState(VK_SHIFT as i32) } < 0 {
                (x, y) = constrain(live.tool, a[0], a[1], x, y);
            }
            live.pts[1] = [x, y, a[2]];
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
        let (a, b) = (live.pts[0], live.pts[live.pts.len() - 1]);
        if !live.tool.is_freehand() && (b[0] - a[0]).hypot(b[1] - a[1]) < 2.0 {
            // Clique sem arrastar numa forma: devolve o que estava embaixo.
            if let Some(px) = self.surface.as_mut().map(|s| s.pixels()) {
                let r = self.canvas.cancel_live(px);
                self.present(r);
            }
        } else {
            self.canvas.end_live();
            self.canvas.strokes.push(live.clone());
            self.push_action(Action::Add(live));
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
        if self.canvas.strokes.is_empty() {
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
            HK_QUIT => unsafe {
                DestroyWindow(self.hwnd);
            },
            _ => {}
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
            hover: self.bar.hover,
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            colors: &COLORS,
        };
        let bar = &mut self.bar;
        let Some(s) = bar.surface.as_mut() else { return };
        toolbar::paint(&bar.layout, &st, s.pixels());
        s.present(bar.hwnd, None, None);
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
                let id = bandeja::menu(self.hwnd, &[
                    It { id: MENU_BAR, text: if self.bar.shown { "Guardar a barra na bandeja" } else { "Mostrar a barra" }, checked: false },
                    It { id: MENU_DRAW, text: "Desenhar\tCtrl+Shift+D", checked: self.draw },
                    It { id: MENU_HIDE, text: "Esconder desenhos\tCtrl+Shift+H", checked: self.hidden },
                    It { id: MENU_CLEAR, text: "Apagar tudo\tCtrl+Shift+X", checked: false },
                    Sep,
                    It { id: MENU_AUTOSTART, text: "Abrir com o Windows", checked: auto },
                    Sep,
                    It { id: MENU_QUIT, text: "Sair\tCtrl+Shift+Q", checked: false },
                ]);
                match id {
                    MENU_BAR => self.set_bar(!self.bar.shown),
                    MENU_DRAW => self.set_draw(!self.draw),
                    MENU_HIDE => self.set_hidden(!self.hidden),
                    MENU_CLEAR => self.clear_all(),
                    MENU_AUTOSTART => {
                        instalar::set_autostart(!auto);
                    }
                    MENU_QUIT => unsafe {
                        DestroyWindow(self.hwnd);
                    },
                    _ => {}
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
                a.pointer_move(pointer_id(wp));
                0
            }
            WM_POINTERUP | WM_POINTERCAPTURECHANGED => {
                a.pointer_up(pointer_id(wp));
                0
            }
            WM_POINTERWHEEL | WM_MOUSEWHEEL => {
                a.resize_pen(((wp >> 16) as u16 as i16).signum() as i32);
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
            WM_HOTKEY => {
                a.hotkey(wp as i32);
                0
            }
            WM_TIMER if wp == IDLE_TIMER => {
                a.idle();
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
                let item = item_at(a, pointer_id(wp));
                a.bar_hover(item);
                0
            }
            WM_POINTERDOWN => {
                if (wp >> 16) as u32 & POINTER_FLAG_FIRSTBUTTON != 0 {
                    if let Some(item) = item_at(a, pointer_id(wp)) {
                        a.bar_click(item);
                    }
                }
                0
            }
            WM_POINTERLEAVE => {
                a.bar_hover(None);
                0
            }
            WM_NCPOINTERUPDATE => {
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
    unsafe {
        // Uma instância só. A segunda mostra a barra da primeira e sai.
        let name = wide("Local\\RabiscoOverlay");
        CreateMutexW(null(), 1, name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
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
        let classes: [(&Vec<u16>, WNDPROC); 2] = [(&overlay_class, Some(overlay_proc)), (&bar_class, Some(bar_proc))];
        for (class, proc_) in classes {
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: proc_,
                hInstance: hinst,
                hCursor: arrow,
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
        let bar_y = (GetSystemMetrics(SM_CYSCREEN) - 340) / 2;
        let bar_hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            bar_class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            16,
            bar_y.max(0),
            80,
            340,
            hwnd,
            null_mut(),
            hinst,
            null(),
        );
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
            bar: Bar {
                hwnd: bar_hwnd,
                layout: toolbar::layout(1.0),
                surface: None,
                hover: None,
                tip,
                tip_text: wide(""),
                tip_shown: false,
                shown: !start_hidden,
            },
            tray_icon: {
                let n = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem());
                let px: Vec<u32> = toolbar::app_icon(n).into_iter().map(instalar::unpremul).collect();
                argb_icon(&px, n, None)
            },
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
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
        bandeja::add(hwnd, a.tray_icon);
        if flag("--boas-vindas") {
            bandeja::balloon(hwnd, "Rabisco instalado", "Ele abre com o Windows e fica aqui na bandeja. Clique no ícone para mostrar ou guardar a barra.");
        }

        let mods = MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT;
        let mut failed = Vec::new();
        for (id, vk, label) in [(HK_DRAW, 'D', "Ctrl+Shift+D"), (HK_HIDE, 'H', "Ctrl+Shift+H"), (HK_CLEAR, 'X', "Ctrl+Shift+X"), (HK_QUIT, 'Q', "Ctrl+Shift+Q")] {
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
