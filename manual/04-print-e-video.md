# Print e vídeo

[← Apresentar e revisar](03-apresentar.md) · [Manual](README.md) · [Atalhos →](05-atalhos.md)

## Print da tela

`Ctrl+Shift+S` em qualquer programa, `Ctrl+S` no modo desenho, o disquete da barra ou **Salvar print** na bandeja.

Captura todos os monitores com os desenhos e:

- copia a imagem para a área de transferência, pronta para colar no Slack, no Discord, num e-mail ou no Photoshop;
- grava `Imagens\Rabisco\rabisco_AAAA-MM-DD_HHhMMmSS.png`;
- troca `Imagens\Rabisco\ultimo.png`, um caminho fixo para outro programa pegar sempre o último.

Um ✓ verde aparece por 1 s embaixo do cursor (um ✗ vermelho, se falhar). A barra e o aviso não entram no print. As guias entram, se estiverem ligadas.

## Print de uma área

`Ctrl+Shift+A` em qualquer programa, ou **Print de uma região** na bandeja. O monitor escurece de leve: arraste um retângulo. Ao soltar, só aquela área vai para a área de transferência e para a pasta, como o print inteiro.

`Esc`, o botão direito ou `Ctrl+Shift+A` de novo cancelam.

## Gravar a tela em vídeo

O botão **●** da barra (ou a bandeja) abre o menu:

| Item | O que grava |
|---|---|
| **Gravar o monitor** | o monitor onde está o cursor |
| **Gravar uma janela** | a janela em que você clicar |
| **Gravar uma região** | o retângulo que você arrastar |
| **Som do sistema** | o que sai nas caixas ou no fone (liga e desliga) |
| **Microfone** | a sua voz (liga e desliga) |

`Ctrl+Shift+F`, em qualquer programa, grava do último jeito escolhido no menu. Apertar de novo, ou o **■** da barra, para. Sem nada escolhido ainda: o monitor do cursor, com o som do sistema e sem microfone.

### Enquanto escolhe

- **Janela:** a tela escurece, menos a janela sob o mouse, que ganha uma moldura vermelha e uma etiqueta com o nome e o tamanho.
- **Região:** o monitor escurece e a área que você arrasta fica clara, com o tamanho em pixels.
- `Esc` ou o botão direito cancelam.

### Enquanto grava

Uma moldura vermelha cerca o que está sendo gravado e uma etiqueta **REC mm:ss** mostra o tempo. Nada disso sai no vídeo, nem a barra e a lupa. Os desenhos saem.

Gravando uma janela, a moldura acompanha a janela se você mudar ela de lugar. Fechar essa janela termina o vídeo.

### O arquivo

- Vai para `Vídeos\Rabisco\rabisco_AAAA-MM-DD_HHhMMmSS.mp4`.
- MP4 H.264, 30 quadros por segundo, com som AAC estéreo. Abre em qualquer player e sobe em qualquer site.
- A codificação usa a placa de vídeo, então a gravação quase não pesa no computador (~3% de um núcleo em 1080p).
- Ao parar, aparece um ✓ (ou um ✗, se não deu).

### Se algo der errado

- Se o Rabisco fechar à força no meio, o vídeo é fechado e salvo assim mesmo.
- Se você tirar um microfone ou fone USB no meio, o vídeo continua e aquele som fica mudo até o fim.
- Parar logo depois de começar, antes do primeiro quadro, cancela sem deixar arquivo.
