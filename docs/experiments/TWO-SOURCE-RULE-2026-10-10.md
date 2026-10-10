# Regra das duas fontes — 10 de outubro de 2026

Branch `answer-eval-two-source`, criada a partir de `restructure` em `1d09bfa`.
Trabalho feito numa worktree separada porque a pasta principal tinha
alterações não commitadas de outra sessão (gate de completude), que não
foram tocadas nem incluídas aqui.

## Pergunta

Com uma fonte por caso (SQuAD 2.0), o caminho de resposta ainda respondia a
11% das perguntas sem resposta. Quanto a regra do produto, que exige duas
famílias de fonte para apresentar uma afirmação como resposta, muda isso? E
quanto custa em perguntas que têm resposta?

## Dados

MS MARCO QA v2.1, split de validação (101.093 consultas). Cada linha é uma
consulta real do Bing com cerca de dez trechos de páginas e, ou respostas
escritas por anotadores, ou o marcador "No Answer Present." Foram baixadas
dez páginas de 100 linhas em offsets derivados do hash do rótulo; delas, 200
casos, 100 com resposta e 100 sem, sorteados pelo hash do id. Consultas já
presentes em `bench/fixtures/` (879) foram excluídas. Os dados ficam em
`bench/datasets/` da pasta principal, fora do git; o MS MARCO é licenciado
só para pesquisa não comercial.

- Linhas: sha256 `7143d751…74c0`. Casos: sha256 `c13ec469…0e0d`.
- Rótulo da amostra: `msmarco-qa-two-source-2026-10-10-a`.
- Mediana de 10 trechos por caso; a maioria dos trechos com resposta vem de
  um site só: os anotadores marcaram dois ou mais trechos em 6 de 100 casos.

## Execução

`claude-haiku-5.5` via OpenRouter, código do commit `632b424`, o modelo lendo
os dez trechos (`--max-passages 10`). Uma passada do modelo por caso; as
linhas abaixo são montagens diferentes das mesmas saídas.

| Montagem | Com resposta: respondidas | Dessas, citando o trecho dos anotadores | Sem resposta: respondidas |
|---|---|---|---|
| Uma fonte basta | 57 de 100 | 49 | 28 de 100 |
| **Duas famílias (regra do produto)** | **24 de 100** | 23 | **6 de 100** |

Dentro da regra das duas famílias, as checagens de responsividade levaram as
respostas indevidas de 9 (só stance) para 6.

A comparação por texto com a resposta de referência não serve aqui: as
respostas do MS MARCO são frases livres, e só 5 a 9 respostas bateram
literalmente. A coluna "citando o trecho dos anotadores" é a medida de
acerto usada: 49 de 57 e 23 de 24.

Outros números da mesma passada:

- Nas 100 com resposta: em 20 o extrator não propôs afirmação; 8 foram lidas
  como exploração e 1 como destino.
- Afirmações nas perguntas com resposta: 67 com uma fonte, 28 com duas ou
  mais, 16 contestadas (trechos que se contradizem), 11 fora do pedido.
- Nas 100 sem resposta: em 61 o extrator não propôs nada.
- Latência por caso: p50 9,0 s, p95 20,6 s, com dez trechos no prompt.
- 8 citações rejeitadas por não existirem literalmente.

## Leitura

1. **A regra das duas fontes corta a maior parte das respostas indevidas**:
   de 28 para 6 em 100.
2. **O preço é cobertura**: as perguntas respondidas caem de 57 para 24.
   Nesse conjunto, a resposta costuma estar em um trecho só, então exigir
   duas famílias manda a maioria para abstenção.
3. **Quando responde, cita a evidência certa** na grande maioria dos casos
   (86% com uma fonte, 96% com duas), pelo critério dos anotadores.
4. Com dez trechos, a latência do caminho de resposta passa do teto de 5 s.

## Limites

- Uma amostra de 200, um modelo, uma execução, inglês. Lida só em agregado.
- "No Answer Present." tem ruído: parte dessas consultas é respondível pelos
  trechos, então 6 e 28 superestimam o erro em alguma medida não medida.
- Os trechos do MS MARCO são recortes curtos escolhidos pelo Bing, não
  páginas inteiras coletadas pelo sistema. Na busca ao vivo, a quantidade de
  fontes que repetem um fato pode ser maior ou menor que aqui.
- "Citou o trecho dos anotadores" mede se a evidência é a esperada, não se a
  frase da resposta está correta.

## Decisão em aberto

A afirmação com uma fonte só não se perde: ela sai na lista com status
`single_source` e a citação. O que a regra decide é se o status geral é
`extractive` ou `insufficient_evidence`. Há três políticas possíveis, e a
escolha é de produto:

- **Conservadora (atual):** só duas famílias viram resposta. 6% de respostas
  indevidas, 24% de cobertura neste conjunto.
- **Permissiva:** uma fonte basta. 57% de cobertura, 28% de respostas
  indevidas.
- **Rotulada:** uma fonte vira um status próprio ("resposta de fonte única"),
  distinto de resposta corroborada e de abstenção, e o cliente escolhe o que
  aceitar por tarefa. É o que o whitepaper descreve como política por tarefa.
