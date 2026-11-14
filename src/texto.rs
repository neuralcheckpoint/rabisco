//! Caixa de texto: edição (WM_CHAR, com acento e cedilha), quebra de linha e
//! rasterização com o GDI (Segoe UI, antialias em tons de cinza). A cobertura
//! sai do canal verde do texto branco sobre preto e vira um `TextMask`, que o
//! canvas trata como qualquer traço (borracha, desfazer, reconstrução).

use crate::canvas::TextMask;
use core::ptr::null_mut;
use windows_sys::Win32::Foundation::SIZE;
use windows_sys::Win32::Graphics::Gdi::*;

/// Tamanho da fonte (px) para a espessura da barra: 2/4/8/16 → 24/32/48/80.
pub fn font_px(size: f32) -> i32 {
    (16.0 + size * 4.0).round().clamp(12.0, 400.0) as i32
}

/// Texto sendo digitado, com o cursor em `caret` (byte, sempre num limite de letra).
#[derive(Default)]
pub struct Editor {
    pub text: String,
    pub caret: usize,
    /// Metade alta de um par UTF-16 (emoji) esperando a outra.
    high: Option<u16>,
}

/// Marca que se junta à letra de antes (acento combinado): anda e sai junto com ela.
fn combining(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F | 0xFE0F)
}

/// Teclas de mover o cursor na caixa.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Move {
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
}

impl Editor {
    /// Abre com um texto já pronto (editar um texto confirmado), cursor no fim.
    pub fn with(text: &str) -> Editor {
        Editor { text: text.into(), caret: text.len(), high: None }
    }

    /// Uma unidade UTF-16 de WM_CHAR. Devolve true se o texto mudou.
    /// As teclas mortas do ABNT2 (´ ~ ^ `) chegam já compostas (á, ã, ê) pelo
    /// TranslateMessage; aqui só entra o caractere pronto.
    pub fn input(&mut self, u: u16) -> bool {
        match u {
            0x08 => self.backspace(),
            0x0D => self.insert("\n"),
            0xD800..=0xDBFF => {
                self.high = Some(u);
                false
            }
            0xDC00..=0xDFFF => {
                let Some(h) = self.high.take() else { return false };
                match char::decode_utf16([h, u]).next() {
                    Some(Ok(c)) => self.insert(c.encode_utf8(&mut [0; 4])),
                    _ => false,
                }
            }
            // Controles (Esc, Tab, Ctrl+letra) não escrevem nada.
            0..=0x1F | 0x7F => false,
            _ => {
                self.high = None;
                match char::from_u32(u as u32) {
                    Some(c) => self.insert(c.encode_utf8(&mut [0; 4])),
                    None => false,
                }
            }
        }
    }

    /// Põe `s` no cursor (colar: `\r\n` vira quebra, tab vira espaço, outros controles saem).
    pub fn insert(&mut self, s: &str) -> bool {
        let clean: String = s.replace("\r\n", "\n").replace('\r', "\n").chars().map(|c| if c == '\t' { ' ' } else { c }).filter(|&c| c == '\n' || !c.is_control()).collect();
        self.text.insert_str(self.caret, &clean);
        self.caret += clean.len();
        !clean.is_empty()
    }

    /// Começo da letra antes de `i`, com os acentos combinados dela.
    fn prev(&self, i: usize) -> usize {
        let mut at = i;
        for (b, c) in self.text[..i].char_indices().rev() {
            at = b;
            if !combining(c) {
                break;
            }
        }
        at
    }

    /// Fim da letra que começa em `i`, com os acentos combinados que vêm depois.
    fn next(&self, i: usize) -> usize {
        let mut chars = self.text[i..].char_indices();
        let Some((_, c)) = chars.next() else { return i };
        let mut at = i + c.len_utf8();
        for (b, c) in chars {
            if !combining(c) {
                break;
            }
            at = i + b + c.len_utf8();
        }
        at
    }

    /// Apaga a letra antes do cursor, com os acentos combinados dela.
    pub fn backspace(&mut self) -> bool {
        let p = self.prev(self.caret);
        let changed = p < self.caret;
        self.text.replace_range(p..self.caret, "");
        self.caret = p;
        changed
    }

    /// Delete: apaga a letra depois do cursor.
    pub fn delete(&mut self) -> bool {
        let n = self.next(self.caret);
        self.text.replace_range(self.caret..n, "");
        n > self.caret
    }

    /// Move o cursor; `lines` (do último desenho) serve para Home, End, cima e baixo.
    /// Devolve true se ele andou.
    pub fn move_caret(&mut self, m: Move, lines: &[Line]) -> bool {
        let old = self.caret;
        let li = line_of(lines, self.caret);
        let line = lines.get(li).copied().unwrap_or(Line { start: 0, end: self.text.len(), w: 0 });
        self.caret = match m {
            Move::Left => self.prev(self.caret),
            Move::Right => self.next(self.caret),
            Move::Home => line.start,
            Move::End => line.end,
            Move::Up | Move::Down => {
                let to = if m == Move::Up { li.checked_sub(1) } else { Some(li + 1).filter(|&j| j < lines.len()) };
                match to.map(|j| lines[j]) {
                    // Mesma quantidade de letras desde o começo da linha.
                    Some(t) => {
                        let n = self.text[line.start..self.caret.max(line.start)].chars().count();
                        self.text[t.start..t.end].char_indices().nth(n).map_or(t.end, |(b, _)| t.start + b)
                    }
                    None if m == Move::Up => 0,
                    None => self.text.len(),
                }
            }
        };
        self.high = None;
        self.caret != old
    }
}

/// Linha onde está o byte `i` (a última que começa antes dele).
pub fn line_of(lines: &[Line], i: usize) -> usize {
    lines.iter().rposition(|l| l.start <= i).unwrap_or(0)
}

/// Uma linha do texto: bytes [start, end) e largura em px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    pub w: i32,
}

/// Quebra o texto em linhas: Enter sempre quebra; linha que passa de `max_w`
/// quebra antes da palavra que não cabe, e palavra sozinha maior que a linha
/// quebra entre letras. `widths` dá, de uma vez por parágrafo, a largura do
/// começo até cada byte: a quebra fica linear (medir o resto a cada linha era
/// quadrático, 308 ms com 20.000 letras).
pub fn wrap(text: &str, max_w: i32, widths: &mut dyn FnMut(&str, &mut Vec<i32>)) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut cum = Vec::new();
    let mut base = 0;
    for para in text.split('\n') {
        widths(para, &mut cum);
        let b = para.as_bytes();
        let width = |s: usize, e: usize| cum[e] - cum[s];
        // O espaço do fim da linha quebrada não conta.
        let trim = |s: usize, mut e: usize| {
            while e > s && b[e - 1] == b' ' {
                e -= 1;
            }
            e
        };
        let mut s = 0;
        loop {
            if width(s, para.len()) <= max_w || s == para.len() {
                lines.push(Line { start: base + s, end: base + para.len(), w: width(s, para.len()) });
                break;
            }
            // Último ponto de quebra (começo de palavra depois de espaço) que ainda cabe.
            let mut cut = None;
            for i in s + 1..para.len() {
                if width(s, trim(s, i)) > max_w {
                    break;
                }
                if b[i - 1] == b' ' && b[i] != b' ' {
                    cut = Some(i);
                }
            }
            let cut = cut.unwrap_or_else(|| {
                // Palavra maior que a linha: corta entre letras (pelo menos uma).
                let mut idx = para[s..].char_indices().map(|(i, _)| s + i).skip(1);
                let mut e = idx.next().unwrap_or(para.len());
                for i in idx {
                    if width(s, i) <= max_w { e = i } else { break }
                }
                e
            });
            lines.push(Line { start: base + s, end: base + cut, w: width(s, trim(s, cut)) });
            s = cut;
        }
        base += para.len() + 1;
    }
    lines
}

/// Margem da caixa em volta do texto, em px.
pub fn pad(px: i32) -> i32 {
    (px / 6).max(2)
}

/// Caixa (w, h) das linhas e posição do cursor (canto de cima, relativo à caixa).
pub fn text_box(lines: &[Line], line_h: i32, pad: i32, caret_w: i32) -> ((i32, i32), (i32, i32)) {
    let w = lines.iter().map(|l| l.w).max().unwrap_or(0) + 2 * pad + caret_w;
    let h = lines.len().max(1) as i32 * line_h + 2 * pad;
    let last = lines.last().map_or(0, |l| l.w);
    let caret = (pad + last, pad + (lines.len().max(1) as i32 - 1) * line_h);
    ((w, h), caret)
}

/// Fonte Segoe UI num DC de memória, viva enquanto a caixa está aberta.
pub struct Font {
    dc: HDC,
    font: HFONT,
    old: HGDIOBJ,
    pub px: i32,
    pub line_h: i32,
}

/// O que a caixa aberta lembra da tecla anterior (ver `Font::update`).
#[derive(Default)]
pub struct Laid {
    text: String,
    pub lines: Vec<Line>,
}

/// Texto rasterizado + onde fica o cursor (na tela, relativo ao canto da caixa).

#[cfg(test)]
pub struct Rendered {
    pub mask: TextMask,
    pub caret: (i32, i32),
}

impl Font {
    pub fn new(px: i32) -> Option<Font> {
        unsafe {
            let face: Vec<u16> = "Segoe UI".encode_utf16().chain(Some(0)).collect();
            // ClearType não combina com janela em camadas: antialias em cinza.
            let font = CreateFontW(
                -px,
                0,
                0,
                0,
                FW_SEMIBOLD as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_TT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                ANTIALIASED_QUALITY as u32,
                (DEFAULT_PITCH | FF_SWISS) as u32,
                face.as_ptr(),
            );
            if font.is_null() {
                return None;
            }
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                DeleteObject(font);
                return None;
            }
            let old = SelectObject(dc, font);
            let mut tm: TEXTMETRICW = core::mem::zeroed();
            GetTextMetricsW(dc, &mut tm);
            let line_h = (tm.tmHeight + tm.tmExternalLeading).max(1);
            Some(Font { dc, font, old, px, line_h })
        }
    }

    /// Largura do começo de `s` até cada byte (nos limites de letra), numa
    /// chamada só do GDI.
    pub fn widths(&self, s: &str, cum: &mut Vec<i32>) {
        let u: Vec<u16> = s.encode_utf16().collect();
        let mut dx = vec![0i32; u.len()];
        if !u.is_empty() {
            let mut sz = SIZE { cx: 0, cy: 0 };
            unsafe { GetTextExtentExPointW(self.dc, u.as_ptr(), u.len() as i32, 0, null_mut(), dx.as_mut_ptr(), &mut sz) };
        }
        cum.clear();
        cum.resize(s.len() + 1, 0);
        let mut k = 0;
        for (b, c) in s.char_indices() {
            cum[b] = if k == 0 { 0 } else { dx[k - 1] };
            k += c.len_utf16();
        }
        cum[s.len()] = if k == 0 { 0 } else { dx[k - 1] };
    }

    #[cfg(test)]
    pub fn measure(&self, s: &str) -> i32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut sz = SIZE { cx: 0, cy: 0 };
        unsafe { GetTextExtentPoint32W(self.dc, w.as_ptr(), w.len() as i32, &mut sz) };
        sz.cx
    }

    /// Largura do cursor (px).
    pub fn caret_w(&self) -> i32 {
        (self.px / 16).max(2)
    }

    /// Quebra em `max_w` (largura total da caixa) e rasteriza.
    #[cfg(test)]
    pub fn render(&self, text: &str, max_w: i32) -> Rendered {
        let mut mask = TextMask::default();
        let (_, caret) = self.update(text, text.len(), max_w, &mut Laid::default(), &mut mask);
        Rendered { mask, caret }
    }

    /// Refaz `mask` para `text` a partir do que `laid` lembra da tecla anterior:
    /// o GDI só redesenha da linha que mudou para baixo (desde uma acima: o
    /// acento e a perna do ç passam da linha). Devolve a primeira linha de
    /// pixels que mudou e o cursor.
    pub fn update(&self, text: &str, at: usize, max_w: i32, laid: &mut Laid, mask: &mut TextMask) -> (i32, (i32, i32)) {
        let pad = pad(self.px);
        let cw = self.caret_w();
        let lines = wrap(text, (max_w - 2 * pad - cw).max(self.px), &mut |s, cum| self.widths(s, cum));
        let ((w, h), mut caret) = text_box(&lines, self.line_h, pad, cw);
        if at < text.len() {
            // Cursor no meio: largura do começo da linha até ele.
            let li = line_of(&lines, at);
            let mut cum = Vec::new();
            self.widths(&text[lines[li].start..at.max(lines[li].start)], &mut cum);
            caret = (pad + cum.last().copied().unwrap_or(0), pad + li as i32 * self.line_h);
        }
        mask.src.clear();
        mask.src.push_str(text);
        mask.px = self.px;
        let same = |a: &Line, b: &Line| a.w == b.w && text[a.start..a.end] == laid.text[b.start..b.end];
        let k = lines.iter().zip(&laid.lines).take_while(|(a, b)| same(a, b)).count();
        let first = k.saturating_sub(1);
        let from = if first == 0 { 0 } else { (pad + first as i32 * self.line_h).min(h) };
        if mask.w != w {
            // Caixa mais larga ou mais estreita: as linhas de cima ficam, em
            // outra largura (sem passar pelo GDI de novo).
            let mut cov = vec![0u8; (w * h) as usize];
            let n = w.min(mask.w) as usize;
            for y in 0..from.min(mask.h) as usize {
                cov[y * w as usize..][..n].copy_from_slice(&mask.cov[y * mask.w as usize..][..n]);
            }
            mask.cov = cov;
        }
        mask.cov.resize((w * h) as usize, 0);
        (mask.w, mask.h) = (w, h);
        let band = &mut mask.cov[(from * w) as usize..];
        band.fill(0);
        if !text.trim().is_empty() && from < h {
            let j = first.saturating_sub(1);
            self.raster(text, &lines[j..], pad + j as i32 * self.line_h - from, w, h - from, band);
        }
        laid.text.clear();
        laid.text.push_str(text);
        laid.lines = lines;
        (from, caret)
    }

    /// Desenha `lines` (a primeira em y0) numa faixa w x h e põe em `cov`.
    fn raster(&self, text: &str, lines: &[Line], y0: i32, w: i32, h: i32, cov: &mut [u8]) {
        let pad = pad(self.px);
        unsafe {
            let mut bmi: BITMAPINFO = core::mem::zeroed();
            bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;
            let mut bits = null_mut();
            let dib = CreateDIBSection(null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if dib.is_null() {
                return;
            }
            let prev = SelectObject(self.dc, dib);
            SetBkMode(self.dc, TRANSPARENT as i32);
            SetTextColor(self.dc, 0x00FF_FFFF);
            for (i, l) in lines.iter().enumerate() {
                let u: Vec<u16> = text[l.start..l.end].trim_end().encode_utf16().collect();
                TextOutW(self.dc, pad, y0 + i as i32 * self.line_h, u.as_ptr(), u.len() as i32);
            }
            GdiFlush();
            // Branco sobre preto em cinza: qualquer canal é a cobertura.
            let px = core::slice::from_raw_parts(bits as *const u32, (w * h) as usize);
            for (c, p) in cov.iter_mut().zip(px) {
                *c = (p >> 8) as u8;
            }
            SelectObject(self.dc, prev);
            DeleteObject(dib);
        }
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteDC(self.dc);
            DeleteObject(self.font);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(units: &[u16]) -> Editor {
        let mut e = Editor::default();
        for &u in units {
            e.input(u);
        }
        e
    }

    fn utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn digita_acento_cedilha_enter_e_apaga() {
        // O que o WM_CHAR entrega no ABNT2: as teclas mortas já vêm compostas.
        let mut e = typed(&utf16("ação é"));
        e.input(0x0D);
        for u in utf16("ÇÃ") {
            e.input(u);
        }
        assert_eq!(e.text, "ação é\nÇÃ");
        e.input(0x08);
        assert_eq!(e.text, "ação é\nÇ");
        e.input(0x08);
        e.input(0x08); // apaga a quebra de linha
        assert_eq!(e.text, "ação é");
    }

    #[test]
    fn backspace_leva_o_acento_combinado_junto() {
        let mut e = typed(&[0x61, 0x6F, 0x0303]); // "aõ" com o til separado
        assert_eq!(e.text.chars().count(), 3);
        assert!(e.backspace());
        assert_eq!(e.text, "a");
        assert!(e.backspace());
        assert!(!e.backspace()); // vazio: nada muda
    }

    #[test]
    fn controles_nao_escrevem_e_emoji_chega_em_duas_metades() {
        let mut e = typed(&[0x1B, 0x09, 0x01, 0x7F]); // Esc, Tab, Ctrl+A, Ctrl+Backspace
        assert_eq!(e.text, "");
        assert!(!e.input(0xD83D));
        assert!(e.input(0xDE00));
        assert_eq!(e.text, "😀");
        assert!(!e.input(0xDE00)); // metade baixa solta é ignorada
        assert!(e.backspace());
        assert_eq!(e.text, "");
    }

    #[test]
    fn setas_colar_e_apagar_no_meio() {
        let lines = |t: &str| wrap(t, 1000, &mut ten);
        let mut e = Editor::with("cafe\nbar");
        assert_eq!(e.caret, 8);
        assert!(e.move_caret(Move::Up, &lines(&e.text)));
        assert_eq!(e.caret, 3); // 3 letras para dentro da linha de cima
        e.move_caret(Move::Right, &lines(&e.text));
        assert!(e.input(0x301)); // acento combinado depois do "e"
        e.move_caret(Move::Home, &lines(&e.text));
        assert_eq!(e.caret, 0);
        e.move_caret(Move::End, &lines(&e.text));
        assert_eq!(e.caret, "cafe\u{301}".len());
        e.move_caret(Move::Left, &lines(&e.text));
        assert_eq!(e.caret, 3, "a seta pula o e com o acento junto");
        assert!(e.delete());
        assert_eq!(e.text, "caf\nbar");
        assert!(e.insert("é já\r\ncom\ttab\u{7}"));
        assert_eq!(e.text, "café já\ncom tab\nbar");
        assert!(e.backspace());
        assert_eq!(e.text, "café já\ncom ta\nbar");
        e.move_caret(Move::Down, &lines(&e.text));
        assert_eq!(e.caret, e.text.len(), "linha de baixo mais curta: vai para o fim dela");
        assert!(!e.move_caret(Move::Down, &lines(&e.text)));
        assert!(!e.move_caret(Move::Right, &lines(&e.text)));
    }

    #[test]
    fn cursor_no_meio_fica_depois_da_letra() {
        let f = Font::new(32).expect("fonte");
        let mut m = TextMask::default();
        let (_, fim) = f.update("ab\ncd", 5, 2000, &mut Laid::default(), &mut m);
        let (_, meio) = f.update("ab\ncd", 4, 2000, &mut Laid::default(), &mut m);
        let (_, cima) = f.update("ab\ncd", 2, 2000, &mut Laid::default(), &mut m);
        assert_eq!(meio.1, fim.1);
        assert!(meio.0 < fim.0 && meio.0 > pad(32));
        assert!(cima.1 < fim.1);
        assert_eq!((m.src.as_str(), m.px), ("ab\ncd", 32), "o texto fica guardado para editar depois");
    }

    /// Cada letra mede 10 px: largura do começo até cada byte.
    fn ten(s: &str, cum: &mut Vec<i32>) {
        cum.clear();
        cum.resize(s.len() + 1, 0);
        let mut n = 0;
        for (b, _) in s.char_indices() {
            cum[b] = n;
            n += 10;
        }
        cum[s.len()] = n;
    }

    /// Linear: uma medida por parágrafo, mesmo com muitas linhas.
    #[test]
    fn quebra_mede_uma_vez_por_paragrafo() {
        let t = format!("{}\n{}", "palavra ".repeat(500), "W".repeat(300));
        let mut vezes = 0;
        let lines = wrap(&t, 300, &mut |s, cum| {
            vezes += 1;
            ten(s, cum)
        });
        assert_eq!(vezes, 2);
        assert!(lines.len() > 100 && lines.iter().all(|l| l.w <= 300));
    }

    /// Digitar, quebrar, Enter e apagar refazendo só o que mudou dá a mesma
    /// cobertura que rasterizar tudo de novo.
    #[test]
    fn texto_refeito_da_linha_que_mudou_igual_ao_inteiro() {
        let f = Font::new(32).expect("fonte");
        let mut laid = Laid::default();
        let mut mask = TextMask::default();
        let mut t = String::new();
        let mut parcial = false;
        let passos: Vec<char> = "Ação na borda do céu, com cedilha ç e til ã\nÉ outra linha bem comprida que quebra".chars().collect();
        for (i, &c) in passos.iter().enumerate() {
            t.push(c);
            let (from, caret) = f.update(&t, t.len(), 400, &mut laid, &mut mask);
            let full = f.render(&t, 400);
            assert_eq!((mask.w, mask.h, caret), (full.mask.w, full.mask.h, full.caret), "passo {i}");
            assert!(mask.cov == full.mask.cov, "passo {i}: cobertura diferente");
            parcial |= from > 0;
            if i % 9 == 8 {
                t.pop();
                f.update(&t, t.len(), 400, &mut laid, &mut mask);
                assert!(mask.cov == f.render(&t, 400).mask.cov, "apagar no passo {i}");
            }
        }
        assert!(parcial, "nunca refez só um pedaço");
    }

    fn texts<'a>(t: &'a str, lines: &[Line]) -> Vec<&'a str> {
        lines.iter().map(|l| &t[l.start..l.end]).collect()
    }

    #[test]
    fn quebra_no_enter_na_palavra_e_na_letra() {
        let t = "olá mundo\n\naaaaaaaaaaaaa fim";
        let lines = wrap(t, 60, &mut ten);
        assert_eq!(texts(t, &lines), vec!["olá ", "mundo", "", "aaaaaa", "aaaaaa", "a fim"]);
        assert_eq!(lines[0].w, 30); // espaço do fim da linha quebrada não conta
        assert_eq!(lines[2].w, 0);
        assert!(lines.iter().all(|l| l.w <= 60));
        // Sem nada para quebrar: uma linha só, com a largura inteira (cursor depois do espaço).
        let one = wrap("ab ", 60, &mut ten);
        assert_eq!(one, vec![Line { start: 0, end: 3, w: 30 }]);
        assert_eq!(wrap("", 60, &mut ten), vec![Line { start: 0, end: 0, w: 0 }]);
    }

    #[test]
    fn caixa_e_cursor_seguem_as_linhas() {
        let t = "ab\nabcd";
        let lines = wrap(t, 100, &mut ten);
        let ((w, h), caret) = text_box(&lines, 20, 3, 2);
        assert_eq!((w, h), (40 + 6 + 2, 2 * 20 + 6));
        assert_eq!(caret, (3 + 40, 3 + 20));
    }

    #[test]
    fn gdi_rasteriza_com_antialias() {
        let f = Font::new(32).expect("fonte");
        assert!(f.line_h >= 32);
        assert!(f.measure("ação") > f.measure("a"));
        let r = f.render("Ação", 2000);
        let m = &r.mask;
        assert_eq!(m.cov.len(), (m.w * m.h) as usize);
        assert!(m.cov.contains(&255), "sem pixel cheio");
        assert!(m.cov.iter().any(|&c| c > 20 && c < 235), "sem borda suave (antialias)");
        // Margem limpa: a primeira linha de pixels não tem tinta.
        assert!(m.cov[..m.w as usize].iter().all(|&c| c == 0));
        // Texto vazio: caixa só do cursor, sem tinta.
        let e = f.render("", 2000);
        assert!(e.mask.cov.iter().all(|&c| c == 0));
        // Quebra por largura: a caixa fica mais alta e não passa do limite.
        let longo = f.render("uma frase comprida que precisa quebrar", 300);
        assert!(longo.mask.w <= 300 && longo.mask.h > 2 * f.line_h, "{}x{}", longo.mask.w, longo.mask.h);
    }
}
