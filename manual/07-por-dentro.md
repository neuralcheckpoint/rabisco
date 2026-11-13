# Por dentro

[← Dúvidas e problemas](06-duvidas.md) · [Manual](README.md)

Para quem quer compilar, mexer no código ou entender por que o Rabisco é leve.

## Compilar

Precisa do [Rust](https://rustup.rs) no Windows 10 ou 11.

```
cargo build --release
target\release\rabisco.exe
```

- `cargo test` roda os testes rápidos.
- `cargo test --release imagens -- --ignored` gera as imagens deste manual pelo próprio motor de desenho.
- O mesmo exe é o instalador: com o nome `Instalar-Rabisco.exe`, ou com `--instalar`, ele instala. `--desinstalar` remove.

## Como o código está dividido

| Arquivo | O que tem |
|---|---|
| `src/main.rs` | Janelas, entrada da caneta (`WM_POINTER`), atalhos, desfazer |
| `src/canvas.rs` | Traços, rasterização e composição |
| `src/toolbar.rs` | Barra flutuante e ícones (desenhados em código, sem imagens) |
| `src/bandeja.rs` | Ícone e menu da bandeja |
| `src/instalar.rs` | Instalar, desinstalar e abrir com o Windows (tudo no usuário, sem administrador) |
| `src/png.rs`, `src/imagens.rs` | Print e codificador PNG próprio |
| `src/texto.rs`, `src/guias.rs`, `src/foco.rs` | Texto, guias, holofote e zoom |
| `src/gravar.rs`, `src/gravar_gpu.rs`, `src/gravar_som.rs` | Gravação de vídeo e som |
| `src/perf.rs`, `tests/` | Suíte de performance e testes de ponta a ponta |

Rust puro com a API do Windows (`windows-sys` e, só na gravação, `windows`). Sem framework de interface e sem navegador embutido.

## Por que ele é leve

- **Uma janela transparente em camadas** (`UpdateLayeredWindowIndirect`) cobre todos os monitores. Cada movimento da caneta manda só o retângulo que mudou.
- **Os traços são vetores.** A imagem da tela só existe enquanto você desenha. No modo mouse, ou depois de 8 s parado, ela é solta, porque o Windows continua mostrando o último quadro. Por isso parado ele fica em ~2 MB.
- **Blocos de 64×64 px** guardam o traço em andamento, criados só onde a caneta passa. Desfazer e borracha refazem só a área do traço.
- **Partes pesadas em paralelo:** refazer a tela inteira e codificar o PNG do print dividem a imagem em faixas, em até 8 threads.
- **A gravação roda em outro processo** (`rabisco.exe --gravar`), que só existe enquanto grava. Ela captura pelo Windows Graphics Capture e codifica na placa de vídeo pelo Media Foundation, sem os quadros passarem pela CPU. A memória do driver vai embora junto com o processo, e uma falha do driver não derruba o Rabisco.

## Números (tela de 6400×2160, 4K + 1440p)

| O quê | Tempo |
|---|---|
| Uma amostra da caneta | ~1,4 µs |
| Refazer a tela com 200 traços | ~13 ms |
| Um quadro do fade | ~0,2 ms |
| Uma tecla no texto | ~0,3 ms |
| Print da tela inteira (PNG) | ~32 ms |

| Estado | Memória privada |
|---|---|
| Parado, no modo mouse | ~1,7 MB |
| Desenhando | ~59 MB (a imagem da tela) |
| 8 s depois de parar de desenhar | ~2-6 MB |
| Gravando (no processo da gravação) | ~120 MB em 1080p, ~230 MB em 4K |

A suíte de performance (`perf/orcamento.toml`) mede tempo, alocação e memória, e reprova se algo piorar:

```
cargo test --release --no-fail-fast perf -- --ignored --test-threads=1 --nocapture
```

## Licença

MIT. Veja [LICENSE](../LICENSE).
