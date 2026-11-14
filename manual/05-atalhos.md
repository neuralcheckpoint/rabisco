# Atalhos

[← Print e vídeo](04-print-e-video.md) · [Manual](README.md) · [Dúvidas e problemas →](06-duvidas.md)

## Em qualquer programa

| Atalho | Faz |
|---|---|
| `Ctrl+Shift+D` | Liga / desliga o modo desenho |
| `Ctrl+Shift+H` | Esconde / mostra os desenhos |
| `Ctrl+Shift+X` | Apaga tudo |
| `Ctrl+Shift+S` | Print da tela com os desenhos |
| `Ctrl+Shift+A` | Print de uma área (arraste um retângulo) |
| `Ctrl+Shift+F` | Começa / para a gravação de vídeo |
| `Ctrl+Shift+B` | Barra transparente: liga / desliga |
| `Ctrl+Alt+H` | Holofote: liga / desliga |
| `Ctrl+Alt+Z` | Zoom: liga / desliga |
| `Ctrl+Shift+Q` | Fecha o Rabisco |

## No modo desenho

| Tecla | Faz |
|---|---|
| `P` `M` `E` | Caneta, marca-texto, borracha |
| `L` `A` `R` `O` | Linha, seta, retângulo, elipse |
| `N` | Marcador numerado |
| `T` | Texto |
| `G` | Guias: terços → 16:9 com área segura → terços no 16:9 → desligado |
| `F` | Fade (traço some sozinho): liga / desliga |
| `S` | Holofote (`S` de novo ou `Esc` sai) |
| `Z` | Zoom (clique congela para desenhar em cima; `Z` de novo ou `Esc` sai) |
| `W` / `K` | Quadro branco / quadro preto (a mesma tecla tira) |
| `1` a `9` | Cores |
| Roda do mouse, `+` `−` | Espessura (no holofote, o tamanho do círculo; no zoom, o aumento) |
| `Shift` arrastando | Linha e seta em 45°, retângulo quadrado, elipse redonda |
| `Ctrl+Z` / `Ctrl+Y` | Desfazer / refazer |
| `C` ou `Delete` | Apagar tudo (dá para desfazer) |
| `H` | Esconder os desenhos |
| `Ctrl+S` | Print |
| `Esc` | Volta ao modo mouse |

## Com a caixa de texto aberta

| Tecla | Faz |
|---|---|
| `Enter` | Nova linha |
| `Backspace` / `Delete` | Apaga a letra antes / depois do cursor |
| Setas, `Home`, `End` | Andam com o cursor |
| `Ctrl+V` | Cola texto |
| `Esc` ou clicar fora | Confirma o texto |
| Roda do mouse | Tamanho da letra |

## Na escolha de janela ou região

| Tecla | Faz |
|---|---|
| `Esc` ou botão direito | Cancela |

## Mudar os atalhos

Os dez atalhos de "Em qualquer programa" podem ser trocados. As teclas do modo desenho são fixas.

1. Botão direito no ícone da bandeja → **Editar atalhos…**
2. Abre a janela **Atalhos do Rabisco**, com um campo por ação. Clique no campo e aperte o atalho novo. `Backspace` deixa a ação sem atalho, e **Padrões** volta todos ao original.
3. Clique em **Salvar**. O Rabisco aplica na hora, sem reiniciar.

| Pode usar | Exemplos |
|---|---|
| `Ctrl`, `Shift`, `Alt` e `Win` + uma letra, um número ou `F1` a `F24` | `Ctrl+Alt+D`, `Win+Shift+5`, `Ctrl+F9` |
| `F1` a `F24` sozinhas | `F9` |

- Letra ou número sem `Ctrl`, `Alt` ou `Win` não vale: roubaria a digitação em todos os programas. A janela avisa e não salva, e também avisa se duas ações ficarem com o mesmo atalho.
- Se outro programa já usa o atalho escolhido, o Rabisco avisa ao salvar.
- O menu da bandeja e as dicas da barra mostram sempre os atalhos atuais.

Os atalhos ficam em `%APPDATA%\Rabisco\atalhos.txt`, um por linha (`gravar = Ctrl+Alt+R`; `nenhum` desliga). Dá para editar à mão, e apagar o arquivo volta tudo ao padrão. A janela não mostra a tecla `Win`: um atalho com `Win` só entra pelo arquivo, e continua valendo enquanto o campo dele não for mexido.
