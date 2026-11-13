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
| `Z` | Zoom (`Z` de novo ou `Esc` sai) |
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
| `Backspace` | Apaga a última letra |
| `Esc` ou clicar fora | Confirma o texto |
| Roda do mouse | Tamanho da letra |

## Na escolha de janela ou região

| Tecla | Faz |
|---|---|
| `Esc` ou botão direito | Cancela |

## Mudar os atalhos

Os oito atalhos de "Em qualquer programa" podem ser trocados. As teclas do modo desenho são fixas.

1. Botão direito no ícone da bandeja → **Editar atalhos…**
2. O Bloco de Notas abre o arquivo `atalhos.txt` com os atalhos atuais:

   ```
   desenhar = Ctrl+Shift+D
   gravar = Ctrl+Shift+F
   ...
   ```

3. Troque o que vem depois do `=`, salve e feche o Bloco de Notas. O Rabisco aplica na hora, sem reiniciar.

| Pode usar | Exemplos |
|---|---|
| `Ctrl`, `Shift`, `Alt` e `Win` + uma letra, um número ou `F1` a `F24` | `Ctrl+Alt+D`, `Win+Shift+5`, `Ctrl+F9` |
| `F1` a `F24` sozinhas | `F9` |
| `nenhum` | desliga aquele atalho |

- Letra ou número sem `Ctrl`, `Alt` ou `Win` não vale: roubaria a digitação em todos os programas.
- Linha errada, ou atalho repetido em duas ações, é avisada ao aplicar e fica com o padrão (ou desligada, no caso da repetida).
- Apagar o arquivo volta tudo ao padrão.
- O menu da bandeja e as dicas da barra mostram sempre os atalhos atuais.

O arquivo fica em `%APPDATA%\Rabisco\atalhos.txt`.
