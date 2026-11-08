//! Instalar, desinstalar e abrir com o Windows, sem instalador à parte: o
//! próprio `rabisco.exe` se copia para `%LOCALAPPDATA%\Programs\Rabisco`,
//! cria o atalho no Iniciar, a entrada em Configurações > Aplicativos e a
//! chave `Run` (abre na bandeja ao entrar no Windows). Tudo no usuário (HKCU),
//! sem pedir administrador.
//!
//! Vira instalador por `--instalar` ou quando o arquivo se chama `Instalar*.exe`.

use crate::toolbar;
use core::ffi::c_void;
use core::ptr::{null, null_mut};
use std::path::{Path, PathBuf};
use windows_sys::Win32::System::Com::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::{BOOL, GUID};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Rabisco";
const NAME: &str = "Rabisco";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn env_dir(var: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(var).unwrap_or_default())
}

pub fn install_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join("Programs").join(NAME)
}

fn shortcut_path() -> PathBuf {
    env_dir("APPDATA").join(r"Microsoft\Windows\Start Menu\Programs").join("Rabisco.lnk")
}

fn current_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_default()
}

/// Rodando como instalador? (`--instalar` ou nome `Instalar*.exe`)
pub fn is_installer(args: &[String]) -> bool {
    let stem = current_exe().file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    args.iter().any(|a| a == "--instalar") || stem.starts_with("instalar")
}

fn message(text: &str, icon: MESSAGEBOX_STYLE) {
    let (t, c) = (wide(text), wide(NAME));
    unsafe { MessageBoxW(null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | icon) };
}

fn set_string(key: &str, value: &str, data: &str) -> bool {
    let (k, v, d) = (wide(key), wide(value), wide(data));
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), REG_SZ, d.as_ptr() as *const c_void, (d.len() * 2) as u32) == 0 }
}

fn set_dword(key: &str, value: &str, data: u32) {
    let (k, v) = (wide(key), wide(value));
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), REG_DWORD, &data as *const u32 as *const c_void, 4) };
}

const PREFS_KEY: &str = r"Software\Rabisco";

/// Preferência do app (DWORD em HKCU\Software\Rabisco).
pub fn pref(value: &str) -> Option<u32> {
    let (k, v) = (wide(PREFS_KEY), wide(value));
    let (mut data, mut len) = (0u32, 4u32);
    let ok = unsafe { RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut data as *mut u32 as *mut c_void, &mut len) } == 0;
    ok.then_some(data)
}

pub fn set_pref(value: &str, data: u32) {
    set_dword(PREFS_KEY, value, data);
}

fn get_string(key: &str, value: &str) -> Option<String> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let ok = unsafe { RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr() as *mut c_void, &mut len) } == 0;
    ok.then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]))
}

fn delete_value(key: &str, value: &str) {
    let (k, v) = (wide(key), wide(value));
    unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr()) };
}

// ---------- abrir com o Windows ----------

pub fn autostart() -> bool {
    get_string(RUN_KEY, NAME).is_some()
}

/// Liga com o exe que está rodando agora (instalado ou não).
pub fn set_autostart(on: bool) -> bool {
    if on {
        set_string(RUN_KEY, NAME, &format!("\"{}\" --bandeja", current_exe().display()))
    } else {
        delete_value(RUN_KEY, NAME);
        true
    }
}

// ---------- instalar / desinstalar ----------

/// Fecha o Rabisco que estiver aberto e espera ele sair (para soltar o exe).
fn close_running() {
    let class = wide("RabiscoOverlay");
    for _ in 0..50 {
        let hwnd = unsafe { FindWindowW(class.as_ptr(), null()) };
        if hwnd.is_null() {
            return;
        }
        unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn copy_retry(from: &Path, to: &Path) -> std::io::Result<u64> {
    let mut last = None;
    for _ in 0..30 {
        match std::fs::copy(from, to) {
            Ok(n) => return Ok(n),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(last.unwrap())
}

pub fn install() {
    let dir = install_dir();
    let exe = dir.join("rabisco.exe");
    let ico = dir.join("rabisco.ico");
    close_running();
    let copied = std::fs::create_dir_all(&dir).and_then(|_| {
        if current_exe() == exe { Ok(0) } else { copy_retry(&current_exe(), &exe) }
    });
    if let Err(e) = copied {
        message(&format!("Não consegui copiar para\n{}\n\n{e}", dir.display()), MB_ICONERROR);
        return;
    }
    let _ = std::fs::write(&ico, ico_file(&[16, 24, 32, 48, 64, 256]));
    let _ = make_shortcut(&shortcut_path(), &exe, &ico);

    let quoted = format!("\"{}\"", exe.display());
    set_string(RUN_KEY, NAME, &format!("{quoted} --bandeja"));
    let size_kb = std::fs::metadata(&exe).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
    for (value, data) in [
        ("DisplayName", NAME),
        ("DisplayVersion", env!("CARGO_PKG_VERSION")),
        ("Publisher", NAME),
        ("DisplayIcon", &ico.display().to_string()),
        ("InstallLocation", &dir.display().to_string()),
        ("UninstallString", &format!("{quoted} --desinstalar")),
    ] {
        set_string(UNINSTALL_KEY, value, data);
    }
    set_dword(UNINSTALL_KEY, "NoModify", 1);
    set_dword(UNINSTALL_KEY, "NoRepair", 1);
    set_dword(UNINSTALL_KEY, "EstimatedSize", size_kb.max(1));

    // Abre já instalado, com a barra e um aviso na bandeja.
    let _ = std::process::Command::new(&exe).arg("--boas-vindas").spawn();
}

pub fn uninstall() {
    close_running();
    delete_value(RUN_KEY, NAME);
    let k = wide(UNINSTALL_KEY);
    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
    let _ = std::fs::remove_file(shortcut_path());
    message("O Rabisco foi removido.", MB_ICONINFORMATION);
    // O exe não apaga a própria pasta enquanto roda: um cmd apaga depois.
    let dir = install_dir();
    if current_exe().starts_with(&dir) {
        use std::os::windows::process::CommandExt;
        let cmd = format!("ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{}\"", dir.display());
        let _ = std::process::Command::new("cmd").args(["/c", &cmd]).creation_flags(0x0800_0000).spawn(); // CREATE_NO_WINDOW
    }
}

// ---------- atalho (.lnk) por COM, sem crate de COM ----------

const IID_ISHELLLINKW: GUID = GUID::from_u128(0x000214F9_0000_0000_C000_000000000046);
const IID_IPERSISTFILE: GUID = GUID::from_u128(0x0000010b_0000_0000_C000_000000000046);
const CLSID_SHELLLINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);

/// Chama o método `index` da vtable de `obj`.
unsafe fn vcall(obj: *mut c_void, index: usize) -> *const c_void {
    unsafe { *(*(obj as *const *const *const c_void)).add(index) }
}

fn make_shortcut(lnk: &Path, target: &Path, icon: &Path) -> Result<(), i32> {
    type SetStr = unsafe extern "system" fn(*mut c_void, *const u16) -> i32;
    type SetIcon = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;
    type Qi = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32;
    type Save = unsafe extern "system" fn(*mut c_void, *const u16, BOOL) -> i32;
    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    unsafe {
        CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        let mut link: *mut c_void = null_mut();
        let hr = CoCreateInstance(&CLSID_SHELLLINK, null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELLLINKW, &mut link);
        if hr < 0 {
            return Err(hr);
        }
        let (t, d, i) = (wide(&target.display().to_string()), wide("Anotar por cima da tela"), wide(&icon.display().to_string()));
        let wd = wide(&target.parent().unwrap_or(target).display().to_string());
        // IShellLinkW: 7 SetDescription, 9 SetWorkingDirectory, 17 SetIconLocation, 20 SetPath
        core::mem::transmute::<_, SetStr>(vcall(link, 20))(link, t.as_ptr());
        core::mem::transmute::<_, SetStr>(vcall(link, 7))(link, d.as_ptr());
        core::mem::transmute::<_, SetStr>(vcall(link, 9))(link, wd.as_ptr());
        core::mem::transmute::<_, SetIcon>(vcall(link, 17))(link, i.as_ptr(), 0);
        let mut file: *mut c_void = null_mut();
        let mut hr = core::mem::transmute::<_, Qi>(vcall(link, 0))(link, &IID_IPERSISTFILE, &mut file);
        if hr >= 0 {
            let p = wide(&lnk.display().to_string());
            hr = core::mem::transmute::<_, Save>(vcall(file, 6))(file, p.as_ptr(), 1); // IPersistFile::Save
            core::mem::transmute::<_, Release>(vcall(file, 2))(file);
        }
        core::mem::transmute::<_, Release>(vcall(link, 2))(link);
        if hr < 0 { Err(hr) } else { Ok(()) }
    }
}

// ---------- ícone ----------

/// Arquivo .ico com uma imagem 32 bits por tamanho (BMP dentro do ICO).
pub fn ico_file(sizes: &[i32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&n| {
            let px = toolbar::app_icon(n);
            let mut b = Vec::new();
            for v in [40u32, n as u32, (n * 2) as u32] {
                b.extend(v.to_le_bytes());
            }
            b.extend(1u16.to_le_bytes());
            b.extend(32u16.to_le_bytes());
            b.extend([0u8; 24]);
            for y in (0..n).rev() {
                for x in 0..n {
                    b.extend(unpremul(px[(y * n + x) as usize]).to_le_bytes());
                }
            }
            b.extend(vec![0u8; (((n + 31) / 32) * 4 * n) as usize]); // máscara AND vazia
            b
        })
        .collect();
    let mut out = Vec::new();
    out.extend([0u16, 1, sizes.len() as u16].iter().flat_map(|v| v.to_le_bytes()));
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&n, img) in sizes.iter().zip(&images) {
        let d = if n >= 256 { 0 } else { n as u8 };
        out.extend([d, d, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((img.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in &images {
        out.extend(img);
    }
    out
}

/// BGRA pré-multiplicado → ARGB normal (o que ícone espera).
pub fn unpremul(p: u32) -> u32 {
    let a = p >> 24;
    if a == 0 {
        return 0;
    }
    let ch = |shift: u32| (((p >> shift) & 0xFF) * 255 / a).min(255) << shift;
    (a << 24) | ch(16) | ch(8) | ch(0)
}
