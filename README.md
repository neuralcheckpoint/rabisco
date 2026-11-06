# Rabisco

Anote por cima de qualquer coisa na tela do Windows: um vídeo, um programa, uma chamada de vídeo. Funciona no estilo do Epic Pen, mas é grátis e de código aberto.

![Anotações por cima de um programa](imagens/exemplo.png)

## ⬇️ Baixar

### [**Baixar o instalador (Instalar-Rabisco.exe)**](https://github.com/neuralcheckpoint/rabisco/releases/latest/download/Instalar-Rabisco.exe)

Dê dois cliques e pronto: o Rabisco é instalado só para o seu usuário (não pede administrador), ganha um atalho no menu Iniciar, passa a abrir junto com o Windows, guardado na bandeja, e aparece em **Configurações > Aplicativos** para desinstalar. Funciona no Windows 10 e 11.

Prefere sem instalar? [**Baixar rabisco.exe**](https://github.com/neuralcheckpoint/rabisco/releases/latest/download/rabisco.exe) (versão portátil, ~340 KB): é só dar dois cliques. Rodar `rabisco.exe --instalar` instala do mesmo jeito.

> **O Windows pode mostrar "O Windows protegeu o computador".** Isso aparece com qualquer programa novo que não tem assinatura digital paga. Clique em **Mais informações → Executar assim mesmo**. Se preferir, você pode compilar a partir do código (veja [Compilar](#compilar)).

Versões anteriores e notas de cada versão: [Releases](https://github.com/neuralcheckpoint/rabisco/releases).

- **Leve:** um `.exe` de ~340 KB, que é o programa e o instalador ao mesmo tempo. Parado, ocupa ~2 MB de RAM.
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
| Bolinhas coloridas | 9 cores |
| Pontos de tamanhos diferentes | Espessura (a roda do mouse em cima da barra ajusta fino) |
| Setas curvas | Desfazer / refazer |
| Lixeira | Apagar tudo (dá para desfazer) |
| Olho | Esconder / mostrar os desenhos |

Escolher uma ferramenta ou uma cor liga o modo desenho. Passe o mouse em cima de um botão para ver o nome e o atalho.

<br clear="right">

### Atalhos que funcionam em qualquer programa

| Atalho | Faz |
|---|---|
| `Ctrl+Shift+D` | Liga/desliga o modo desenho |
| `Ctrl+Shift+H` | Esconde/mostra os desenhos |
| `Ctrl+Shift+X` | Apaga tudo |
| `Ctrl+Shift+Q` | Fecha o Rabisco |

### Teclas no modo desenho

| Tecla | Faz |
|---|---|
| `P` `M` `E` | Caneta, marca-texto, borracha |
| `L` `A` `R` `O` | Linha, seta, retângulo, elipse |
| `1` a `9` | Cores |
| Roda do mouse, `+` `−` | Espessura |
| `Ctrl+Z` / `Ctrl+Y` | Desfazer / refazer |
| `C` ou `Delete` | Apagar tudo |
| `H` | Esconder |
| `Esc` | Voltar ao modo mouse |

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

| Arquivo | O que tem |
|---|---|
| `src/main.rs` | Janelas, entrada da caneta (`WM_POINTER`), atalhos, desfazer |
| `src/canvas.rs` | Traços, rasterização e composição |
| `src/toolbar.rs` | Barra flutuante e ícones (desenhados em código, sem imagens) |
| `src/bandeja.rs` | Ícone e menu da bandeja |
| `src/instalar.rs` | Instalar, desinstalar e abrir com o Windows (tudo no usuário, sem administrador) |

## Licença

MIT. Veja [LICENSE](LICENSE).
