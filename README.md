# Rabisco

Anote por cima de qualquer coisa na tela do Windows: um vídeo, um programa, uma chamada de vídeo. Funciona no estilo do Epic Pen, mas é grátis e de código aberto.

![Anotações por cima de um programa](imagens/exemplo.png)

## ⬇️ Baixar

### [**Baixar o instalador (Instalar-Rabisco.exe)**](https://github.com/neuralcheckpoint/rabisco/releases/latest/download/Instalar-Rabisco.exe)

Dê dois cliques e pronto: o Rabisco é instalado só para o seu usuário (não pede administrador), ganha um atalho no menu Iniciar, passa a abrir junto com o Windows, guardado na bandeja, e aparece em **Configurações > Aplicativos** para desinstalar. Funciona no Windows 10 e 11.

Prefere sem instalar? [**Baixar rabisco.exe**](https://github.com/neuralcheckpoint/rabisco/releases/latest/download/rabisco.exe) (versão portátil, ~540 KB): é só dar dois cliques. Rodar `rabisco.exe --instalar` instala do mesmo jeito.

> **O Windows pode mostrar "O Windows protegeu o computador".** Isso aparece com qualquer programa novo que não tem assinatura digital paga. Clique em **Mais informações → Executar assim mesmo**. Se preferir, você pode compilar a partir do código (veja [Compilar](#compilar)).

Versões anteriores e notas de cada versão: [Releases](https://github.com/neuralcheckpoint/rabisco/releases).

- **Leve:** um `.exe` de ~540 KB, que é o programa e o instalador ao mesmo tempo. Parado, ocupa ~2 MB de RAM.
- **Para apresentar e revisar:** print com as anotações, guias de enquadramento, marcadores numerados, texto, traço que some sozinho (fade), holofote e zoom.
- **Grava a tela em vídeo:** monitor, janela ou região, com o som do sistema e o microfone, em MP4 H.264 pelo encoder da placa de vídeo.
- **Caneta de verdade:** usa a pressão da mesa digitalizadora, e a ponta de trás da caneta funciona como borracha.
- **Fora do caminho:** no modo mouse, o clique passa direto para o programa de baixo, e os desenhos continuam na tela.
- **Todos os monitores** ao mesmo tempo, com DPI diferente em cada um.

Feito em Rust puro com a API do Windows (`windows-sys`), sem framework de interface e sem navegador embutido.

## Como usar

Abra o Rabisco. A barra aparece na esquerda da tela, e um ícone aparece na bandeja, ao lado do relógio. No Windows 11, ele pode ficar escondido na setinha `^`: arraste-o para fora para deixá-lo sempre à vista.

**Na bandeja:** um clique no ícone mostra ou guarda a barra. O botão direito abre um menu com desenhar, esconder os desenhos, apagar tudo, **Abrir com o Windows** (liga e desliga) e sair. O ✕ da barra também guarda o Rabisco na bandeja, sem fechar.

<img src="imagens/barra.png" alt="Barra de ferramentas" width="120" align="right">

| Parte da barra | O que faz |
|---|---|
| ⠿ e ✕ no topo | Arrastar a barra / guardar na bandeja |
| Seta | **Modo mouse**: volta a clicar nos programas (os desenhos ficam) |
| Lápis | Caneta (sente a pressão) |
| Marca-texto | Traço grosso e transparente; não escurece onde passa duas vezes |
| Borracha | Apaga o traço inteiro em que você encostar |
| Linha, seta, retângulo, elipse | Formas. Segure **Shift** para travar em 45° ou deixar quadrado/círculo |
| Marcador, texto | Círculo numerado 1, 2, 3… a cada clique / clicar e digitar |
| Guias, fade | Guias de enquadramento / traço novo some sozinho |
| Bolinhas coloridas | 9 cores |
| Pontos de tamanhos diferentes | Espessura (a roda do mouse em cima da barra ajusta fino) |
| Setas curvas | Desfazer / refazer |
| Lixeira | Apagar tudo (dá para desfazer) |
| Olho | Esconder / mostrar os desenhos |
| Holofote, lupa | Escurece a tela menos um círculo no cursor / zoom em volta do cursor |
| Disquete | Salvar print |
| ● / ■ | Menu da gravação de vídeo / parar a gravação |

Escolher uma ferramenta ou uma cor liga o modo desenho. Passe o mouse em cima de um botão para ver o nome e o atalho.

<br clear="right">

### Atalhos que funcionam em qualquer programa

| Atalho | Faz |
|---|---|
| `Ctrl+Shift+D` | Liga/desliga o modo desenho |
| `Ctrl+Shift+H` | Esconde/mostra os desenhos |
| `Ctrl+Shift+X` | Apaga tudo |
| `Ctrl+Shift+S` | Salva print da tela com as anotações |
| `Ctrl+Shift+R` | Começa / para a gravação de vídeo |
| `Ctrl+Shift+Q` | Fecha o Rabisco |

### Teclas no modo desenho

| Tecla | Faz |
|---|---|
| `P` `M` `E` | Caneta, marca-texto, borracha |
| `L` `A` `R` `O` | Linha, seta, retângulo, elipse |
| `N` `T` | Marcador numerado, texto |
| `G` | Guias: terços → 16:9 com área segura → terços no 16:9 → desligado |
| `F` | Fade: liga / desliga |
| `S` `Z` | Holofote, zoom (de novo ou `Esc` sai) |
| `Ctrl+S` | Salvar print |
| `1` a `9` | Cores |
| Roda do mouse, `+` `−` | Espessura |
| `Ctrl+Z` / `Ctrl+Y` | Desfazer / refazer |
| `C` ou `Delete` | Apagar tudo |
| `H` | Esconder |
| `Esc` | Voltar ao modo mouse |

### Print
`Ctrl+S` (no modo desenho), `Ctrl+Shift+S` (em qualquer programa), o disquete da barra ou "Salvar print" na bandeja. Captura todos os monitores com as anotações e:
- copia a imagem para a área de transferência;
- grava `Imagens\Rabisco\rabisco_AAAA-MM-DD_HHhMMmSS.png`;
- atualiza `Imagens\Rabisco\ultimo.png`.

A barra não entra no print; as guias entram, se estiverem ligadas.

### Marcadores, texto e fade
- **Marcador (`N`):** cada clique põe o próximo número (o maior na tela + 1, até 99).
- **Texto (`T`):** clique e digite, na cor atual. `Enter` quebra a linha; `Esc` ou clicar fora confirma.
- **Fade (`F`):** cada traço novo fica 3 s na tela e some. Serve para apresentar: risca, fala, e a tela se limpa sozinha.

### Holofote e zoom
- **Holofote (`S`):** escurece o monitor menos um círculo em volta do cursor. A roda do mouse muda o tamanho.
- **Zoom (`Z`):** lupa 2x em volta do cursor; a roda vai de 1,5x a 4x. A lupa não aparece em gravação nem em compartilhamento de tela.

### Gravar a tela em vídeo
O botão ● da barra (ou a bandeja) abre o menu: **Gravar o monitor**, **Gravar uma janela** (clique nela), **Gravar uma região** (arraste), **Som do sistema** e **Microfone**. `Ctrl+Shift+R` grava do último jeito escolhido e para a gravação.
- Enquanto grava, um ponto vermelho com o tempo fica no canto da área gravada. Ele não sai no vídeo, nem a barra.
- O vídeo vai para `Vídeos\Rabisco\rabisco_AAAA-MM-DD_HHhMMmSS.mp4` (H.264, 30 quadros por segundo).
- Sem nada escolhido: o monitor do cursor, com o som do sistema e sem o microfone.
- Fechar a janela que está sendo gravada termina o vídeo. Se o Rabisco fechar à força, o vídeo é salvo assim mesmo.

## Compilar

Precisa do [Rust](https://rustup.rs) no Windows 10 ou 11.

```
cargo build --release
target\release\rabisco.exe
```

Testes: `cargo test`. As imagens deste manual são geradas pelo próprio motor de desenho: `cargo test --release imagens -- --ignored`.

## Como ele fica leve

- A tela é uma janela transparente em camadas (`UpdateLayeredWindowIndirect`). Cada movimento da caneta envia só o retângulo que mudou.
- Os traços são guardados como vetores. A imagem da tela só existe enquanto você desenha: no modo mouse, ou depois de 8 s parado, ela é liberada, porque o Windows continua mostrando o último quadro. Desfazer e borracha refazem só a área do traço.
- O traço em andamento usa blocos de 64×64 px, criados só onde a caneta passa.
- A gravação roda num processo separado (`rabisco.exe --gravar`), que só existe enquanto grava: captura pelo Windows Graphics Capture, codifica na placa de vídeo pelo Media Foundation, e a memória do driver vai embora junto com ele. Gravando 1080p, usa ~2% de um núcleo.
- Uma suíte de performance (`perf/orcamento.toml`) mede tempo e memória e reprova se algo piorar.

| Arquivo | O que tem |
|---|---|
| `src/main.rs` | Janelas, entrada da caneta (`WM_POINTER`), atalhos, desfazer |
| `src/canvas.rs` | Traços, rasterização e composição |
| `src/toolbar.rs` | Barra flutuante e ícones (desenhados em código, sem imagens) |
| `src/bandeja.rs` | Ícone e menu da bandeja |
| `src/instalar.rs` | Instalar, desinstalar e abrir com o Windows (tudo no usuário, sem administrador) |
| `src/png.rs`, `src/imagens.rs` | Print e codificador PNG próprio |
| `src/texto.rs`, `src/guias.rs`, `src/foco.rs` | Texto, guias, holofote e zoom |
| `src/gravar*.rs` | Gravação de vídeo e som |
| `src/perf.rs`, `tests/` | Suíte de performance e testes de ponta a ponta |

## Licença

MIT. Veja [LICENSE](LICENSE).
