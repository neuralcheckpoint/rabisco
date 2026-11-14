//! A parte do Windows da gravação: Windows.Graphics.Capture na GPU (D3D11) e o
//! H.264 do Media Foundation no encoder de hardware, sem os quadros passarem
//! pela RAM. d3d11, mfplat e mfreadwrite vêm por LoadLibrary nesta thread e
//! saem no fim: o app parado não carrega nada disso.

use crate::gravar::{Mode, RATE, Stats, Timeline, Track, audio_index, audio_time, bitrate, even, mix, region_in_monitor};
use crate::gravar_som::Source;
use core::ffi::c_void;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, HWND};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{GUID, HRESULT, IInspectable, Interface, Result};
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, FreeLibrary, HMODULE, POINT};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
use windows_sys::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, GetCurrentProcess, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST, GetProcessTimes, INFINITE, SetWaitableTimer, WaitForSingleObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect, IsWindow};

/// Texturas na fila do encoder: o quadro novo nunca é escrito numa que ele ainda lê.
const SLOTS: usize = 4;

type Out = *mut *mut c_void;

struct Lib(HMODULE);

impl Lib {
    fn open(name: &str) -> Result<Lib> {
        let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let h = unsafe { LoadLibraryW(w.as_ptr()) };
        if h.is_null() { Err(E_FAIL.into()) } else { Ok(Lib(h)) }
    }

    /// Ponteiro da função `name` (terminado em \0), no tipo `T`.
    fn get<T: Copy>(&self, name: &[u8]) -> Result<T> {
        let f = unsafe { GetProcAddress(self.0, name.as_ptr()) }.ok_or(windows::core::Error::from(E_FAIL))?;
        Ok(unsafe { core::mem::transmute_copy(&f) })
    }
}

impl Drop for Lib {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.0) };
    }
}

/// As funções das DLLs carregadas sob demanda. O Drop chama o MFShutdown antes
/// de soltar as DLLs (os campos caem depois do `drop`).
struct Api {
    create_device: unsafe extern "system" fn(*mut c_void, i32, *mut c_void, u32, *const i32, u32, u32, Out, *mut i32, Out) -> HRESULT,
    winrt_device: unsafe extern "system" fn(*mut c_void, Out) -> HRESULT,
    shutdown: unsafe extern "system" fn() -> HRESULT,
    attributes: unsafe extern "system" fn(Out, u32) -> HRESULT,
    media_type: unsafe extern "system" fn(Out) -> HRESULT,
    sample: unsafe extern "system" fn(Out) -> HRESULT,
    memory_buffer: unsafe extern "system" fn(u32, Out) -> HRESULT,
    surface_buffer: unsafe extern "system" fn(*const GUID, *mut c_void, u32, i32, Out) -> HRESULT,
    device_manager: unsafe extern "system" fn(*mut u32, Out) -> HRESULT,
    sink_writer: unsafe extern "system" fn(*const u16, *mut c_void, *mut c_void, Out) -> HRESULT,
    _libs: [Lib; 3],
}

impl Api {
    fn load() -> Result<Api> {
        let d3d = Lib::open("d3d11.dll")?;
        let plat = Lib::open("mfplat.dll")?;
        let rw = Lib::open("mfreadwrite.dll")?;
        let startup: unsafe extern "system" fn(u32, u32) -> HRESULT = plat.get(b"MFStartup\0")?;
        let api = Api {
            create_device: d3d.get(b"D3D11CreateDevice\0")?,
            winrt_device: d3d.get(b"CreateDirect3D11DeviceFromDXGIDevice\0")?,
            shutdown: plat.get(b"MFShutdown\0")?,
            attributes: plat.get(b"MFCreateAttributes\0")?,
            media_type: plat.get(b"MFCreateMediaType\0")?,
            sample: plat.get(b"MFCreateSample\0")?,
            memory_buffer: plat.get(b"MFCreateMemoryBuffer\0")?,
            surface_buffer: plat.get(b"MFCreateDXGISurfaceBuffer\0")?,
            device_manager: plat.get(b"MFCreateDXGIDeviceManager\0")?,
            sink_writer: rw.get(b"MFCreateSinkWriterFromURL\0")?,
            _libs: [d3d, plat, rw],
        };
        // MF_VERSION; LITE: sem a parte de rede do Media Foundation.
        unsafe { startup(0x0002_0070, MFSTARTUP_LITE) }.ok()?;
        Ok(api)
    }
}

impl Drop for Api {
    fn drop(&mut self) {
        let _ = unsafe { (self.shutdown)() };
    }
}

/// Chama uma função de criação `f(&mut ptr)` e embrulha o ponteiro.
fn make<T: Interface>(f: impl FnOnce(Out) -> HRESULT) -> Result<T> {
    let mut p = null_mut();
    f(&mut p).ok()?;
    if p.is_null() { Err(E_FAIL.into()) } else { Ok(unsafe { T::from_raw(p) }) }
}

fn texture(device: &ID3D11Device, w: i32, h: i32) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w as u32,
        Height: h as u32,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut t = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut t)) }?;
    t.ok_or_else(|| E_FAIL.into())
}

/// Quantas referências o objeto tem agora (AddRef + Release).
fn refs(t: &ID3D11Texture2D) -> u32 {
    unsafe {
        let raw = t.as_raw();
        let vt = *(raw as *const *const windows::core::IUnknown_Vtbl);
        ((*vt).AddRef)(raw);
        ((*vt).Release)(raw)
    }
}

fn media_type(api: &Api, w: i32, h: i32, fps: u32, sub: &GUID) -> Result<IMFMediaType> {
    let t: IMFMediaType = make(|p| unsafe { (api.media_type)(p) })?;
    unsafe {
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, sub)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
        t.SetUINT64(&MF_MT_FRAME_RATE, ((fps as u64) << 32) | 1)?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
    }
    Ok(t)
}

/// MP4 H.264 no encoder de hardware, recebendo texturas BGRA da mesma GPU (o
/// sink writer põe o conversor para NV12 na GPU, pelo gerenciador DXGI).
fn writer(api: &Api, device: &ID3D11Device, path: &Path, w: i32, h: i32, fps: u32, sound: bool) -> Result<IMFSinkWriter> {
    let mut token = 0;
    let mgr: IMFDXGIDeviceManager = make(|p| unsafe { (api.device_manager)(&mut token, p) })?;
    let attrs: IMFAttributes = make(|p| unsafe { (api.attributes)(p, 3) })?;
    let wpath: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    unsafe {
        mgr.ResetDevice(device, token)?;
        attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        attrs.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, &mgr)?;
        // O arquivo termina em .part: o contêiner não pode vir da extensão.
        attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
        let wr: IMFSinkWriter = make(|p| (api.sink_writer)(wpath.as_ptr(), null_mut(), attrs.as_raw(), p))?;
        let out = media_type(api, w, h, fps, &MFVideoFormat_H264)?;
        out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate(w as u32, h as u32, fps))?;
        let stream = wr.AddStream(&out)?;
        let inp = media_type(api, w, h, fps, &MFVideoFormat_RGB32)?;
        let enc: IMFAttributes = make(|p| (api.attributes)(p, 1))?;
        // VBR sem teto (eAVEncCommonRateControlMode_UnconstrainedVBR): tela parada quase não gasta.
        enc.SetUINT32(&CODECAPI_AVEncCommonRateControlMode, 2)?;
        if wr.SetInputMediaType(stream, &inp, &enc).is_err() {
            wr.SetInputMediaType(stream, &inp, None)?;
        }
        if sound {
            // Faixa 1: AAC estéreo 48 kHz a 160 kbps (20.000 bytes/s, um dos 4 que o
            // AAC encoder MFT aceita), a partir de PCM 16 bits.
            let a = |sub: &GUID| -> Result<IMFMediaType> {
                let t: IMFMediaType = make(|p| (api.media_type)(p))?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                t.SetGUID(&MF_MT_SUBTYPE, sub)?;
                t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
                t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
                Ok(t)
            };
            let out = a(&MFAudioFormat_AAC)?;
            out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 20_000)?;
            let s = wr.AddStream(&out)?;
            let inp = a(&MFAudioFormat_PCM)?;
            inp.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
            inp.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, RATE * 4)?;
            wr.SetInputMediaType(s, &inp, None)?;
        }
        wr.BeginWriting()?;
        Ok(wr)
    }
}

fn write_frame(api: &Api, wr: &IMFSinkWriter, tex: &ID3D11Texture2D, t: i64, d: i64) -> Result<()> {
    unsafe {
        let buf: IMFMediaBuffer = make(|p| (api.surface_buffer)(&ID3D11Texture2D::IID, tex.as_raw(), 0, 0, p))?;
        if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
            buf.SetCurrentLength(b2.GetContiguousLength()?)?;
        }
        let s: IMFSample = make(|p| (api.sample)(p))?;
        s.AddBuffer(&buf)?;
        s.SetSampleTime(t)?;
        s.SetSampleDuration(d)?;
        wr.WriteSample(0, &s)
    }
}

/// PCM 16 bits estéreo a partir da amostra `n`, na faixa 1.
fn write_audio(api: &Api, wr: &IMFSinkWriter, pcm: &[i16], n: u64) -> Result<()> {
    if pcm.is_empty() {
        return Ok(());
    }
    let bytes = pcm.len() as u32 * 2;
    unsafe {
        let buf: IMFMediaBuffer = make(|p| (api.memory_buffer)(bytes, p))?;
        let mut p = null_mut();
        buf.Lock(&mut p, None, None)?;
        core::ptr::copy_nonoverlapping(pcm.as_ptr() as *const u8, p, bytes as usize);
        buf.Unlock()?;
        buf.SetCurrentLength(bytes)?;
        let s: IMFSample = make(|p| (api.sample)(p))?;
        s.AddBuffer(&buf)?;
        let t = audio_time(n);
        s.SetSampleTime(t)?;
        s.SetSampleDuration(audio_time(n + pcm.len() as u64 / 2) - t)?;
        wr.WriteSample(1, &s)
    }
}

/// QPC agora, em 100 ns (a unidade da hora dos pacotes do WASAPI).
fn qpc100() -> i64 {
    use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    let (mut c, mut f) = (0i64, 0i64);
    unsafe { (QueryPerformanceCounter(&mut c), QueryPerformanceFrequency(&mut f)) };
    (c as i128 * 10_000_000 / f.max(1) as i128) as i64
}

/// Os MFTs que o sink writer montou (conversor, encoder), pelo nome: diz se é o NVENC.
fn transforms(wr: &IMFSinkWriter) -> String {
    let mut out = String::new();
    let Ok(ex) = wr.cast::<IMFSinkWriterEx>() else { return out };
    for i in 0.. {
        let mut t = None;
        if unsafe { ex.GetTransformForStream(0, i, None, &mut t) }.is_err() {
            break;
        }
        let name = t.and_then(|t| unsafe { t.GetAttributes() }.ok()).and_then(|a| unsafe {
            let n = a.GetStringLength(&MFT_FRIENDLY_NAME_Attribute).ok()?;
            let mut b = vec![0u16; n as usize + 1];
            a.GetString(&MFT_FRIENDLY_NAME_Attribute, &mut b, None).ok()?;
            Some(String::from_utf16_lossy(&b[..n as usize]))
        });
        out += &format!("{}|", name.unwrap_or_else(|| "?".into()));
    }
    out
}

/// Pico do privado do processo (o PeakPagefileUsage): sem medir a cada quadro.
fn private_peak() -> usize {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    let mut m: PROCESS_MEMORY_COUNTERS = unsafe { core::mem::zeroed() };
    m.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb) };
    m.PeakPagefileUsage
}

fn cpu_time() -> Duration {
    let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut c, mut e, mut k, mut u) = (z, z, z, z);
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    let ns = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) * 100;
    Duration::from_nanos(ns(k) + ns(u))
}

fn monitor_rect(h: *mut c_void) -> (i32, i32, i32, i32) {
    let mut mi: MONITORINFO = unsafe { core::mem::zeroed() };
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    unsafe { GetMonitorInfoW(h, &mut mi) };
    let r = mi.rcMonitor;
    (r.left, r.top, r.right - r.left, r.bottom - r.top)
}

/// Item do WGC, recorte (no espaço da imagem capturada) e área na tela.
type Target = (GraphicsCaptureItem, Option<(i32, i32, i32, i32)>, (i32, i32, i32, i32));

fn target(mode: Mode) -> Result<Target> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    let monitor = |x: i32, y: i32| unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    unsafe {
        match mode {
            Mode::Monitor => {
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                let m = monitor(pt.x, pt.y);
                Ok((interop.CreateForMonitor(HMONITOR(m))?, None, monitor_rect(m)))
            }
            Mode::Region(x, y, w, h) => {
                let m = monitor(x + w / 2, y + h / 2);
                let mon = monitor_rect(m);
                let r = region_in_monitor((x, y, w, h), mon).ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;
                Ok((interop.CreateForMonitor(HMONITOR(m))?, Some(r), (mon.0 + r.0, mon.1 + r.1, r.2, r.3)))
            }
            Mode::Window(hwnd) => {
                // Contorno visível (DWM): o GetWindowRect inclui a borda invisível de ~7 px
                // e a moldura vermelha saía deslocada da imagem gravada.
                let mut r: windows_sys::Win32::Foundation::RECT = core::mem::zeroed();
                let dwm = windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute(hwnd as _, windows_sys::Win32::Graphics::Dwm::DWMWA_EXTENDED_FRAME_BOUNDS as u32, &mut r as *mut _ as _, size_of_val(&r) as u32);
                if dwm != 0 {
                    GetWindowRect(hwnd as _, &mut r);
                }
                let area = (r.left, r.top, r.right - r.left, r.bottom - r.top);
                Ok((interop.CreateForWindow(HWND(hwnd as _))?, None, area))
            }
        }
    }
}

pub fn record(mode: Mode, fps: u32, sound: (bool, bool), part: &Path, stop: &dyn Fn() -> bool, limit: Option<Duration>, started: &dyn Fn(i32, i32, i32, i32)) -> std::result::Result<Stats, String> {
    unsafe {
        CoInitializeEx(null(), COINIT_MULTITHREADED as u32);
        // A thread dorme entre os quadros; com a CPU cheia de outro job, sem
        // prioridade ela acordava até 1 s atrasada e os quadros se perdiam (medido).
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
    }
    let r = Api::load().and_then(|api| run(&api, mode, fps, sound, part, stop, limit, started));
    unsafe { CoUninitialize() };
    r.map_err(|e| format!("{:08x}", e.code().0))
}

#[allow(clippy::too_many_arguments)]
fn run(api: &Api, mode: Mode, fps: u32, sound: (bool, bool), part: &Path, stop: &dyn Fn() -> bool, limit: Option<Duration>, started: &dyn Fn(i32, i32, i32, i32)) -> Result<Stats> {
    let asked = Instant::now();
    // O device D3D11 nasce noutra thread enquanto esta cria o item do WGC e abre
    // o som (cada um leva de ~50 ms a ~1 s com a máquina carregada). Os
    // ponteiros voltam como números: as interfaces do crate não são Send.
    let create = api.create_device;
    let make_device = move || {
        let (mut dev, mut ctx) = (null_mut(), null_mut());
        let levels = [0xb100, 0xb000]; // 11.1, 11.0
        // BGRA (o WGC) + vídeo (o encoder e o conversor do MF).
        let hr = unsafe { create(null_mut(), 1, null_mut(), 0x20 | 0x800, levels.as_ptr(), 2, 7, &mut dev, null_mut(), &mut ctx) };
        (hr, dev as usize, ctx as usize)
    };
    // Cada fonte pedida que não abrir (sem dispositivo, sem permissão do
    // microfone) fica de fora, com o motivo nas notas; o vídeo grava mesmo assim.
    let mut notes = String::new();
    let mut sources: Vec<Source> = Vec::new();
    let (tg, (hr, dev, ctx)) = std::thread::scope(|sc| {
        let h = sc.spawn(make_device);
        let tg = target(mode);
        for (on, system, name) in [(sound.0, true, "sistema"), (sound.1, false, "microfone")] {
            if on {
                match crate::gravar_som::open(system) {
                    Ok(s) => sources.push(s),
                    Err(e) => notes += &format!("sem_{name}:{e}|"),
                }
            }
        }
        (tg, h.join().unwrap_or((E_FAIL, 0, 0)))
    });
    let (item, crop, area) = tg?;
    hr.ok()?;
    let (device, ctx) = unsafe { (ID3D11Device::from_raw(dev as _), ID3D11DeviceContext::from_raw(ctx as _)) };
    // O encoder usa o mesmo device em outra thread.
    let _ = unsafe { device.cast::<ID3D11Multithread>()?.SetMultithreadProtected(true) };
    let dxgi: IDXGIDevice = device.cast()?;
    let rt: IDirect3DDevice = make::<IInspectable>(|p| unsafe { (api.winrt_device)(dxgi.as_raw(), p) })?.cast()?;

    let mut size = item.Size()?;
    let (cx, cy, w, h) = crop.unwrap_or((0, 0, even(size.Width), even(size.Height)));
    if w < 16 || h < 16 {
        return Err(E_FAIL.into());
    }
    let fmt = DirectXPixelFormat::B8G8R8A8UIntNormalized;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&rt, fmt, 2, size)?;
    let session = pool.CreateCaptureSession(&item)?;
    // Sem a borda amarela (Windows 11); onde não dá, ela fica, e não sai no vídeo.
    let _ = session.SetIsBorderRequired(false);
    let latest = texture(&device, w, h)?;
    let mut tracks: Vec<Track> = sources.iter().map(|_| Track::default()).collect();
    let mut t0q = 0;
    let wr = writer(api, &device, part, w, h, fps, !sources.is_empty())?;
    session.StartCapture()?;

    let timer = unsafe { CreateWaitableTimerExW(null(), null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, 0x1F0003) };
    let mut tl = Timeline::new(fps);
    let mut slots: Vec<ID3D11Texture2D> = Vec::new();
    let mut st = Stats { w, h, ..Default::default() };
    let mut first: Option<(Instant, Duration)> = None;
    let mut fresh = false;
    let limit = limit.map(|l| l.as_nanos() as i64 / 100);
    let since = |t: Instant| t.elapsed().as_nanos() as i64 / 100;
    let res = loop {
        // Dorme até a hora do próximo quadro (5 ms enquanto o primeiro não chega).
        let wait = first.map_or(50_000, |(t0, _)| tl.next_time() - since(t0));
        if wait > 0 && !timer.is_null() {
            let due = -wait;
            unsafe {
                SetWaitableTimer(timer, &due, 0, None, null(), 0);
                WaitForSingleObject(timer, INFINITE);
            }
        }
        // Só o último quadro do WGC interessa; os outros voltam para o pool.
        while let Ok(frame) = pool.TryGetNextFrame() {
            let cs = frame.ContentSize()?;
            let tex: ID3D11Texture2D = unsafe { frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()? };
            let bx = D3D11_BOX { left: cx as u32, top: cy as u32, front: 0, right: (cx + w).min(cs.Width) as u32, bottom: (cy + h).min(cs.Height) as u32, back: 1 };
            if bx.right > bx.left && bx.bottom > bx.top {
                unsafe { ctx.CopySubresourceRegion(&latest, 0, 0, 0, 0, &tex, 0, Some(&bx)) };
                fresh = true;
            }
            let _ = frame.Close();
            // Janela que mudou de tamanho: o pool acompanha; o vídeo fica do tamanho do começo.
            if cs != size {
                pool.Recreate(&rt, fmt, 2, cs)?;
                size = cs;
            }
        }
        // Janela gravada que fechou: o vídeo termina (senão o último quadro ficava para sempre).
        let gone = matches!(mode, Mode::Window(h) if unsafe { IsWindow(h as _) } == 0);
        let halt = stop();
        if halt || gone || limit.is_some_and(|l| tl.next_time() >= l) {
            // Parou antes do primeiro quadro: cancela calado, sem MP4 vazio.
            break if halt && first.is_none() { Err(HRESULT(0x8007_04C7_u32 as i32).into()) } else { Ok(()) };
        }
        let t0 = match first {
            Some((t0, _)) => t0,
            None if fresh => {
                // O quadro 0 sai antes de o relógio começar: o NVENC só abre no
                // primeiro WriteSample (~1 s), e sem isso esse segundo viraria
                // quadros perdidos.
                let (t, d) = tl.tick(0, true).unwrap_or_default();
                slots.push(texture(&device, w, h)?);
                unsafe { ctx.CopyResource(&slots[0], &latest) };
                write_frame(api, &wr, &slots[0], t, d)?;
                fresh = false;
                st.start = asked.elapsed();
                first = Some((Instant::now(), cpu_time()));
                t0q = qpc100();
                for s in &sources {
                    s.start()?;
                }
                started(area.0, area.1, w, h);
                continue;
            }
            // Sem o primeiro quadro em 20 s: HRESULT de WAIT_TIMEOUT.
            None if asked.elapsed() > Duration::from_secs(20) => break Err(HRESULT(0x8007_0102_u32 as i32).into()),
            None => continue,
        };
        if !sources.is_empty() {
            let now = audio_index(qpc100() - t0q).max(0) as u64;
            for (s, tr) in sources.iter_mut().zip(&mut tracks) {
                s.poll(t0q, tr);
                // Loopback com o sistema mudo não manda pacote: passados 200 ms
                // sem nada, silêncio até 100 ms atrás (o pacote atrasado que
                // chegar depois perde o começo, em Track::place).
                if tr.end() + RATE as u64 / 5 < now {
                    tr.pad_to(now - RATE as u64 / 10);
                }
            }
            // Só até agora: o loopback marca o pacote com a hora em que ele vai
            // tocar (a latência do dispositivo, até ~180 ms medidos), e passar
            // daqui deixava o áudio mais comprido que o vídeo.
            let n = tracks[0].start;
            write_audio(api, &wr, &mix(&mut tracks, now), n)?;
        }
        let Some((t, d)) = tl.tick(since(t0), fresh) else { continue };
        let Some(slot) = crate::gravar::pick_slot(slots.iter().position(|s| refs(s) == 1), slots.len(), SLOTS) else {
            // O encoder segura todas: este quadro conta como perdido, a imagem fica para o próximo.
            (tl.frames, tl.lost) = (tl.frames - 1, tl.lost + 1);
            continue;
        };
        if slot == slots.len() {
            slots.push(texture(&device, w, h)?);
        }
        unsafe { ctx.CopyResource(&slots[slot], &latest) };
        if let Err(e) = write_frame(api, &wr, &slots[slot], t, d) {
            break Err(e);
        }
        fresh = false;
    };
    if let Some((t0, c0)) = first {
        st.wall = t0.elapsed();
        st.cpu = cpu_time() - c0;
    }
    // O som termina junto com o último quadro: o que falta vira silêncio.
    let res = res.and_then(|_| {
        if sources.is_empty() || first.is_none() {
            return Ok(());
        }
        let end = audio_index(tl.next_time()).max(0) as u64;
        for (s, tr) in sources.iter_mut().zip(&mut tracks) {
            s.poll(t0q, tr);
            tr.pad_to(end);
            st.audio_drift = st.audio_drift.max(tr.drift);
        }
        let n = tracks[0].start;
        write_audio(api, &wr, &mix(&mut tracks, end), n)?;
        st.audio = tracks[0].start;
        Ok(())
    });
    for s in &sources {
        notes += &format!("pacotes:{}{}|", s.packets, if s.dead { ":caiu" } else { "" });
    }
    drop(sources);
    st.private_peak = private_peak();
    let closing = Instant::now();
    let _ = session.Close();
    let _ = pool.Close();
    let fin = res.and_then(|_| unsafe { wr.Finalize() });
    st.stop = closing.elapsed();
    st.notes = transforms(&wr) + &notes;
    if !timer.is_null() {
        unsafe { CloseHandle(timer) };
    }
    (st.frames, st.repeated, st.lost) = (tl.frames, tl.repeated, tl.lost);
    fin.map(|_| st)
}
