//! Som da gravação: WASAPI em modo compartilhado, o loopback do dispositivo de
//! saída padrão (o som do sistema) e a captura do de entrada padrão (o
//! microfone). Cada pacote vai para estéreo 48 kHz em f32 (`gravar::Resampler`)
//! e entra na fila da fonte (`gravar::Track`) na hora que o WASAPI diz (QPC).

use crate::gravar::{Resampler, Track, audio_index};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use windows::core::Result;

/// Buffer do WASAPI: 1 s, para a thread que dorme entre os quadros (ou que
/// esperou o NVENC abrir) não perder pacote.
const BUFFER: i64 = 10_000_000;

#[derive(Clone, Copy)]
enum Fmt {
    F32,
    I16,
    I32,
}

pub struct Source {
    client: IAudioClient,
    cap: IAudioCaptureClient,
    fmt: Fmt,
    ch: usize,
    rs: Resampler,
    scratch: Vec<f32>,
    out: Vec<f32>,
    /// Pacotes recebidos (para o relatório).
    pub packets: u64,
    /// O dispositivo caiu no meio (desconectado: AUDCLNT_E_DEVICE_INVALIDATED).
    pub dead: bool,
}

/// Abre o loopback (`system`) ou o microfone. O erro vira texto curto para as notas.
pub fn open(system: bool) -> std::result::Result<Source, String> {
    open_inner(system).map_err(|e| format!("{:08x}", e.code().0))
}

fn open_inner(system: bool) -> Result<Source> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        // Sem dispositivo padrão: E_NOTFOUND (0x80070490).
        let dev = en.GetDefaultAudioEndpoint(if system { eRender } else { eCapture }, eConsole)?;
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
        let wf = client.GetMixFormat()?;
        let f = *wf;
        // O formato de mixagem do Windows: quase sempre float 32 (EXTENSIBLE).
        let sub = if f.wFormatTag == 0xFFFE { (*(wf as *const WAVEFORMATEXTENSIBLE)).SubFormat.data1 as u16 } else { f.wFormatTag };
        let fmt = match (sub, f.wBitsPerSample) {
            (3, 32) => Some(Fmt::F32),
            (1, 16) => Some(Fmt::I16),
            (1, 32) => Some(Fmt::I32),
            _ => None,
        };
        let flags = if system { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
        let init = fmt.map(|_| client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER, 0, wf, None));
        CoTaskMemFree(Some(wf as _));
        // Formato que não sei ler: HRESULT de formato não suportado.
        let Some(fmt) = fmt else { return Err(AUDCLNT_E_UNSUPPORTED_FORMAT.into()) };
        init.unwrap()?;
        let cap: IAudioCaptureClient = client.GetService()?;
        let ch = f.nChannels.max(1) as usize;
        Ok(Source { client, cap, fmt, ch, rs: Resampler::new(f.nSamplesPerSec, ch), scratch: Vec::new(), out: Vec::new(), packets: 0, dead: false })
    }
}

impl Source {
    pub fn start(&self) -> Result<()> {
        unsafe { self.client.Start() }
    }

    /// Lê todos os pacotes prontos para a fila. `t0` = QPC do início do vídeo, em
    /// 100 ns. Um erro desliga só esta fonte: a faixa dela segue em silêncio
    /// (o `pad_to` de quem chama) e o vídeo fecha normalmente.
    pub fn poll(&mut self, t0: i64, track: &mut Track) {
        if !self.dead && self.read(t0, track).is_err() {
            self.dead = true;
        }
    }

    fn read(&mut self, t0: i64, track: &mut Track) -> Result<()> {
        loop {
            let n = unsafe { self.cap.GetNextPacketSize()? };
            if n == 0 {
                return Ok(());
            }
            let (mut data, mut frames, mut flags, mut qpc) = (core::ptr::null_mut(), 0u32, 0u32, 0u64);
            unsafe { self.cap.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc))? };
            let len = frames as usize * self.ch;
            self.scratch.clear();
            // Pacote marcado como silêncio: os dados não valem, são zeros.
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                self.scratch.resize(len, 0.0);
            } else {
                unsafe {
                    match self.fmt {
                        Fmt::F32 => self.scratch.extend_from_slice(core::slice::from_raw_parts(data as *const f32, len)),
                        Fmt::I16 => self.scratch.extend(core::slice::from_raw_parts(data as *const i16, len).iter().map(|&v| v as f32 / 32768.0)),
                        Fmt::I32 => self.scratch.extend(core::slice::from_raw_parts(data as *const i32, len).iter().map(|&v| v as f32 / 2_147_483_648.0)),
                    }
                }
            }
            unsafe { self.cap.ReleaseBuffer(frames)? };
            self.packets += 1;
            self.out.clear();
            self.rs.push(&self.scratch, &mut self.out);
            // A hora do pacote (QPC em 100 ns) diz onde ele entra na linha do
            // tempo; com TIMESTAMP_ERROR ela não vale, e o pacote emenda no fim.
            let at = if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 { track.end() as i64 } else { audio_index((qpc as i64).wrapping_sub(t0)) };
            track.place(at, &self.out);
        }
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}
