//! PNG mínimo, sem dependência: RGB 8 bits, filtro "Up" e deflate com
//! Huffman fixo + LZ77 de uma tabela hash só. Telas são quase todas áreas
//! chapadas, então isso encolhe uns 20x sem o custo de um crate (o `png` +
//! `flate2` somariam centenas de KB ao exe). O `decode` lê só o que o
//! `encode` escreve (e PNG RGB sem compressão): é usado para entregar o
//! CF_DIB da área de transferência sob demanda, sem guardar 55 MB.

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

const fn make_crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[n] = c;
        n += 1;
    }
    t
}

static CRC_TABLE: [u32; 256] = make_crc_table();

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c = CRC_TABLE[((c ^ b as u32) & 255) as usize] ^ (c >> 8);
    }
    !c
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    // 5552 bytes é o máximo antes de b estourar u32: o módulo sai do laço.
    for chunk in data.chunks(5552) {
        for &v in chunk {
            a += v as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/// Adler-32 de `a ++ b` a partir dos dois (como o `adler32_combine` do zlib).
pub fn adler32_combine(a1: u32, a2: u32, len2: usize) -> u32 {
    const M: u64 = 65521;
    let rem = len2 as u64 % M;
    let s1 = (a1 as u64 & 0xFFFF) + (a2 as u64 & 0xFFFF) + M - 1;
    let s2 = rem * (a1 as u64 & 0xFFFF) % M + (a1 as u64 >> 16) + (a2 as u64 >> 16) + M - rem;
    (((s2 % M) << 16) | (s1 % M)) as u32
}

const LEN_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193,
    12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Escreve bits do menor para o maior, como o deflate pede.
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, v: u32, n: u32) {
        self.acc |= (v as u64) << self.n;
        self.n += n;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// Código de Huffman: sai do bit mais alto para o mais baixo.
    fn code(&mut self, code: u32, len: u32) {
        self.put(code.reverse_bits() >> (32 - len), len);
    }

    /// Literal ou fim de bloco (0..=287) na tabela fixa.
    fn sym(&mut self, v: u32) {
        match v {
            0..=143 => self.code(0x30 + v, 8),
            144..=255 => self.code(0x190 + v - 144, 9),
            256..=279 => self.code(v - 256, 7),
            _ => self.code(0xC0 + v - 280, 8),
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.put(0, 8 - self.n);
        }
        self.out
    }
}

/// Um pedaço do deflate: um bloco de Huffman fixo, não final, fechado com um
/// bloco "stored" vazio para terminar no byte (o sync flush do zlib). Assim os
/// pedaços de cada thread só precisam ser colados.
fn deflate_part(raw: &[u8]) -> Vec<u8> {
    let n = raw.len();
    let mut b = Bits { out: Vec::with_capacity(n / 8 + 64), acc: 0, n: 0 };
    b.put(0, 1); // não é o último
    b.put(1, 2); // Huffman fixo
    let mut head = vec![0u32; 1 << 15]; // posição + 1; 0 = vazio
    let hash = |i: usize| (u32::from_le_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]).wrapping_mul(0x9E37_79B1) >> 17) as usize;
    let mut i = 0;
    while i < n {
        let mut len = 0;
        let mut dist = 0;
        if i + 4 <= n {
            let h = hash(i);
            let cand = head[h] as usize;
            head[h] = (i + 1) as u32;
            if cand != 0 {
                let j = cand - 1;
                if i - j <= 32768 {
                    let max = (n - i).min(258);
                    let mut l = 0;
                    while l < max && raw[j + l] == raw[i + l] {
                        l += 1;
                    }
                    if l >= 4 {
                        (len, dist) = (l, i - j);
                    }
                }
            }
        }
        if len == 0 {
            b.sym(raw[i] as u32);
            i += 1;
            continue;
        }
        let lc = LEN_BASE.partition_point(|&x| x as usize <= len) - 1;
        b.sym(257 + lc as u32);
        b.put((len - LEN_BASE[lc] as usize) as u32, LEN_EXTRA[lc] as u32);
        let dc = DIST_BASE.partition_point(|&x| x as usize <= dist) - 1;
        b.code(dc as u32, 5);
        b.put((dist - DIST_BASE[dc] as usize) as u32, DIST_EXTRA[dc] as u32);
        for p in i + 1..(i + len).min(n.saturating_sub(3)) {
            head[hash(p)] = (p + 1) as u32;
        }
        i += len;
    }
    b.sym(256);
    b.put(0, 3); // "stored", não final: alinha no byte
    let mut z = b.finish();
    z.extend([0, 0, 0xFF, 0xFF]);
    z
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

/// Threads do `encode` (a captura roda fora da janela, numa thread só dela).
const PNG_THREADS: usize = 8;

/// `px`: w*h pixels 0x??RRGGBB de cima para baixo (o alfa é ignorado).
pub fn encode(w: usize, h: usize, px: &[u32]) -> Vec<u8> {
    // Faixa de pelo menos 64 linhas: a perda de compressão na emenda é mínima.
    let parts = std::thread::available_parallelism().map_or(1, |n| n.get()).min(PNG_THREADS).min(h.div_ceil(64)).max(1);
    encode_in(w, h, px, parts)
}

/// Filtro + deflate em `parts` faixas de linhas, em paralelo. O LZ77 não cruza
/// a faixa; o filtro Up olha a linha de cima mesmo na emenda.
fn encode_in(w: usize, h: usize, px: &[u32], parts: usize) -> Vec<u8> {
    let rows = h.div_ceil(parts.max(1)).max(1);
    let n = h.div_ceil(rows);
    let slots: Vec<std::sync::Mutex<(Vec<u8>, u32, usize)>> = (0..n).map(|_| Default::default()).collect();
    crate::canvas::par(n, &|i| {
                    *slots[i].lock().unwrap() = {
                    let (y0, y1) = (i * rows, ((i + 1) * rows).min(h));
                    let mut raw = Vec::with_capacity((w * 3 + 1) * (y1 - y0));
                    let rgb = |p: u32| [(p >> 16) as u8, (p >> 8) as u8, p as u8];
                    for y in y0..y1 {
                        raw.push(2); // filtro Up: linhas iguais viram zeros
                        for x in 0..w {
                            let cur = rgb(px[y * w + x]);
                            let up = if y > 0 { rgb(px[(y - 1) * w + x]) } else { [0; 3] };
                            raw.extend([cur[0].wrapping_sub(up[0]), cur[1].wrapping_sub(up[1]), cur[2].wrapping_sub(up[2])]);
                        }
                    }
                    (deflate_part(&raw), adler32(&raw), raw.len())
                    }
    });
    let strips: Vec<_> = slots.into_iter().map(|m| m.into_inner().unwrap()).collect();
    let mut z = vec![0x78, 0x01];
    let mut adler = 1;
    for (d, a, n) in &strips {
        z.extend(d);
        adler = adler32_combine(adler, *a, *n);
    }
    z.extend([0x03, 0x00]); // último bloco: Huffman fixo vazio (só o fim de bloco)
    z.extend(adler.to_be_bytes());
    let mut out = SIGNATURE.to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend((w as u32).to_be_bytes());
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]); // 8 bits, RGB, sem entrelaçar
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

// ---------- leitura (só o que o encode escreve) ----------

/// Tabela da Huffman fixa lida do jeito que os bits chegam (do menor para o
/// maior): os 9 próximos bits dão (símbolo, tamanho do código).
const fn make_fixed_table() -> [(u16, u8); 512] {
    let mut t = [(0u16, 0u8); 512];
    let mut sym = 0;
    while sym < 288 {
        let (code, len) = match sym {
            0..=143 => (0x30 + sym, 8),
            144..=255 => (0x190 + sym - 144, 9),
            256..=279 => (sym - 256, 7),
            _ => (0xC0 + sym - 280, 8),
        };
        let rev = (code as u32).reverse_bits() >> (32 - len);
        let mut hi = 0;
        while hi < 1 << (9 - len) {
            t[(rev | hi << len) as usize] = (sym as u16, len as u8);
            hi += 1;
        }
        sym += 1;
    }
    t
}

static FIXED: [(u16, u8); 512] = make_fixed_table();

struct Reader<'a> {
    d: &'a [u8],
    pos: usize, // em bits
}

impl Reader<'_> {
    /// Os próximos 57+ bits (zeros depois do fim), sem andar.
    #[inline]
    fn peek(&self) -> u64 {
        let i = self.pos >> 3;
        let v = match self.d.get(i..i + 8) {
            Some(b) => u64::from_le_bytes(b.try_into().unwrap()),
            None => {
                let mut b = [0u8; 8];
                let tail = self.d.get(i..).unwrap_or(&[]);
                b[..tail.len()].copy_from_slice(tail);
                u64::from_le_bytes(b)
            }
        };
        v >> (self.pos & 7)
    }

    /// Anda n bits; None se passou do fim.
    #[inline]
    fn skip(&mut self, n: u32) -> Option<()> {
        self.pos += n as usize;
        (self.pos <= self.d.len() * 8).then_some(())
    }

    #[inline]
    fn bits(&mut self, n: u32) -> Option<u32> {
        let v = (self.peek() & ((1u64 << n) - 1)) as u32;
        self.skip(n)?;
        Some(v)
    }

    fn bit(&mut self) -> Option<u32> {
        self.bits(1)
    }

    /// Próximo símbolo da tabela fixa (0..=287).
    #[inline]
    fn sym(&mut self) -> Option<u32> {
        let (s, len) = FIXED[(self.peek() & 511) as usize];
        self.skip(len as u32)?;
        Some(s as u32)
    }
}

fn inflate(z: &[u8], size_hint: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(size_hint);
    let mut r = Reader { d: z, pos: 0 };
    loop {
        let last = r.bit()?;
        match r.bits(2)? {
            0 => {
                r.pos = (r.pos + 7) & !7;
                let at = r.pos >> 3;
                let len = u16::from_le_bytes([*z.get(at)?, *z.get(at + 1)?]) as usize;
                out.extend(z.get(at + 4..at + 4 + len)?);
                r.pos += (4 + len) * 8;
            }
            1 => loop {
                let s = r.sym()?;
                match s {
                    0..=255 => out.push(s as u8),
                    256 => break,
                    _ => {
                        let lc = (s - 257) as usize;
                        let len = *LEN_BASE.get(lc)? as usize + r.bits(*LEN_EXTRA.get(lc)? as u32)? as usize;
                        // Código de distância: 5 bits do mais alto para o mais baixo.
                        let dc = (r.bits(5)?.reverse_bits() >> 27) as usize;
                        let dist = *DIST_BASE.get(dc)? as usize + r.bits(*DIST_EXTRA.get(dc)? as u32)? as usize;
                        if dist > out.len() {
                            return None;
                        }
                        // Pode se sobrepor (dist < len): copia em pedaços de até dist.
                        let from = out.len() - dist;
                        let mut done = 0;
                        while done < len {
                            let n = (len - done).min(dist + done);
                            out.extend_from_within(from..from + n);
                            done += n;
                        }
                    }
                }
            },
            _ => return None, // Huffman dinâmico: não é nosso
        }
        if last == 1 {
            return Some(out);
        }
    }
}

/// Devolve (largura, altura, pixels 0xFFRRGGBB de cima para baixo).
pub fn decode(png: &[u8]) -> Option<(usize, usize, Vec<u32>)> {
    if png.get(..8)? != SIGNATURE {
        return None;
    }
    let (mut pos, mut dims, mut idat) = (8, None, Vec::new());
    while pos + 12 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().ok()?) as usize;
        let kind = &png[pos + 4..pos + 8];
        let data = png.get(pos + 8..pos + 8 + len)?;
        let crc = u32::from_be_bytes(png.get(pos + 8 + len..pos + 12 + len)?.try_into().ok()?);
        if crc != crc32(&png[pos + 4..pos + 8 + len]) {
            return None;
        }
        match kind {
            b"IHDR" if len == 13 && data[8..] == [8, 2, 0, 0, 0] => {
                let w = u32::from_be_bytes(data[0..4].try_into().ok()?) as usize;
                let h = u32::from_be_bytes(data[4..8].try_into().ok()?) as usize;
                dims = Some((w, h));
            }
            b"IDAT" => idat.extend(data),
            _ => {}
        }
        pos += 12 + len;
    }
    let (w, h) = dims?;
    let stride = w * 3;
    let raw = inflate(idat.get(2..)?, (stride + 1) * h)?;
    if raw.len() != (stride + 1) * h {
        return None;
    }
    let mut img = vec![0u8; stride * h];
    for y in 0..h {
        let (filter, line) = (raw[y * (stride + 1)], &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)]);
        let (above, rest) = img.split_at_mut(y * stride);
        let cur = &mut rest[..stride];
        match filter {
            0 => cur.copy_from_slice(line),
            1 => {
                for x in 0..stride {
                    cur[x] = line[x].wrapping_add(if x >= 3 { cur[x - 3] } else { 0 });
                }
            }
            2 if y > 0 => {
                for ((c, &l), &u) in cur.iter_mut().zip(line).zip(&above[(y - 1) * stride..]) {
                    *c = l.wrapping_add(u);
                }
            }
            2 => cur.copy_from_slice(line),
            _ => return None,
        }
    }
    let px = img.chunks_exact(3).map(|c| 0xFF00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32).collect();
    Some((w, h, px))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Imagem com áreas chapadas, degradê e ruído (o pior caso para o LZ).
    fn amostra(w: usize, h: usize) -> Vec<u32> {
        let mut seed = 12345u32;
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let v = match (x * 4 / w, y * 3 / h) {
                    (0, _) => 0x202428,
                    (1, 0) => ((x * 255 / w) as u32) << 16 | ((y * 255 / h) as u32) << 8 | 0x40,
                    (2, _) => seed >> 8,
                    _ => 0xE0E0E0,
                };
                0xFF00_0000 | (v & 0xFF_FFFF)
            })
            .collect()
    }

    #[test]
    fn crc_e_adler_conhecidos() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(&vec![255u8; 100_000]), {
            let (mut a, mut b) = (1u32, 0u32);
            for _ in 0..100_000 {
                a = (a + 255) % 65521;
                b = (b + a) % 65521;
            }
            (b << 16) | a
        });
    }

    #[test]
    fn ida_e_volta_sem_perda() {
        for (w, h) in [(1, 1), (3, 2), (37, 23), (300, 200), (1000, 7)] {
            let px = amostra(w, h);
            let (w2, h2, back) = decode(&encode(w, h, &px)).expect("decodifica");
            assert_eq!((w2, h2), (w, h));
            assert_eq!(back, px, "{w}x{h}");
        }
    }

    /// O zlib do IDAT: inflado e com o Adler-32 do fim batendo.
    fn idat_confere(png: &[u8]) -> Vec<u8> {
        let len = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        let z = &png[41..41 + len];
        let raw = inflate(&z[2..z.len() - 4], 0).expect("inflate");
        assert_eq!(u32::from_be_bytes(z[z.len() - 4..].try_into().unwrap()), adler32(&raw));
        raw
    }

    #[test]
    fn adler_emendado() {
        let a: Vec<u8> = (0..70_000u32).map(|i| (i * 7 % 251) as u8).collect();
        for cut in [0, 1, 5552, 40_000, 70_000] {
            assert_eq!(adler32_combine(adler32(&a[..cut]), adler32(&a[cut..]), a.len() - cut), adler32(&a), "{cut}");
        }
    }

    #[test]
    fn faixas_em_paralelo_dao_a_mesma_imagem() {
        for (w, h) in [(1, 1), (37, 23), (300, 200), (1000, 7)] {
            let px = amostra(w, h);
            let one = idat_confere(&encode_in(w, h, &px, 1));
            for parts in [2, 3, 8, 50] {
                let png = encode_in(w, h, &px, parts);
                assert_eq!(idat_confere(&png), one, "{w}x{h} em {parts}");
                assert_eq!(decode(&png).unwrap().2, px);
            }
        }
    }

    #[test]
    fn todos_os_blocos_tem_crc_valido_e_a_estrutura_certa() {
        let png = encode(50, 40, &amostra(50, 40));
        assert_eq!(&png[..8], SIGNATURE);
        let (mut pos, mut kinds) = (8, Vec::new());
        while pos < png.len() {
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
            let crc = u32::from_be_bytes(png[pos + 8 + len..pos + 12 + len].try_into().unwrap());
            assert_eq!(crc, crc32(&png[pos + 4..pos + 8 + len]));
            kinds.push(String::from_utf8_lossy(&png[pos + 4..pos + 8]).into_owned());
            pos += 12 + len;
        }
        assert_eq!(kinds, ["IHDR", "IDAT", "IEND"]);
        // O CRC do decode pega bloco corrompido.
        let mut ruim = png.clone();
        ruim[40] ^= 1;
        assert!(decode(&ruim).is_none());
    }

    #[test]
    fn tela_chapada_encolhe_muito() {
        let (w, h) = (1280, 720);
        let mut px = vec![0xFF1E2024u32; w * h];
        for y in 100..300 {
            px[y * w + 100..y * w + 600].fill(0xFFFFFFFF);
        }
        let png = encode(w, h, &px);
        assert!(png.len() < w * h * 3 / 100, "{} bytes", png.len());
        assert_eq!(decode(&png).unwrap().2, px);
    }

    #[test]
    fn le_filtros_sub_e_up() {
        // Linha 0 com Sub, linha 1 com Up, linha 2 com Sub, em bloco "stored".
        let (w, h) = (4usize, 3usize);
        let px: Vec<u32> = (0..12u32).map(|i| 0xFF00_0000 | (i * 0x0B1D29).wrapping_mul(2654435761) & 0xFF_FFFF).collect();
        let rgb = |x: usize, y: usize| {
            let p = px[y * w + x];
            [(p >> 16) as u8, (p >> 8) as u8, p as u8]
        };
        let mut raw = Vec::new();
        for (y, f) in [(0, 1u8), (1, 2), (2, 1)] {
            raw.push(f);
            for x in 0..w {
                let (c, prev) = (rgb(x, y), if f == 1 { if x > 0 { rgb(x - 1, y) } else { [0; 3] } } else { rgb(x, y - 1) });
                raw.extend((0..3).map(|k| c[k].wrapping_sub(prev[k])));
            }
        }
        let mut z = vec![0x78, 0x01, 1];
        z.extend((raw.len() as u16).to_le_bytes());
        z.extend((!(raw.len() as u16)).to_le_bytes());
        z.extend(&raw);
        z.extend(adler32(&raw).to_be_bytes());
        let mut out = SIGNATURE.to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend((w as u32).to_be_bytes());
        ihdr.extend((h as u32).to_be_bytes());
        ihdr.extend([8, 2, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        assert_eq!(decode(&out).unwrap().2, px);
    }

    #[test]
    fn le_png_sem_compressao() {
        // O formato antigo do manual: bloco "stored", filtro 0.
        let (w, h) = (3usize, 2usize);
        let px: Vec<u32> = (0..6).map(|i| 0xFF00_0000 | i * 0x101010).collect();
        let mut raw = Vec::new();
        for y in 0..h {
            raw.push(0);
            for x in 0..w {
                let p = px[y * w + x];
                raw.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
        }
        let mut z = vec![0x78, 0x01, 1];
        z.extend((raw.len() as u16).to_le_bytes());
        z.extend((!(raw.len() as u16)).to_le_bytes());
        z.extend(&raw);
        z.extend(adler32(&raw).to_be_bytes());
        let mut out = SIGNATURE.to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend((w as u32).to_be_bytes());
        ihdr.extend((h as u32).to_be_bytes());
        ihdr.extend([8, 2, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        assert_eq!(decode(&out).unwrap().2, px);
    }
}
