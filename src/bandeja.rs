//! Ícone na bandeja (área de notificação): clique mostra/guarda a barra,
//! botão direito abre o menu. A mensagem `TaskbarCreated` avisa que o
//! Explorer reiniciou e o ícone precisa voltar.

use core::ptr::null_mut;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Mensagem que a bandeja manda para a janela da tela.
pub const WM_TRAY: u32 = WM_APP + 1;

fn copy(dst: &mut [u16], s: &str) {
    let max = dst.len() - 1;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16().take(max).chain(Some(0))) {
        *d = c;
    }
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut d: NOTIFYICONDATAW = unsafe { core::mem::zeroed() };
    d.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = 1;
    d
}

pub fn add(hwnd: HWND, icon: HICON) {
    let mut d = data(hwnd);
    d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    d.uCallbackMessage = WM_TRAY;
    d.hIcon = icon;
    copy(&mut d.szTip, "Rabisco · clique: barra · direito: menu");
    unsafe {
        Shell_NotifyIconW(NIM_ADD, &d);
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        Shell_NotifyIconW(NIM_SETVERSION, &d);
    }
}

pub fn remove(hwnd: HWND) {
    let d = data(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &d) };
}

pub fn balloon(hwnd: HWND, title: &str, text: &str) {
    let mut d = data(hwnd);
    d.uFlags = NIF_INFO;
    d.dwInfoFlags = NIIF_INFO;
    copy(&mut d.szInfoTitle, title);
    copy(&mut d.szInfo, text);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
}

pub enum Entry {
    Item { id: u32, text: String, checked: bool },
    Sep,
}

/// Menu no cursor; devolve o id escolhido (0 = nada).
pub fn menu(hwnd: HWND, entries: &[Entry]) -> u32 {
    unsafe {
        let m = CreatePopupMenu();
        for e in entries {
            match e {
                Entry::Item { id, text, checked } => {
                    let t: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
                    AppendMenuW(m, MF_STRING | if *checked { MF_CHECKED } else { 0 }, *id as usize, t.as_ptr());
                }
                Entry::Sep => {
                    AppendMenuW(m, MF_SEPARATOR, 0, core::ptr::null());
                }
            }
        }
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Sem isso o menu não fecha ao clicar fora.
        SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, null_mut());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);
        id as u32
    }
}
