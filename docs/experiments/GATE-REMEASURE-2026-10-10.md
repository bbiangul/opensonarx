# Medições refeitas com o gate de completude — 10 de outubro de 2026

Código do commit `c5671ce` (branch `restructure`), `claude-haiku-5.5` via
OpenRouter, as mesmas amostras já usadas antes do gate. As rodadas com e sem
gate são passadas diferentes do modelo, então parte das diferenças é
variação entre execuções.

Com o gate, uma resposta apoiada sai de dois jeitos: **completa** (status
`extractive`: uma checagem aprovou que cobre a pergunta inteira) ou
**parcial** (status `partial_answer`).

## Resultados

| Conjunto e regra | Com resposta: completas / parciais / abstenções | Sem resposta: completas / parciais |
|---|---|---|
| SQuAD 2.0 amostra D, uma fonte — antes do gate | 94 / – / 6 | 11 / – |
| SQuAD 2.0 amostra D, uma fonte — com gate | 84 / 7 / 9 | 7 / 3 |
| MS MARCO QA amostra A, duas fontes — antes | 24 / – / 76 | 6 / – |
| MS MARCO QA amostra A, duas fontes — com gate | 16 / 1 / 83 | 1 / 3 |
| MS MARCO QA amostra A, uma fonte — antes | 57 / – / 43 | 28 / – |
| MS MARCO QA amostra A, uma fonte — com gate | 44 / 15 / 41 | 18 / 10 |

Cada grupo tem 100 casos. No SQuAD, das 84 completas, 81 contêm a resposta
de referência, e as 7 parciais também contêm. No MS MARCO, as respostas
emitidas citam o trecho marcado pelos anotadores em 17 de 17 (duas fontes) e
53 de 59 (uma fonte).

Regressão de desenvolvimento (20 casos): 19/20; o caso `quantity` abstém.

## Leitura

- O gate reduz as respostas completas indevidas em todos os conjuntos: 11 → 7,
  6 → 1 e 28 → 18.
- O custo é cobertura: as respostas completas a perguntas respondíveis caem
  de 94 para 84, de 24 para 16 e de 57 para 44. No SQuAD, as 7 rebaixadas a
  parcial continham a resposta de referência.
- Com gate e duas fontes, o sistema dá resposta completa a 16% das perguntas
  respondíveis do MS MARCO e erra em 1%.
- As montagens sem juiz de responsividade agora sempre abstêm: o gate não
  permite resposta com requisito crítico pendente, e sem juiz o requisito
  "responde ao pedido" fica desconhecido.
- Latência por caso: p50 de 8,3 s (SQuAD) e 10,6 s (MS MARCO, dez trechos),
  com quatro a cinco chamadas ao modelo.

## Limites

Os mesmos das rodadas anteriores: uma amostra de 200 por conjunto, um modelo,
inglês, rótulos "sem resposta" com ruído. As amostras D e A já tinham sido
rodadas antes; nenhuma foi lida caso a caso.

Relatórios: `outputs/validation-20261009/answer-eval-gate-haiku-5.5-*.json`.
