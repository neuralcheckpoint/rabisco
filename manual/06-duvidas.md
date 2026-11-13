# Dúvidas e problemas

[← Atalhos](05-atalhos.md) · [Manual](README.md) · [Por dentro →](07-por-dentro.md)

## "O Windows protegeu o computador"

Esse aviso (SmartScreen) aparece com qualquer programa novo que não tem assinatura digital paga. Clique em **Mais informações → Executar assim mesmo**. Se preferir não confiar no exe baixado, compile a partir do código: veja [Por dentro](07-por-dentro.md#compilar).

## Um atalho não funciona

Os atalhos globais (`Ctrl+Shift+…`) só funcionam se nenhum outro programa já os tiver pegado antes. Quando isso acontece, o Rabisco avisa ao abrir com a mensagem "Outro programa já usa: …" e a lista dos atalhos perdidos. Os botões da barra e o menu da bandeja continuam funcionando.

O contrário também acontece: enquanto o Rabisco está aberto, os atalhos dele valem em qualquer programa. Se algum programa seu usa `Ctrl+Shift+S`, `X` ou `B`, por exemplo, esse atalho passa a ir para o Rabisco.

Nos dois casos, troque o atalho do Rabisco por outro, ou desligue: bandeja → **Editar atalhos…** ([Mudar os atalhos](05-atalhos.md#mudar-os-atalhos)).

## Não acho o ícone da bandeja

No Windows 11, ele pode estar escondido na setinha `^` ao lado do relógio. Arraste-o para fora para deixá-lo sempre à vista.

## Cliquei no ✕ e a barra sumiu

O ✕ só guarda a barra na bandeja. Clique no ícone da bandeja para ela voltar, ou aperte `Ctrl+Shift+B` (com a barra guardada, ele só mostra a barra).

## Não quero que ele abra com o Windows

Botão direito no ícone da bandeja → desmarque **Abrir com o Windows**.

## Onde ficam os arquivos

| O quê | Onde |
|---|---|
| Prints | `Imagens\Rabisco` (e o último também em `Imagens\Rabisco\ultimo.png`) |
| Vídeos | `Vídeos\Rabisco` |
| Programa instalado | `%LOCALAPPDATA%\Programs\Rabisco` |
| Atalhos | `%APPDATA%\Rabisco\atalhos.txt` (só existe depois do primeiro **Editar atalhos…**) |
| Escolhas (barra, gravação, som) | registro do Windows, só no seu usuário |

## Os desenhos aparecem no compartilhamento de tela?

Sim: os desenhos, as guias e o holofote aparecem para quem está vendo a sua tela. A barra, a lupa do zoom, a moldura e a etiqueta da gravação não aparecem.

## Ele pesa no computador?

Quase nada. Parado ele ocupa ~2 MB de memória e não usa o processador. No modo desenho ele guarda uma imagem da tela (~55 MB numa tela 4K + 1440p) e a solta 8 s depois que você para de desenhar. A gravação roda num processo separado, que some quando ela termina.

## Achei um problema

Abra uma [issue](https://github.com/neuralcheckpoint/rabisco/issues) contando o que fez, o que esperava e o que aconteceu, com a versão do Rabisco (em **Configurações > Aplicativos**) e do Windows.
