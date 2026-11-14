# Rabisco

Anote por cima de qualquer coisa na tela do Windows: um vídeo, um programa, uma chamada de vídeo. Funciona no estilo do Epic Pen, mas é grátis e de código aberto.

![Anotações por cima de um programa](imagens/exemplo.png)

## ⬇️ Baixar

### [**Baixar o Rabisco (Rabisco.zip)**](https://github.com/neuralcheckpoint/rabisco/releases/latest/download/Rabisco.zip)

Extraia o zip e dê dois cliques no `Instalar-Rabisco.exe`, e pronto: o Rabisco é instalado só para o seu usuário (não pede administrador), ganha um atalho no menu Iniciar e passa a abrir junto com o Windows, guardado na bandeja. Funciona no Windows 10 e 11.

Prefere sem instalar? Use o `rabisco.exe` do mesmo zip (versão portátil, ~580 KB).

> **O Windows pode mostrar "O Windows protegeu o computador".** Isso aparece com qualquer programa novo sem assinatura digital paga. Clique em **Mais informações → Executar assim mesmo**.

Notas de cada versão: [Releases](https://github.com/neuralcheckpoint/rabisco/releases).

## 📖 [Manual](manual/README.md)

| | |
|---|---|
| [Instalar e começar](manual/01-comecar.md) | A barra, a bandeja, atualizar e desinstalar |
| [Desenhar](manual/02-desenhar.md) | Ferramentas, cores, espessura, caneta com pressão |
| [Apresentar e revisar](manual/03-apresentar.md) | Marcadores numerados, texto, fade, guias, holofote, zoom |
| [Print e vídeo](manual/04-print-e-video.md) | Print da tela ou de uma área, gravação com som |
| [Atalhos](manual/05-atalhos.md) | Todos numa página só, e como trocar |
| [Dúvidas e problemas](manual/06-duvidas.md) | Aviso do Windows, atalho que não funciona, onde ficam os arquivos |
| [Por dentro](manual/07-por-dentro.md) | Compilar, o código e por que ele é leve |

## O que ele faz

- **Desenha por cima de tudo:** caneta, marca-texto, linha, seta, retângulo, elipse e borracha, em todos os monitores.
- **Fora do caminho:** no modo mouse, o clique passa direto para o programa de baixo, e os desenhos continuam na tela.
- **Caneta de verdade:** sente a pressão da mesa digitalizadora, e a ponta de trás da caneta apaga.
- **Para apresentar e revisar:** marcadores numerados, texto, traço que some sozinho (fade), guias de enquadramento, holofote e zoom.
- **Print com os desenhos:** da tela inteira ou de uma área, direto para a área de transferência.
- **Grava a tela em vídeo:** monitor, janela ou região, com som do sistema e microfone, em MP4 pela placa de vídeo.
- **Leve:** um `.exe` de ~550 KB, que é o programa e o instalador ao mesmo tempo. Parado, ocupa ~2 MB de RAM.

## Atalhos principais

| Atalho | Faz |
|---|---|
| `Ctrl+Shift+D` | Liga/desliga o modo desenho |
| `Esc` | Volta ao modo mouse (os desenhos ficam) |
| `Ctrl+Shift+X` | Apaga tudo |
| `Ctrl+Shift+S` | Print da tela com os desenhos |
| `Ctrl+Shift+A` | Print de uma área |
| `Ctrl+Shift+F` | Começa / para a gravação de vídeo |

Dá para trocar qualquer um deles: bandeja → **Editar atalhos…** ([como](manual/05-atalhos.md#mudar-os-atalhos)).

[Todos os atalhos →](manual/05-atalhos.md)

## Compilar

Precisa do [Rust](https://rustup.rs) no Windows 10 ou 11.

```
cargo build --release
```

Mais em [Por dentro](manual/07-por-dentro.md). Feito em Rust puro com a API do Windows, sem framework de interface e sem navegador embutido.

## Licença

MIT. Veja [LICENSE](LICENSE).
