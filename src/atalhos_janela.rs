//! Janela "Editar atalhos": um campo de atalho do Windows (msctls_hotkey32) por
//! ação. Enquanto ela está aberta os atalhos globais ficam soltos (senão o
//! RegisterHotKey pegaria a tecla antes do campo); ao fechar, o overlay recebe
//! WM_KEYS_EDITED e registra de novo a partir do arquivo.

use crate::atalhos::{self, ACTIONS, Key};
use crate::{WM_KEYS_EDITED, cursor_dpi, cursor_monitor, wide};
use core::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const NAMES: [&str; atalhos::N] = ["Desenhar", "Esconder desenhos", "Apagar tudo", "Salvar print", "Print de uma área", "Gravar vídeo", "Barra transparente", "Holofote", "Zoom", "Sair"];
const ID_FIELD: i32 = 100;
const ID_DEFAULTS: i32 = 300;

// Só a thread da interface mexe nisto.
static mut WIN: HWND = null_mut();
static mut OWNER: HWND = null_mut();
static mut FONT: HFONT = null_mut();
/// Atalho de cada linha ao abrir e o valor que o campo mostrou para ele: se o
/// campo não mudou, fica o original (o campo não sabe mostrar o Win).
static mut START: [(Option<Key>, usize); atalhos::N] = [(None, 0); atalhos::N];

/// A janela, se aberta (o laço de mensagens passa as teclas por ela).
pub fn hwnd() -> HWND {
    unsafe { WIN }
}

pub fn open(owner: HWND, icon: HICON) {
    unsafe {
        if !WIN.is_null() {
            SetForegroundWindow(WIN);
            return;
        }
        OWNER = owner;
        let class = wide("RabiscoAtalhos");
        let inst = GetModuleHandleW(null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc),
            hInstance: inst,
            hIcon: icon,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_BTNFACE + 1) as usize as HBRUSH,
            lpszClassName: class.as_ptr(),
            ..core::mem::zeroed()
        };
        RegisterClassExW(&wc); // a segunda vez falha e tudo bem

        let dpi = cursor_dpi();
        let px = |v: i32| v * dpi as i32 / 96;
        let (style, ex) = (WS_CAPTION | WS_SYSMENU, WS_EX_CONTROLPARENT);
        let mut r = RECT { left: 0, top: 0, right: px(380), bottom: px(418) };
        AdjustWindowRectExForDpi(&mut r, style, 0, ex, dpi);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let (mx, my, mw, mh) = cursor_monitor();
        let title = wide("Atalhos do Rabisco");
        WIN = CreateWindowExW(ex, class.as_ptr(), title.as_ptr(), style, mx + (mw - w) / 2, my + (mh - h) / 2, w, h, null_mut(), null_mut(), inst, null());
        if WIN.is_null() {
            return;
        }

        let mut ncm: NONCLIENTMETRICSW = core::mem::zeroed();
        ncm.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as _, 0, dpi);
        FONT = CreateFontIndirectW(&ncm.lfMessageFont);
        let add = |class: &str, text: &str, style: u32, ex: u32, id: i32, (x, y, w, h): (i32, i32, i32, i32)| {
            let (c, t) = (wide(class), wide(text));
            let hw = CreateWindowExW(ex, c.as_ptr(), t.as_ptr(), WS_CHILD | WS_VISIBLE | style, px(x), px(y), px(w), px(h), WIN, id as usize as HMENU, inst, null());
            SendMessageW(hw, WM_SETFONT, FONT as WPARAM, 0);
            hw
        };

        add("STATIC", "Clique num campo e aperte o atalho novo.\nBackspace deixa a ação sem atalho.", 0, 0, -1, (16, 12, 348, 34));
        let keys = atalhos::load().keys;
        for (i, name) in NAMES.iter().enumerate() {
            let y = 56 + i as i32 * 32;
            add("STATIC", name, 0x200 /* SS_CENTERIMAGE: texto no meio da altura */, 0, -1, (16, y, 150, 24));
            let f = add("msctls_hotkey32", "", WS_TABSTOP, WS_EX_CLIENTEDGE, ID_FIELD + i as i32, (170, y, 194, 24));
            START[i] = (keys[i], set_field(f, keys[i]));
        }
        add("BUTTON", "Padrões", WS_TABSTOP, 0, ID_DEFAULTS, (16, 378, 90, 28));
        add("BUTTON", "Cancelar", WS_TABSTOP, 0, IDCANCEL, (182, 378, 88, 28));
        add("BUTTON", "Salvar", WS_TABSTOP | BS_DEFPUSHBUTTON as u32, 0, IDOK, (276, 378, 88, 28));

        ShowWindow(WIN, SW_SHOW);
        SetForegroundWindow(WIN);
        SetFocus(GetDlgItem(WIN, ID_FIELD));
    }
}

/// Mostra o atalho no campo; devolve o valor do campo.
unsafe fn set_field(f: HWND, k: Option<Key>) -> usize {
    // RegisterHotKey: Alt=1 Ctrl=2 Shift=4; campo: Shift=1 Ctrl=2 Alt=4.
    let v = k.map_or(0, |k| {
        let m = k.mods;
        let f = (m & 4 != 0) as u32 * HOTKEYF_SHIFT | (m & 2 != 0) as u32 * HOTKEYF_CONTROL | (m & 1 != 0) as u32 * HOTKEYF_ALT;
        (k.vk | f << 8) as usize
    });
    unsafe { SendMessageW(f, HKM_SETHOTKEY, v, 0) };
    v
}

unsafe fn read_field(i: usize) -> Option<Key> {
    let v = unsafe { SendMessageW(GetDlgItem(WIN, ID_FIELD + i as i32), HKM_GETHOTKEY, 0, 0) } as usize & 0xFFFF;
    let (start, shown) = unsafe { START[i] };
    if v == shown {
        return start;
    }
    let (vk, f) = ((v & 0xFF) as u32, (v >> 8) as u32);
    (vk != 0).then(|| Key {
        mods: (f & HOTKEYF_SHIFT != 0) as u32 * 4 | (f & HOTKEYF_CONTROL != 0) as u32 * 2 | (f & HOTKEYF_ALT != 0) as u32,
        vk,
    })
}

/// Confere e grava; false mantém a janela aberta.
unsafe fn save() -> bool {
    let keys: [Option<Key>; atalhos::N] = core::array::from_fn(|i| unsafe { read_field(i) });
    let mut problems = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        let Some(k) = *k else { continue };
        if !atalhos::valid(k) {
            problems.push(format!("{}: use Ctrl, Alt ou Win com uma letra ou um número, ou F1 a F24.", NAMES[i]));
        } else if let Some(j) = keys[..i].iter().position(|o| *o == Some(k)) {
            problems.push(format!("{} e {} estão com o mesmo atalho ({}).", NAMES[j], NAMES[i], atalhos::label(Some(k))));
        }
    }
    let text = if !problems.is_empty() {
        problems.join("\n")
    } else if let Err(e) = atalhos::save(&keys) {
        format!("Não deu para gravar {}:\n{e}", atalhos::path().display())
    } else {
        return true;
    };
    let (text, title) = (wide(&text), wide("Rabisco"));
    unsafe { MessageBoxW(WIN, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING) };
    false
}

unsafe extern "system" fn proc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                match (w & 0xFFFF) as i32 {
                    IDOK => {
                        if save() {
                            DestroyWindow(h);
                        }
                    }
                    IDCANCEL => {
                        DestroyWindow(h);
                    }
                    ID_DEFAULTS => {
                        for (i, (_, def, _)) in ACTIONS.iter().enumerate() {
                            set_field(GetDlgItem(h, ID_FIELD + i as i32), atalhos::parse_key(def));
                            START[i].1 = usize::MAX; // daqui em diante vale o que o campo mostra
                        }
                    }
                    _ => {}
                }
                0
            }
            WM_DESTROY => {
                DeleteObject(FONT as HGDIOBJ);
                WIN = null_mut();
                PostMessageW(OWNER, WM_KEYS_EDITED, 0, 0);
                0
            }
            _ => DefWindowProcW(h, msg, w, l),
        }
    }
}
