//! Atalhos globais configuráveis, em `%APPDATA%\Rabisco\atalhos.txt`: uma
//! linha `nome = Ctrl+Shift+D` por ação. Linha ausente ou errada usa o padrão;
//! `nenhum` desliga o atalho.

use std::path::PathBuf;

/// Ordem fixa: o índice é a ação (o `main` liga cada índice ao id do RegisterHotKey).
pub const ACTIONS: [(&str, &str, &str); 8] = [
    ("desenhar", "Ctrl+Shift+D", "liga / desliga o modo desenho"),
    ("esconder", "Ctrl+Shift+H", "esconde / mostra os desenhos"),
    ("apagar", "Ctrl+Shift+X", "apaga tudo"),
    ("print", "Ctrl+Shift+S", "print da tela com os desenhos"),
    ("print_area", "Ctrl+Shift+A", "print de uma área"),
    ("gravar", "Ctrl+Shift+F", "começa / para a gravação"),
    ("barra", "Ctrl+Shift+B", "barra transparente (guardada: mostra a barra)"),
    ("sair", "Ctrl+Shift+Q", "fecha o Rabisco"),
];
pub const DRAW: usize = 0;
pub const HIDE: usize = 1;
pub const CLEAR: usize = 2;
pub const SAVE: usize = 3;
pub const PRINT_REG: usize = 4;
pub const REC: usize = 5;
pub const BAR: usize = 6;
pub const QUIT: usize = 7;

// Mesmos valores de MOD_ALT/CONTROL/SHIFT/WIN do RegisterHotKey.
const MODS: [(&str, u32); 4] = [("Ctrl", 2), ("Shift", 4), ("Alt", 1), ("Win", 8)];

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Key {
    pub mods: u32,
    pub vk: u32,
}

pub struct Atalhos {
    pub keys: [Option<Key>; 8],
    /// Linhas que não deu para usar (vão no aviso ao abrir).
    pub bad: Vec<String>,
}

pub fn path() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("Rabisco").join("atalhos.txt")
}

/// Lê o arquivo; sem arquivo, os padrões.
pub fn load() -> Atalhos {
    parse(&std::fs::read_to_string(path()).unwrap_or_default())
}

/// Cria o arquivo com os valores atuais (se ainda não existe) e devolve o caminho.
pub fn ensure_file() -> std::io::Result<PathBuf> {
    let p = path();
    if !p.exists() {
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(&p, template())?;
    }
    Ok(p)
}

fn template() -> String {
    let mut s = String::from(
        "# Atalhos do Rabisco, valem em qualquer programa.\r\n\
         # Troque o que vem depois do =, salve e feche o Bloco de Notas: o Rabisco aplica na hora.\r\n\
         # Teclas: Ctrl, Shift, Alt e Win + uma letra, um número ou F1 a F24 (ex.: Ctrl+Alt+D, F9).\r\n\
         # Letra ou número sem Ctrl/Alt/Win não vale. \"nenhum\" desliga o atalho.\r\n\r\n",
    );
    for (name, def, what) in ACTIONS {
        s += &format!("# {what}\r\n{name} = {def}\r\n\r\n");
    }
    s
}

pub fn parse(text: &str) -> Atalhos {
    // O Bloco de Notas pode salvar em "UTF-8 com BOM".
    let text = text.trim_start_matches('\u{feff}');
    let mut keys = ACTIONS.map(|(_, def, _)| parse_key(def));
    let mut bad = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let parsed = line.split_once('=').and_then(|(n, v)| {
            let i = ACTIONS.iter().position(|a| a.0.eq_ignore_ascii_case(n.trim()))?;
            let v = v.trim().trim_matches('"');
            Some((i, if v.eq_ignore_ascii_case("nenhum") { None } else { Some(parse_key(v)?) }))
        });
        match parsed {
            Some((i, k)) => keys[i] = k,
            None => bad.push(line.to_string()),
        }
    }
    // Dois com a mesma tecla: fica o primeiro (o RegisterHotKey recusaria o segundo).
    for i in 0..keys.len() {
        if keys[i].is_some() && keys[..i].contains(&keys[i]) {
            bad.push(format!("{} = {} (repetido)", ACTIONS[i].0, label(keys[i])));
            keys[i] = None;
        }
    }
    Atalhos { keys, bad }
}

/// "Ctrl+Shift+D" → Key. Aceita minúsculas e espaços.
pub fn parse_key(s: &str) -> Option<Key> {
    let mut mods = 0;
    let mut vk = None;
    for part in s.split('+').map(str::trim) {
        if let Some(&(_, m)) = MODS.iter().find(|(n, _)| n.eq_ignore_ascii_case(part) || (part.eq_ignore_ascii_case("Control") && *n == "Ctrl")) {
            mods |= m;
            continue;
        }
        if vk.is_some() {
            return None;
        }
        let up = part.to_ascii_uppercase();
        vk = Some(match up.as_bytes() {
            [c @ (b'A'..=b'Z' | b'0'..=b'9')] => *c as u32,
            [b'F', ..] => match up[1..].parse::<u32>() {
                Ok(n @ 1..=24) => 0x6F + n, // VK_F1 = 0x70
                _ => return None,
            },
            _ => return None,
        });
    }
    let vk = vk?;
    // Letra/número sozinho (ou só com Shift) roubaria a digitação em todo programa.
    if vk < 0x70 && mods & !4 == 0 {
        return None;
    }
    Some(Key { mods, vk })
}

/// Texto do atalho para menus e dicas ("" = desligado).
pub fn label(k: Option<Key>) -> String {
    let Some(k) = k else { return String::new() };
    let mut s: Vec<String> = MODS.iter().filter(|(_, m)| k.mods & m != 0).map(|(n, _)| n.to_string()).collect();
    s.push(if k.vk >= 0x70 { format!("F{}", k.vk - 0x6F) } else { char::from(k.vk as u8).to_string() });
    s.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padroes_sem_arquivo() {
        let a = parse("");
        assert!(a.bad.is_empty());
        for (i, (_, def, _)) in ACTIONS.iter().enumerate() {
            assert_eq!(label(a.keys[i]), *def);
        }
    }

    #[test]
    fn modelo_le_igual_aos_padroes() {
        let a = parse(&template());
        assert!(a.bad.is_empty(), "{:?}", a.bad);
        assert_eq!(a.keys, parse("").keys);
    }

    #[test]
    fn troca_desliga_e_recusa() {
        let a = parse("\u{feff}gravar = ctrl + alt + r\nSAIR = nenhum\nbarra = \"F9\"\nprint = S\napagar = Ctrl+Shift+F25\nxis = Ctrl+K\nsem igual\n");
        assert_eq!(label(a.keys[REC]), "Ctrl+Alt+R");
        assert_eq!(a.keys[QUIT], None);
        assert_eq!(label(a.keys[BAR]), "F9");
        // Recusadas ficam no padrão.
        assert_eq!(label(a.keys[SAVE]), "Ctrl+Shift+S");
        assert_eq!(label(a.keys[CLEAR]), "Ctrl+Shift+X");
        assert_eq!(a.bad.len(), 4, "{:?}", a.bad);
        assert_eq!(parse_key("Shift+A"), None);
        assert_eq!(parse_key("Ctrl+A+B"), None);
        assert_eq!(parse_key("Win+1"), Some(Key { mods: 8, vk: b'1' as u32 }));
    }

    #[test]
    fn repetido_fica_com_o_primeiro() {
        let a = parse("esconder = Ctrl+Shift+D");
        assert_eq!(a.keys[DRAW].is_some(), true);
        assert_eq!(a.keys[HIDE], None);
        assert_eq!(a.bad.len(), 1);
    }
}
