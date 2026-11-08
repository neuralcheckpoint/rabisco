//! Gravação de verdade pelo `rabisco.exe --gravar-teste` (ignorados: precisam
//! do WGC, da GPU com encoder H.264 e do ffprobe/ffmpeg).
//! `cargo test --release --test gravar -- --ignored --test-threads=1 --nocapture`
//! Os de janela abrem por ~6 s uma janela animada do próprio teste na tela
//! (o WGC recusa janela fora da tela: E_INVALIDARG) e tocam um bipe curto
//! nela, para conferir movimento e sincronia de áudio e vídeo.

use std::path::{Path, PathBuf};
use std::process::Command;

/// ffprobe/ffmpeg do PATH; senão, o do winget nesta máquina.
fn ff(bin: &str) -> PathBuf {
    if Command::new(bin).arg("-version").output().is_ok() {
        return bin.into();
    }
    let home = std::env::var("LOCALAPPDATA").unwrap_or_default();
    Path::new(&home).join(r"Microsoft\WinGet\Packages\Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-9.0.2-full_build\bin").join(format!("{bin}.exe"))
}

struct Video {
    mp4: PathBuf,
    txt: String,
}

impl Video {
    fn v(&self, k: &str) -> f64 {
        self.txt.lines().find_map(|l| l.strip_prefix(&format!("{k} "))?.parse().ok()).unwrap_or(f64::NAN)
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.mp4);
    }
}

fn grava(modo: &str, segundos: f64, nome: &str) -> Video {
    grava_com(modo, segundos, nome, "-")
}

/// `som`: "-", "sis", "mic" ou "sis,mic" (o 5º argumento do --gravar-teste).
fn grava_com(modo: &str, segundos: f64, nome: &str, som: &str) -> Video {
    let mp4 = std::env::temp_dir().join(format!("rabisco-teste-{}-{nome}.mp4", std::process::id()));
    let txt_path = mp4.with_extension("mp4.txt");
    let st = Command::new(env!("CARGO_BIN_EXE_rabisco")).args(["--gravar-teste", modo, &(segundos as u64).to_string()]).arg(&mp4).args(["30", som]).status().unwrap();
    let txt = std::fs::read_to_string(&txt_path).unwrap_or_default();
    let _ = std::fs::remove_file(&txt_path);
    println!("{modo}: {}", txt.lines().filter(|l| !l.starts_with("modulos")).collect::<Vec<_>>().join(" | "));
    assert!(st.success(), "--gravar-teste {modo} falhou: {txt}");
    assert!(!mp4.with_extension("mp4.part").exists(), "sobrou o .part");
    Video { mp4, txt }
}

/// (codec, largura, altura, quadros, duração) pelo ffprobe.
fn sonda(mp4: &Path) -> (String, u32, u32, u32, f64) {
    let o = Command::new(ff("ffprobe"))
        .args(["-v", "error", "-select_streams", "v:0", "-count_frames", "-show_entries", "stream=codec_name,width,height,nb_read_frames:format=duration", "-of", "default=nw=1"])
        .arg(mp4)
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stdout).to_string();
    let k = |n: &str| s.lines().find_map(|l| l.strip_prefix(&format!("{n}="))).unwrap_or("").to_string();
    (k("codec_name"), k("width").parse().unwrap_or(0), k("height").parse().unwrap_or(0), k("nb_read_frames").parse().unwrap_or(0), k("duration").parse().unwrap_or(0.0))
}

/// O ponto de "gravando" (230, 30, 30) aparece no quadro de 2 s? Olha o centro
/// dele contra um anel em volta: o fundo pode ser vermelho (o papel de parede
/// do Rafa tem uma faixa vermelha nesse canto), mas aí o anel também é.
fn ponto_no_video(v: &Video, w: u32) -> bool {
    let (d, m) = (v.v("ponto_d") as i32, v.v("ponto_m") as i32);
    let (cx, cy, r) = (w as i32 - m - d / 2, m + d / 2, d);
    let s = 2 * r; // par: no yuv420 o ffmpeg arredonda o recorte ímpar
    let crop = format!("crop={s}:{s}:{}:{}", cx - r, (cy - r).max(0));
    let o = Command::new(ff("ffmpeg")).args(["-v", "error", "-ss", "2", "-i"]).arg(&v.mp4).args(["-frames:v", "1", "-vf", &crop, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]).output().unwrap();
    assert_eq!(o.stdout.len(), (s * s * 3) as usize, "ffmpeg não tirou o quadro");
    let oy = cy - (cy - r).max(0);
    let media = |dentro: &dyn Fn(f32) -> bool| {
        let (mut soma, mut n) = ([0f32; 3], 0f32);
        for (i, p) in o.stdout.chunks(3).enumerate() {
            let (x, y) = ((i as i32 % s - r) as f32, (i as i32 / s - oy) as f32);
            if dentro((x * x + y * y).sqrt()) {
                (0..3).for_each(|c| soma[c] += p[c] as f32);
                n += 1.0;
            }
        }
        soma.map(|c| c / n.max(1.0))
    };
    let dist = |a: [f32; 3], b: [f32; 3]| ((0..3).map(|c| (a[c] - b[c]).powi(2)).sum::<f32>()).sqrt();
    let centro = media(&|q| q <= d as f32 / 4.0);
    let anel = media(&|q| q >= d as f32 * 0.8 && q <= d as f32);
    println!("ponto: centro {centro:?} anel {anel:?}");
    dist(centro, [230.0, 30.0, 30.0]) < 60.0 && dist(centro, anel) > 60.0
}

fn confere(v: &Video, segundos: f64) {
    let (codec, w, h, quadros, dur) = sonda(&v.mp4);
    println!("ffprobe: {codec} {w}x{h} {quadros} quadros {dur:.3} s");
    assert_eq!(codec, "h264");
    assert_eq!((w as f64, h as f64), (v.v("w"), v.v("h")), "dimensões");
    assert!(w % 2 == 0 && h % 2 == 0);
    assert!((dur - segundos).abs() <= 0.2, "duração {dur}");
    let esperado = segundos * 30.0;
    assert!(quadros as f64 >= 0.95 * esperado, "{quadros} quadros de {esperado}");
    // O ponto vermelho tem WDA_EXCLUDEFROMCAPTURE: está na tela, não no vídeo.
    assert!(!ponto_no_video(v, w), "o ponto de gravando apareceu no vídeo");
}

#[test]
#[ignore]
fn grava_5s_do_monitor() {
    let v = grava("monitor", 5.0, "monitor");
    confere(&v, 5.0);
}

#[test]
#[ignore]
fn grava_5s_de_regiao() {
    let v = grava("regiao:100,100,1281,721", 5.0, "regiao");
    assert_eq!((v.v("w"), v.v("h")), (1280.0, 720.0), "região par");
    confere(&v, 5.0);
}

// ---------- janela animada, bipe e sincronia ----------

mod janela {
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub const W: i32 = 640;
    pub const H: i32 = 360;
    /// A cada 1,5 s a janela fica branca por 200 ms e toca 200 ms de 1 kHz, na mesma hora.
    pub const PERIODO_MS: u128 = 1500;
    static QUADRO: AtomicU32 = AtomicU32::new(0);
    static BRANCO: AtomicBool = AtomicBool::new(false);
    static HWND_: AtomicIsize = AtomicIsize::new(0);
    static mut INICIO: Option<Instant> = None;
    static mut WAV: Vec<u8> = Vec::new();

    /// WAV PCM 16 bits mono 48 kHz: 200 ms de 1 kHz a meia escala.
    fn bipe() -> Vec<u8> {
        let n = 48_000 / 5;
        let dados: Vec<u8> = (0..n).flat_map(|i| (((i as f32 * 1000.0 * std::f32::consts::TAU / 48_000.0).sin() * 16_000.0) as i16).to_le_bytes()).collect();
        let mut w = Vec::new();
        let u32le = |v: u32| v.to_le_bytes();
        w.extend(b"RIFF");
        w.extend(u32le(36 + dados.len() as u32));
        w.extend(b"WAVEfmt ");
        w.extend(u32le(16));
        w.extend([1, 0, 1, 0]);
        w.extend(u32le(48_000));
        w.extend(u32le(96_000));
        w.extend([2, 0, 16, 0]);
        w.extend(b"data");
        w.extend(u32le(dados.len() as u32));
        w.extend(dados);
        w
    }

    unsafe extern "system" fn proc_(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            match m {
                WM_TIMER => {
                    let ms = (*core::ptr::addr_of!(INICIO)).map_or(0, |t| t.elapsed().as_millis());
                    let branco = ms % PERIODO_MS >= PERIODO_MS - 200;
                    if branco && !BRANCO.swap(true, Ordering::Relaxed) {
                        PlaySoundW((*core::ptr::addr_of!(WAV)).as_ptr() as _, core::ptr::null_mut(), SND_ASYNC | SND_MEMORY);
                    } else if !branco {
                        BRANCO.store(false, Ordering::Relaxed);
                    }
                    QUADRO.fetch_add(1, Ordering::Relaxed);
                    InvalidateRect(h, core::ptr::null(), 0);
                    0
                }
                WM_PAINT => {
                    let mut ps: PAINTSTRUCT = core::mem::zeroed();
                    let dc = BeginPaint(h, &mut ps);
                    let n = QUADRO.load(Ordering::Relaxed);
                    let fundo = if BRANCO.load(Ordering::Relaxed) { 0xFFFFFF } else { 0x302820 };
                    let b = CreateSolidBrush(fundo);
                    FillRect(dc, &RECT { left: 0, top: 0, right: W, bottom: H }, b);
                    DeleteObject(b);
                    // Uma barra que anda e o número do quadro: nenhum quadro igual ao outro.
                    let x = (n as i32 * 9) % (W - 60);
                    let b = CreateSolidBrush(0x30C0F0);
                    FillRect(dc, &RECT { left: x, top: 100, right: x + 60, bottom: 260 }, b);
                    DeleteObject(b);
                    let t: Vec<u16> = format!("quadro {n}").encode_utf16().collect();
                    SetBkMode(dc, 1);
                    SetTextColor(dc, 0xE0E0E0);
                    TextOutW(dc, 20 + (n as i32 % 50), 20, t.as_ptr(), t.len() as i32);
                    EndPaint(h, &ps);
                    0
                }
                WM_DESTROY => {
                    PostQuitMessage(0);
                    0
                }
                _ => DefWindowProcW(h, m, wp, lp),
            }
        }
    }

    pub struct Janela {
        thread: Option<std::thread::JoinHandle<()>>,
        pub hwnd: isize,
        pub x: i32,
        pub y: i32,
    }

    /// Abre a janela (sem foco, por cima) em (x, y) da tela, numa thread com o próprio laço.
    pub fn abre(x: i32, y: i32) -> Janela {
        let thread = std::thread::spawn(move || unsafe {
            // Pixels físicos, como o --gravar-teste: a região bate com a janela.
            windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            WAV = bipe();
            INICIO = Some(Instant::now());
            let classe: Vec<u16> = "RabiscoTesteJanela\0".encode_utf16().collect();
            let wc = WNDCLASSW { lpfnWndProc: Some(proc_), hInstance: GetModuleHandleW(core::ptr::null()), lpszClassName: classe.as_ptr(), ..core::mem::zeroed() };
            RegisterClassW(&wc);
            let titulo: Vec<u16> = "Rabisco teste\0".encode_utf16().collect();
            let h = CreateWindowExW(WS_EX_TOPMOST, classe.as_ptr(), titulo.as_ptr(), WS_POPUP, x, y, W, H, core::ptr::null_mut(), core::ptr::null_mut(), wc.hInstance, core::ptr::null());
            // Mostra sem ativar (sem roubar o foco). Com WS_EX_NOACTIVATE o WGC recusa a janela (E_INVALIDARG).
            ShowWindow(h, SW_SHOWNA);
            // Abre o dispositivo do PlaySound antes de gravar (o primeiro toque atrasava ~90 ms): um bipe mudo.
            let wav = &*core::ptr::addr_of!(WAV);
            let mudo: Vec<u8> = wav.iter().take(44).copied().chain(std::iter::repeat_n(0, wav.len() - 44)).collect();
            PlaySoundW(mudo.as_ptr() as _, core::ptr::null_mut(), SND_MEMORY);
            HWND_.store(h as isize, Ordering::Relaxed);
            SetTimer(h, 1, 10, None);
            let mut m: MSG = core::mem::zeroed();
            while GetMessageW(&mut m, core::ptr::null_mut(), 0, 0) > 0 {
                DispatchMessageW(&m);
            }
        });
        while HWND_.load(Ordering::Relaxed) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // A primeira pintura antes de gravar.
        std::thread::sleep(std::time::Duration::from_millis(300));
        Janela { thread: Some(thread), hwnd: HWND_.load(Ordering::Relaxed), x, y }
    }

    impl Drop for Janela {
        fn drop(&mut self) {
            unsafe {
                PlaySoundW(core::ptr::null(), core::ptr::null_mut(), 0);
                PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0);
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
            HWND_.store(0, Ordering::Relaxed);
        }
    }
}

/// Lado da imagem reduzida (cinza) para comparar quadros.
const MW: usize = 160;
const MH: usize = 90;

/// Quadros do vídeo em cinza 160x90.
fn quadros_cinza(mp4: &Path) -> Vec<Vec<u8>> {
    let o = Command::new(ff("ffmpeg")).args(["-v", "error", "-i"]).arg(mp4).args(["-vf", "scale=160:90,format=gray", "-f", "rawvideo", "-"]).output().unwrap();
    o.stdout.chunks_exact(MW * MH).map(|c| c.to_vec()).collect()
}

/// Áudio do MP4 em mono f32 48 kHz.
fn audio_mono(mp4: &Path) -> Vec<f32> {
    let o = Command::new(ff("ffmpeg")).args(["-v", "error", "-i"]).arg(mp4).args(["-map", "0:a", "-ac", "1", "-ar", "48000", "-f", "f32le", "-"]).output().unwrap();
    o.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

/// (codec, duração) de cada faixa pelo ffprobe.
fn faixas(mp4: &Path) -> Vec<(String, f64)> {
    let o = Command::new(ff("ffprobe")).args(["-v", "error", "-show_entries", "stream=codec_name,duration", "-of", "csv=p=0"]).arg(mp4).output().unwrap();
    String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| {
        let (c, d) = l.split_once(',')?;
        Some((c.to_string(), d.trim().parse().ok()?))
    }).collect()
}

/// Começos (s) dos trechos brancos no vídeo e dos bipes no áudio.
fn flashes_e_bipes(q: &[Vec<u8>], a: &[f32]) -> (Vec<f64>, Vec<f64>) {
    let media = |f: &Vec<u8>| f.iter().map(|&v| v as f64).sum::<f64>() / f.len() as f64;
    let brancos: Vec<bool> = q.iter().map(|f| media(f) > 180.0).collect();
    let v = (1..q.len()).filter(|&i| brancos[i] && !brancos[i - 1]).map(|i| i as f64 / 30.0).collect();
    // Janelas de 5 ms: o bipe tem RMS ~0,35; o resto, o que o sistema tocar.
    let rms: Vec<f32> = a.chunks(240).map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt()).collect();
    let b = (1..rms.len()).filter(|&i| rms[i] > 0.1 && rms[i - 1] <= 0.1 && rms[i.saturating_sub(20)..i].iter().all(|&r| r <= 0.1)).map(|i| i as f64 * 0.005).collect();
    (v, b)
}

/// Grava 5 s com som do sistema e confere faixas, movimento e sincronia.
fn confere_com_som(v: &Video) {
    let f = faixas(&v.mp4);
    println!("faixas: {f:?}");
    let dur = |c: &str| f.iter().find(|x| x.0 == c).map(|x| x.1).unwrap_or(f64::NAN);
    let (dv, da) = (dur("h264"), dur("aac"));
    assert!((dv - 5.0).abs() <= 0.2, "vídeo {dv} s");
    assert!((dv - da).abs() <= 0.1, "vídeo {dv} s, áudio {da} s");
    let (_, _, _, n, _) = sonda(&v.mp4);
    assert!(n as f64 >= 0.95 * 150.0, "{n} quadros");
    // Movimento: quadro igual ao anterior (diferença média < 0,3 de 255) é repetido.
    let q = quadros_cinza(&v.mp4);
    let iguais = q.windows(2).filter(|p| p[0].iter().zip(&p[1]).map(|(a, b)| a.abs_diff(*b) as u32).sum::<u32>() as f64 / (MW * MH) as f64 <= 0.3).count();
    let pct = iguais as f64 / (q.len() - 1) as f64 * 100.0;
    println!("movimento: {iguais} de {} quadros iguais ao anterior ({pct:.1}%), {} repetidos pelo motor", q.len() - 1, v.v("repetidos"));
    assert!(pct <= 5.0, "{pct:.1}% de quadros repetidos com a janela animada");
    assert!(v.v("repetidos") <= 0.05 * v.v("quadros"), "o motor repetiu {} quadros", v.v("repetidos"));
    // Sincronia: o bipe tocou junto com o branco; o áudio do MP4 tem que ter o bipe na mesma hora.
    let (brancos, bipes) = flashes_e_bipes(&q, &audio_mono(&v.mp4));
    println!("brancos {brancos:?} bipes {bipes:?}");
    let pares: Vec<f64> = brancos.iter().filter_map(|&b| bipes.iter().map(|&a| a - b).filter(|d| d.abs() < 0.5).min_by(|x, y| x.abs().total_cmp(&y.abs()))).collect();
    println!("sincronia (áudio - vídeo): {:?} ms", pares.iter().map(|d| (d * 1000.0).round()).collect::<Vec<_>>());
    assert!(!pares.is_empty(), "nenhum bipe junto de um branco (sem som no sistema?)");
    assert!(pares.iter().all(|d| d.abs() <= 0.1), "áudio fora de sincronia: {pares:?}");
}

/// Canto de cima à esquerda do monitor principal, com folga.
fn canto() -> (i32, i32) {
    (200, 200)
}

#[test]
#[ignore]
fn grava_5s_de_janela_animada_com_som() {
    let (x, y) = canto();
    let j = janela::abre(x, y);
    let v = grava_com(&format!("janela:{}", j.hwnd), 5.0, "janela", "sis");
    drop(j);
    assert_eq!((v.v("w"), v.v("h")), (janela::W as f64, janela::H as f64), "tamanho da janela");
    confere_com_som(&v);
}

#[test]
#[ignore]
fn grava_5s_de_regiao_animada_com_som() {
    let (x, y) = canto();
    let j = janela::abre(x, y);
    let v = grava_com(&format!("regiao:{},{},{},{}", j.x, j.y, janela::W, janela::H), 5.0, "regiao-som", "sis");
    drop(j);
    confere_com_som(&v);
}

#[test]
#[ignore]
fn janela_fechada_termina_o_video() {
    let (x, y) = canto();
    let j = janela::abre(x, y);
    let hwnd = j.hwnd;
    // Fecha a janela 2 s depois de começar; o limite do teste é 10 s.
    let fecha = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        drop(j);
    });
    let t = std::time::Instant::now();
    let v = grava(&format!("janela:{hwnd}"), 10.0, "janela-fecha");
    let _ = fecha.join();
    let (_, _, _, quadros, dur) = sonda(&v.mp4);
    println!("ffprobe: {quadros} quadros {dur:.3} s, em {:?}", t.elapsed());
    assert!(t.elapsed() < std::time::Duration::from_secs(7), "seguiu gravando a janela fechada");
    assert!((1.0..4.0).contains(&dur) && quadros > 20, "duração {dur}, {quadros} quadros");
}
