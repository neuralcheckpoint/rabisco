//! Gravação de vídeo (motor): o que não depende do Windows fica aqui, testado;
//! a captura (WGC), a GPU e o encoder ficam em `gravar_gpu.rs`.

use std::path::{Path, PathBuf};
use std::time::Duration;

pub const FPS: u32 = 30;
/// Menor lado aceito (o encoder H.264 recusa imagens minúsculas).
const MIN_SIDE: i32 = 16;

pub fn file_name(y: u16, mo: u16, d: u16, h: u16, mi: u16, s: u16) -> String {
    format!("rabisco_{y:04}-{mo:02}-{d:02}_{h:02}h{mi:02}m{s:02}.mp4")
}

/// Bitrate médio em bit/s: ~8 Mbps em 1080p30, proporcional à área. 60 fps leva
/// +50%, não o dobro: na tela, um quadro quase repete o anterior.
pub fn bitrate(w: u32, h: u32, fps: u32) -> u32 {
    let px = w as u64 * h as u64;
    let b = 8_000_000 * px / (1920 * 1080) * (30 + fps as u64) / 60;
    b.clamp(1_000_000, 80_000_000) as u32
}

/// Lado par (o H.264 4:2:0 não aceita ímpar): perde no máximo 1 px.
pub fn even(v: i32) -> i32 {
    v & !1
}

/// A região (x, y, w, h, na tela) recortada dentro do monitor e com lados pares,
/// em coordenadas do monitor. None se sobrar menos de 16 px de lado.
pub fn region_in_monitor(r: (i32, i32, i32, i32), mon: (i32, i32, i32, i32)) -> Option<(i32, i32, i32, i32)> {
    let (x0, y0) = (r.0.max(mon.0), r.1.max(mon.1));
    let x1 = r.0.saturating_add(r.2).min(mon.0 + mon.2);
    let y1 = r.1.saturating_add(r.3).min(mon.1 + mon.3);
    let (w, h) = (even(x1 - x0), even(y1 - y0));
    (w >= MIN_SIDE && h >= MIN_SIDE).then_some((x0 - mon.0, y0 - mon.1, w, h))
}

/// Tempo do quadro n em 100 ns (a unidade do Media Foundation), pela conta
/// inteira a partir do zero: não acumula erro (1/30 s não é exato em 100 ns).
pub fn frame_time(n: u64, fps: u32) -> i64 {
    (n as u128 * 10_000_000 / fps as u128) as i64
}

/// Linha do tempo de quadros constantes. A cada `1/fps` sai um quadro: o novo,
/// se o WGC mandou um, ou o último de novo (tela parada). Se a thread atrasou
/// e pulou a hora de algum quadro, ele conta como perdido e o tempo não volta.
#[derive(Default)]
pub struct Timeline {
    fps: u32,
    next: u64,
    pub frames: u64,
    pub repeated: u64,
    pub lost: u64,
}

impl Timeline {
    pub fn new(fps: u32) -> Timeline {
        Timeline { fps, ..Default::default() }
    }

    /// `elapsed` = 100 ns desde o primeiro quadro; `fresh` = imagem nova desde o
    /// último. Devolve (tempo, duração) do quadro a gravar, ou None se ainda
    /// não é hora.
    pub fn tick(&mut self, elapsed: i64, fresh: bool) -> Option<(i64, i64)> {
        let due = (elapsed.max(0) as u128 * self.fps as u128 / 10_000_000) as u64;
        if due < self.next {
            return None;
        }
        self.lost += due - self.next;
        self.frames += 1;
        self.repeated += !fresh as u64;
        self.next = due + 1;
        let t = frame_time(due, self.fps);
        Some((t, frame_time(due + 1, self.fps) - t))
    }

    /// Hora do próximo quadro (100 ns desde o primeiro).
    pub fn next_time(&self) -> i64 {
        frame_time(self.next, self.fps)
    }
}

/// Estado da gravação, visto pela interface.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum State {
    #[default]
    Idle,
    Recording,
    /// Pediu para parar; a thread ainda fecha o arquivo.
    Stopping,
    Failed,
}

#[derive(PartialEq, Eq, Debug)]
pub enum Act {
    Start,
    Stop,
    Nothing,
}

/// O atalho foi apertado. Durante "parando" não faz nada: começar outra
/// gravação ali abriria dois encoders e dois arquivos ao mesmo tempo.
pub fn press(s: State) -> (State, Act) {
    match s {
        State::Idle | State::Failed => (State::Recording, Act::Start),
        State::Recording => (State::Stopping, Act::Stop),
        State::Stopping => (State::Stopping, Act::Nothing),
    }
}

/// A thread terminou (parou ou falhou), em qualquer estado.
pub fn ended(ok: bool) -> State {
    if ok { State::Idle } else { State::Failed }
}

/// O que gravar: o monitor do cursor, uma região da tela ou uma janela.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Monitor,
    Region(i32, i32, i32, i32),
    Window(isize),
}

/// `monitor` | `regiao:x,y,w,h` | `janela:<hwnd>` (decimal ou 0x...).
pub fn parse_mode(s: &str) -> Option<Mode> {
    if s == "monitor" {
        return Some(Mode::Monitor);
    }
    if let Some(r) = s.strip_prefix("regiao:") {
        let v: Vec<i32> = r.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
        let [x, y, w, h] = v[..] else { return None };
        return Some(Mode::Region(x, y, w, h));
    }
    let h = s.strip_prefix("janela:")?;
    let h = match h.strip_prefix("0x") {
        Some(hex) => isize::from_str_radix(hex, 16).ok()?,
        None => h.parse().ok()?,
    };
    (h != 0).then_some(Mode::Window(h))
}

pub fn part_of(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".part");
    s.into()
}

/// Reserva o nome em `dir` sem pisar em outro vídeo (`_2`, `_3`...): o `.part`
/// é criado com `create_new` e o final não pode existir. Devolve (final, .part).
pub fn reserve(dir: &Path, name: &str) -> Option<(PathBuf, PathBuf)> {
    let stem = name.strip_suffix(".mp4").unwrap_or(name);
    for n in 1..1000 {
        let fin = if n == 1 { dir.join(name) } else { dir.join(format!("{stem}_{n}.mp4")) };
        if fin.exists() {
            continue;
        }
        let part = part_of(&fin);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&part) {
            Ok(_) => return Some((fin, part)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

/// Troca o `.part` pelo nome final. Sem `replace`, não sobrescreve: o hard
/// link falha se o final apareceu nesse meio tempo, e aí o `.part` fica.
pub fn publish(part: &Path, fin: &Path, replace: bool) -> bool {
    if replace {
        return std::fs::rename(part, fin).is_ok();
    }
    std::fs::hard_link(part, fin).is_ok() && std::fs::remove_file(part).is_ok()
}

pub fn videos_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("USERPROFILE")?).join("Videos").join("Rabisco"))
}

/// Números de uma gravação (para o teste e a suíte de performance).
#[derive(Default, Debug)]
pub struct Stats {
    pub w: i32,
    pub h: i32,
    pub frames: u64,
    pub repeated: u64,
    pub lost: u64,
    /// Do pedido ao primeiro quadro gravado.
    pub start: Duration,
    /// Do fim do último quadro ao arquivo fechado (encoder esvaziado).
    pub stop: Duration,
    /// Tempo de CPU do processo (usuário + núcleo) enquanto gravava, e o tempo de relógio.
    pub cpu: Duration,
    pub wall: Duration,
    /// Maior privado visto durante a gravação, em bytes.
    pub private_peak: usize,
    /// Amostras de 48 kHz gravadas na faixa de áudio (0 sem som).
    pub audio: u64,
    /// Maior desvio (amostras) entre a hora de um pacote de som e o fim da fila.
    pub audio_drift: u64,
    pub notes: String,
}

/// Erro de quem parou antes do primeiro quadro (o HRESULT de ERROR_CANCELLED):
/// não é falha, e não sai MP4.
pub const CANCELED: &str = "800704c7";

/// Textura para o próximo quadro: uma livre (só nós a seguramos), uma nova
/// (`len` = criar) até `max`, ou nenhuma: o encoder segura todas e o quadro
/// fica de fora (copiar numa delas estragaria um quadro que ele ainda vai ler).
pub fn pick_slot(free: Option<usize>, len: usize, max: usize) -> Option<usize> {
    free.or((len < max).then_some(len))
}

/// Grava até `stop` (ou `limit`) no `.part`; o chamador renomeia.
/// `started(x, y, w, h)` = área gravada na tela, chamada no primeiro quadro.
/// `sound` = (som do sistema, microfone).
pub fn record(mode: Mode, fps: u32, sound: (bool, bool), part: &Path, stop: &dyn Fn() -> bool, limit: Option<Duration>, started: &dyn Fn(i32, i32, i32, i32)) -> Result<Stats, String> {
    crate::gravar_gpu::record(mode, fps, sound, part, stop, limit, started)
}

// ---------- som (mixagem em f32, sem crate) ----------

/// Taxa da faixa de áudio do MP4 (estéreo, AAC).
pub const RATE: u32 = 48_000;
/// Desvio aceito entre a hora do pacote (QPC) e o fim da fila: 10 ms.
const TOL: i64 = RATE as i64 / 100;
/// Maior salto da fila de uma vez: 60 s (23 MB em f32). Um QPC absurdo não aloca GB.
const JUMP: u64 = RATE as u64 * 60;

/// Amostra de 48 kHz que cai em `t` (100 ns desde o início do vídeo), arredondada para baixo.
pub fn audio_index(t: i64) -> i64 {
    (t as i128 * RATE as i128).div_euclid(10_000_000) as i64
}

/// Tempo da amostra `n` em 100 ns (como o `frame_time`, sem acumular erro).
pub fn audio_time(n: u64) -> i64 {
    (n as u128 * 10_000_000 / RATE as u128) as i64
}

/// Reamostragem linear para estéreo 48 kHz. Mono vira os dois lados; com mais
/// de dois canais ficam a frente esquerda e a direita. Guarda a fase entre os
/// pacotes, então o fluxo não estala na emenda.
pub struct Resampler {
    step: f64,
    ch: usize,
    t: f64,
    prev: [f32; 2],
}

impl Resampler {
    pub fn new(rate: u32, ch: usize) -> Resampler {
        Resampler { step: rate as f64 / RATE as f64, ch: ch.max(1), t: 0.0, prev: [0.0; 2] }
    }

    /// `input` intercalado com `ch` canais; acrescenta pares (esq, dir) em `out`.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for f in input.chunks_exact(self.ch) {
            let cur = [f[0], f[1.min(self.ch - 1)]];
            // Saídas entre a amostra anterior (t = 0) e esta (t = 1).
            while self.t < 1.0 {
                let t = self.t as f32;
                out.extend([self.prev[0] + (cur[0] - self.prev[0]) * t, self.prev[1] + (cur[1] - self.prev[1]) * t]);
                self.t += self.step;
            }
            self.t -= 1.0;
            self.prev = cur;
        }
    }
}

/// Fila de uma fonte (estéreo 48 kHz intercalado), presa à linha do tempo do
/// vídeo: `start` é a amostra de `buf[0]`. Buraco vira silêncio e pacote
/// atrasado perde o começo, então a fonte nunca sai de sincronia por mais de 10 ms.
#[derive(Default)]
pub struct Track {
    pub buf: Vec<f32>,
    pub start: u64,
    /// Maior distância (amostras) entre a hora de um pacote e o fim da fila, antes de
    /// corrigir, sem contar os buracos.
    pub drift: u64,
}

impl Track {
    pub fn end(&self) -> u64 {
        self.start + self.buf.len() as u64 / 2
    }

    /// Põe `s` (pares estéreo) começando na amostra `at` da linha do tempo.
    pub fn place(&mut self, at: i64, s: &[f32]) {
        let end = self.end() as i64;
        // Hora absurda (mais de 60 s à frente): emenda no fim.
        let at = if at > end + JUMP as i64 { end } else { at };
        let mut s = s;
        // O desvio conta o pacote que emenda ou chega atrasado; buraco é silêncio, não desvio.
        if at <= end + TOL {
            self.drift = self.drift.max(at.abs_diff(end));
        }
        if at > end + TOL {
            self.pad_to(at as u64);
        } else if at < end - TOL {
            s = &s[(end.abs_diff(at).min(s.len() as u64) as usize * 2).min(s.len())..];
        }
        self.buf.extend_from_slice(s);
    }

    /// Silêncio até a amostra `n` (o loopback não manda nada com o sistema mudo),
    /// no máximo 60 s por vez: o resto vem nas próximas chamadas.
    pub fn pad_to(&mut self, n: u64) {
        let end = self.end();
        if n > end {
            self.buf.resize(self.buf.len() + (n - end).min(JUMP) as usize * 2, 0.0);
        }
    }
}

/// Soma as fontes até onde todas têm som (no máximo `upto`), em PCM 16 bits
/// estéreo, e tira isso das filas. Todas as filas começam no mesmo `start`.
pub fn mix(tracks: &mut [Track], upto: u64) -> Vec<i16> {
    let Some(start) = tracks.first().map(|t| t.start) else { return Vec::new() };
    let end = tracks.iter().map(Track::end).min().unwrap_or(start).min(upto).max(start);
    let n = (end - start) as usize * 2;
    let out = (0..n).map(|i| (tracks.iter().map(|t| t.buf[i]).sum::<f32>().clamp(-1.0, 1.0) * 32767.0) as i16).collect();
    for t in tracks.iter_mut() {
        t.buf.drain(..n);
        t.start = end;
    }
    out
}

/// O que o Ctrl+Shift+R grava (a última escolha do menu do botão Gravar).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pick {
    Monitor,
    Window,
    Region,
}

/// Última escolha do menu, salva em HKCU\Software\Rabisco ("Gravacao"): o modo e
/// as fontes de som. Sem nada salvo: o monitor, com o som do sistema e sem o
/// microfone (privacidade: o microfone só grava quem ligou).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Prefs {
    pub pick: Pick,
    pub system: bool,
    pub mic: bool,
}

impl Prefs {
    pub fn from_bits(v: Option<u32>) -> Prefs {
        let Some(v) = v else { return Prefs { pick: Pick::Monitor, system: true, mic: false } };
        let pick = match v & 3 {
            1 => Pick::Window,
            2 => Pick::Region,
            _ => Pick::Monitor,
        };
        Prefs { pick, system: v & 4 != 0, mic: v & 8 != 0 }
    }

    pub fn bits(self) -> u32 {
        self.pick as u32 | (self.system as u32) << 2 | (self.mic as u32) << 3
    }

    /// O argumento de som do `--gravar` e do `--gravar-teste`.
    pub fn sound_arg(self) -> &'static str {
        match (self.system, self.mic) {
            (true, true) => "sis,mic",
            (true, false) => "sis",
            (false, true) => "mic",
            _ => "-",
        }
    }
}

/// O inverso do `parse_mode`, para passar o modo ao processo da gravação.
pub fn mode_arg(m: Mode) -> String {
    match m {
        Mode::Monitor => "monitor".into(),
        Mode::Region(x, y, w, h) => format!("regiao:{x},{y},{w},{h}"),
        Mode::Window(h) => format!("janela:{h}"),
    }
}

/// Retângulo (x, y, w, h) arrastado de `a` até `b`, em qualquer direção, com lados pares.
pub fn drag_rect(a: (i32, i32), b: (i32, i32)) -> (i32, i32, i32, i32) {
    (a.0.min(b.0), a.1.min(b.1), even((a.0 - b.0).abs()), even((a.1 - b.1).abs()))
}

/// Tempo gravado no contador: mm:ss, e h:mm:ss passando de uma hora.
pub fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nome_do_video_com_data_e_hora() {
        assert_eq!(file_name(2026, 11, 7, 9, 5, 3), "rabisco_2026-11-07_09h05m03.mp4");
        assert_eq!(file_name(2026, 12, 31, 23, 59, 59), "rabisco_2026-12-31_23h59m59.mp4");
    }

    #[test]
    fn bitrate_pela_area_e_pelos_quadros() {
        assert_eq!(bitrate(1920, 1080, 30), 8_000_000);
        assert_eq!(bitrate(3840, 2160, 30), 32_000_000);
        assert_eq!(bitrate(1920, 1080, 60), 12_000_000);
        assert_eq!(bitrate(2560, 1440, 30), 14_222_222);
        // Região minúscula não fica sem bits; 4K60 não passa do teto.
        assert_eq!(bitrate(64, 64, 30), 1_000_000);
        assert_eq!(bitrate(7680, 4320, 60), 80_000_000);
    }

    #[test]
    fn regiao_par_e_dentro_do_monitor() {
        let mon = (3840, 0, 2560, 1440);
        assert_eq!(region_in_monitor((4000, 100, 640, 480), mon), Some((160, 100, 640, 480)));
        // Ímpar perde 1 px; passar da borda corta.
        assert_eq!(region_in_monitor((4001, 101, 641, 481), mon), Some((161, 101, 640, 480)));
        assert_eq!(region_in_monitor((6000, 1000, 1000, 1000), mon), Some((2160, 1000, 400, 440)));
        assert_eq!(region_in_monitor((3000, -50, 1000, 300), mon), Some((0, 0, 160, 250)));
        // Fora do monitor ou pequena demais.
        assert_eq!(region_in_monitor((0, 0, 640, 480), mon), None);
        assert_eq!(region_in_monitor((4000, 100, 15, 480), mon), None);
        assert_eq!(region_in_monitor((4000, 100, i32::MAX, i32::MAX), mon), Some((160, 100, 2400, 1340)));
        assert_eq!(even(2161), 2160);
    }

    #[test]
    fn linha_do_tempo_monotonica_em_100ns() {
        assert_eq!(frame_time(0, 30), 0);
        assert_eq!(frame_time(1, 30), 333_333);
        assert_eq!(frame_time(30, 30), 10_000_000);
        assert_eq!(frame_time(3, 60), 500_000);
        let mut tl = Timeline::new(30);
        let mut last = -1;
        let mut total = 0;
        // A thread acorda em horas tortas, às vezes antes da hora.
        for i in 0..300 {
            let elapsed = i * 333_333 / 2 + (i % 7) * 1_000;
            if let Some((t, d)) = tl.tick(elapsed, i % 3 == 0) {
                assert!(t > last, "tempo voltou: {t} <= {last}");
                assert!(d == 333_333 || d == 333_334, "duração {d}");
                last = t;
                total += d;
            }
        }
        assert_eq!(tl.lost, 0);
        assert_eq!(tl.frames, 150);
        assert_eq!(total, frame_time(150, 30));
        assert_eq!(tl.next_time(), frame_time(150, 30));
    }

    #[test]
    fn linha_do_tempo_repete_e_conta_perdidos() {
        let mut tl = Timeline::new(30);
        assert_eq!(tl.tick(0, true), Some((0, 333_333)));
        // Antes da hora do próximo: nada.
        assert_eq!(tl.tick(300_000, false), None);
        // Tela parada: o mesmo quadro de novo, com o tempo certo.
        assert_eq!(tl.tick(340_000, false), Some((333_333, 333_333)));
        assert_eq!(tl.repeated, 1);
        // A thread dormiu 3 quadros: os do meio são perdidos, o tempo segue.
        assert_eq!(tl.tick(frame_time(5, 30) + 10, true), Some((frame_time(5, 30), 333_334)));
        assert_eq!((tl.frames, tl.lost, tl.repeated), (3, 3, 1));
    }

    #[test]
    fn maquina_de_estados_e_atalho_durante_parando() {
        assert_eq!(press(State::Idle), (State::Recording, Act::Start));
        assert_eq!(press(State::Recording), (State::Stopping, Act::Stop));
        // Segundo atalho enquanto o arquivo fecha: ignorado.
        assert_eq!(press(State::Stopping), (State::Stopping, Act::Nothing));
        assert_eq!(press(State::Stopping), (State::Stopping, Act::Nothing));
        assert_eq!(ended(true), State::Idle);
        // A falha (sem WGC, sem encoder) não trava: o próximo atalho tenta de novo.
        assert_eq!(ended(false), State::Failed);
        assert_eq!(press(State::Failed), (State::Recording, Act::Start));
    }

    #[test]
    fn modos_do_gravar_teste() {
        assert_eq!(parse_mode("monitor"), Some(Mode::Monitor));
        assert_eq!(parse_mode("regiao:10,-20,640,480"), Some(Mode::Region(10, -20, 640, 480)));
        assert_eq!(parse_mode("janela:0x1A2B"), Some(Mode::Window(0x1a2b)));
        assert_eq!(parse_mode("janela:4242"), Some(Mode::Window(4242)));
        for ruim in ["regiao:1,2,3", "regiao:a,b,c,d", "janela:0", "janela:", "tela", ""] {
            assert_eq!(parse_mode(ruim), None, "{ruim}");
        }
    }

    #[test]
    fn reserva_o_nome_e_publica_sem_sobrescrever() {
        let dir = std::env::temp_dir().join(format!("rabisco-video-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let name = "rabisco_2026-11-07_10h00m00.mp4";
        let (f1, p1) = reserve(&dir, name).unwrap();
        assert_eq!(f1, dir.join(name));
        assert_eq!(p1, dir.join(format!("{name}.part")));
        // Outra gravação no mesmo segundo, com a primeira ainda em .part.
        let (f2, p2) = reserve(&dir, name).unwrap();
        assert_eq!(f2, dir.join("rabisco_2026-11-07_10h00m00_2.mp4"));
        std::fs::write(&p1, b"um").unwrap();
        assert!(publish(&p1, &f1, false));
        assert!(!p1.exists());
        // Com o final ocupado, não sobrescreve e o .part fica.
        std::fs::write(&p2, b"dois").unwrap();
        assert!(!publish(&p2, &f1, false));
        assert_eq!(std::fs::read(&f1).unwrap(), b"um");
        assert!(p2.exists());
        assert!(publish(&p2, &f2, false));
        let (f3, _) = reserve(&dir, name).unwrap();
        assert_eq!(f3, dir.join("rabisco_2026-11-07_10h00m00_3.mp4"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn amostra_e_tempo_do_audio_em_100ns() {
        assert_eq!(audio_index(0), 0);
        assert_eq!(audio_index(10_000_000), 48_000);
        assert_eq!(audio_index(-1), -1);
        assert_eq!(audio_time(48_000), 10_000_000);
        assert_eq!(audio_time(1), 208);
        // Ida e volta não perde amostra.
        for n in [0u64, 1, 479, 48_001, 1_000_000_007] {
            assert_eq!(audio_index(audio_time(n) + 1), n as i64);
        }
    }

    #[test]
    fn reamostra_para_estereo_48k() {
        // 48 kHz estéreo: passa igual (atrasado de uma amostra, a da fase).
        let mut r = Resampler::new(48_000, 2);
        let mut out = Vec::new();
        r.push(&[0.1, -0.1, 0.2, -0.2, 0.3, -0.3], &mut out);
        assert_eq!(out, [0.0, 0.0, 0.1, -0.1, 0.2, -0.2]);
        // Mono 24 kHz: o dobro de amostras, nos dois lados, com o meio interpolado.
        let mut r = Resampler::new(24_000, 1);
        let mut out = Vec::new();
        r.push(&[1.0, 1.0], &mut out);
        r.push(&[0.0], &mut out);
        assert_eq!(out, [0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.5, 0.5]);
        // 44,1 kHz em 5.1: um segundo vira um segundo, e só a frente entra.
        let mut r = Resampler::new(44_100, 6);
        let mut out = Vec::new();
        let um_s: Vec<f32> = (0..44_100).flat_map(|_| [0.25, -0.25, 9.0, 9.0, 9.0, 9.0]).collect();
        r.push(&um_s[..6 * 1000], &mut out);
        r.push(&um_s[6 * 1000..], &mut out);
        assert!((out.len() as i64 / 2 - 48_000).abs() <= 1, "{}", out.len() / 2);
        // As duas primeiras saem entre o silêncio inicial e a primeira amostra.
        assert!(out[4..].chunks(2).all(|p| p == [0.25, -0.25]));
    }

    #[test]
    fn fila_preenche_buraco_e_corta_atraso() {
        let mut t = Track::default();
        t.place(0, &[0.5; 200]);
        assert_eq!(t.end(), 100);
        // Pacote 5 ms depois do fim: dentro da folga, emenda sem silêncio.
        t.place(340, &[0.5; 20]);
        assert_eq!(t.end(), 110);
        // Buraco de 100 ms (sistema mudo): silêncio até a hora do pacote.
        t.place(110 + 4_800, &[0.5; 4]);
        assert_eq!(t.end(), 110 + 4_800 + 2);
        assert!(t.buf[220..220 + 9_600].iter().all(|&v| v == 0.0));
        assert_eq!(t.drift, 240);
        // Pacote que começa 20 ms antes do fim (já veio silêncio por ele): perde o começo.
        let fim = t.end();
        t.place(fim as i64 - 960, &[0.25; 2 * 1000]);
        assert_eq!(t.end(), fim + 40);
        assert_eq!(t.drift, 960);
        // Pacote de antes do início do vídeo.
        let mut t = Track::default();
        t.place(-1000, &[0.5; 2 * 1500]);
        assert_eq!(t.end(), 500);
        t.pad_to(600);
        assert_eq!(t.end(), 600);
        t.pad_to(10);
        assert_eq!(t.end(), 600);
    }

    #[test]
    fn hora_absurda_nao_aloca_gb() {
        let mut t = Track::default();
        t.place(0, &[0.5; 200]);
        // QPC de horas à frente: o pacote emenda no fim, sem silêncio no meio.
        t.place(48_000 * 3600 * 5, &[0.5; 20]);
        assert_eq!(t.end(), 110);
        // E de muito antes do início: cai inteiro, sem estourar a conta.
        t.place(i64::MIN / 2, &[0.5; 20]);
        assert_eq!(t.end(), 110);
        // Silêncio de horas: 60 s por chamada, até chegar.
        t.pad_to(110 + JUMP * 2 + 5);
        assert_eq!(t.end(), 110 + JUMP);
        t.pad_to(110 + JUMP * 2 + 5);
        t.pad_to(110 + JUMP * 2 + 5);
        assert_eq!(t.end(), 110 + JUMP * 2 + 5);
    }

    #[test]
    fn quadro_sem_textura_livre_fica_de_fora() {
        assert_eq!(pick_slot(Some(2), 4, 4), Some(2));
        // Nenhuma livre: cria enquanto cabe, depois não escreve em ocupada.
        assert_eq!(pick_slot(None, 1, 4), Some(1));
        assert_eq!(pick_slot(None, 4, 4), None);
    }

    #[test]
    fn mistura_soma_corta_e_espera_a_mais_curta() {
        let mut ts = [Track::default(), Track::default()];
        ts[0].place(0, &[0.5, -0.5, 0.75, 0.75, 0.1, 0.1]);
        ts[1].place(0, &[0.25, -0.75, 0.5, 0.5]);
        let pcm = mix(&mut ts, u64::MAX);
        assert_eq!(pcm, [24575, -32767, 32767, 32767]);
        // A primeira ficou com uma amostra; as duas andaram juntas.
        assert_eq!((ts[0].start, ts[0].end(), ts[1].start, ts[1].end()), (2, 3, 2, 2));
        // Com teto, só até ele.
        ts[1].pad_to(10);
        ts[0].pad_to(10);
        assert_eq!(mix(&mut ts, 5).len(), 6);
        assert_eq!(ts[0].start, 5);
        // Uma fonte só.
        let mut um = [Track::default()];
        um[0].place(0, &[1.0, -1.0]);
        assert_eq!(mix(&mut um, u64::MAX), [32767, -32767]);
    }

    #[test]
    fn preferencias_da_gravacao_no_registro() {
        // Nada salvo: monitor, som do sistema, microfone desligado.
        let p = Prefs::from_bits(None);
        assert_eq!(p, Prefs { pick: Pick::Monitor, system: true, mic: false });
        assert_eq!(p.sound_arg(), "sis");
        for pick in [Pick::Monitor, Pick::Window, Pick::Region] {
            for (system, mic) in [(false, false), (true, false), (false, true), (true, true)] {
                let p = Prefs { pick, system, mic };
                assert_eq!(Prefs::from_bits(Some(p.bits())), p);
            }
        }
        assert_eq!(Prefs::from_bits(Some(0)).sound_arg(), "-");
        assert_eq!(Prefs { pick: Pick::Region, system: true, mic: true }.sound_arg(), "sis,mic");
    }

    #[test]
    fn modo_vai_e_volta_pelo_argumento() {
        for m in [Mode::Monitor, Mode::Region(-1920, 10, 640, 480), Mode::Window(0x1a2b)] {
            assert_eq!(parse_mode(&mode_arg(m)), Some(m));
        }
    }

    #[test]
    fn arrasto_em_qualquer_direcao_com_lados_pares() {
        assert_eq!(drag_rect((10, 20), (651, 501)), (10, 20, 640, 480));
        assert_eq!(drag_rect((651, 501), (10, 20)), (10, 20, 640, 480));
        assert_eq!(drag_rect((-100, 50), (-300, 10)), (-300, 10, 200, 40));
        assert_eq!(drag_rect((5, 5), (5, 6)), (5, 5, 0, 0));
    }

    #[test]
    fn contador_mm_ss() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(59), "00:59");
        assert_eq!(clock(61), "01:01");
        assert_eq!(clock(3599), "59:59");
        assert_eq!(clock(3600 + 62), "1:01:02");
    }
}
