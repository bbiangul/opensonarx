# Probabilidade calibrada para respostas em texto — 10 de outubro de 2026

## Resultado

O mecanismo está implementado e desligado por padrão. **Nenhuma tabela de
calibração passou no critério de aceitação**, então o sistema não exibe
probabilidade. As respostas continuam no modo evidencial.

## O que foi construído

- `oi-sdk/src/calibration.rs`: uma tabela de desfechos resolvidos, por estado
  de evidência. Para uma resposta completa, a tabela alimenta o motor de
  incerteza (`oi-index/uncertainty.rs`), que devolve probabilidade e intervalo
  de 90% só quando: o checklist está completo, o estado tem pelo menos 20
  desfechos, o deslocamento declarado está abaixo do limite e a tabela foi
  aceita. Resposta parcial ou abstenção nunca recebe número.
- `oi-bench answer-calibrate`: corrige respostas completas contra as
  respostas de referência com um modelo avaliador, ajusta a tabela nos
  relatórios de treino e mede nos relatórios separados. `--replicate` mede
  uma tabela existente em dados novos sem reajustar.
- Critério de aceitação, fixado antes da replicação (commit `d696389`): ganho
  relativo de Brier de pelo menos 3% sobre prever sempre a taxa-base do
  treino, com limite inferior do intervalo bootstrap de 95% acima de zero. É
  o mesmo par de condições que o PILOT-GATES pede para um sinal probabilístico.
- Opt-in: `OI_ANSWER_CALIBRATION=<tabela.json>` ou `with_answer_calibration`.

## Alvo e dados

- **Alvo:** a resposta completa transmite a mesma resposta que uma referência
  do conjunto, na avaliação de um modelo (`claude-sonnet-5.5`). Resposta a um
  caso que o conjunto marca como sem resposta conta como incorreta.
- **Dados:** MS MARCO QA v2.1, 2.000 consultas novas em dez partes de 200
  (metade com resposta, metade sem), sem interseção entre si nem com amostras
  anteriores. Pipeline com gate (`claude-haiku-5.5`), dez trechos por caso,
  variante de uma fonte, para haver respostas com uma e com duas ou mais
  famílias de fonte.

## Tabela 1 — só número de famílias de fonte

Ajustada nas partes 1–3 (201 desfechos), medida nas partes 4–5 (117).

| Famílias | Previsto | Observado no teste |
|---|---|---|
| 1 | 54% | 58% (42 de 73) |
| 2 ou mais | 80% | 70% (31 de 44) |

Brier 0,2349 contra 0,2348 do chute constante. Erro de calibração de 5,6
pontos. **Rejeitada:** não acrescenta nada a um número único.

## Tabela 2 — famílias × o extrator anotou lacuna

Só nas partes de treino, a nota de lacuna do extrator separava bem os
desfechos (97% de acerto com duas fontes e sem lacuna; 47% com uma fonte e
lacuna). A tabela foi ajustada nas partes 1–5 (318 desfechos).

| Estado | Previsto | Partes 6–7 | Partes 8–10 (replicação) |
|---|---|---|---|
| 1 família, sem lacuna | 71% | 77% (17 de 22) | 79% (30 de 38) |
| 1 família, com lacuna | 51% | 46% (25 de 54) | 65% (64 de 99) |
| 2+ famílias, sem lacuna | 84% | 82% (27 de 33) | 80% (32 de 40) |
| 2+ famílias, com lacuna | 67% | 38% (8 de 21) | 52% (17 de 33) |

| Medida | Partes 6–7 | Partes 8–10 |
|---|---|---|
| Desfechos | 130 | 210 |
| Ganho relativo de Brier sobre a taxa-base | +8,0% | −1,1% |
| Intervalo bootstrap de 95% do ganho | −0,6% a +16% | −8,6% a +6,0% |
| Erro de calibração esperado | 8,1 pontos | 11,3 pontos |

O primeiro teste passava no ganho pontual e não no limite inferior. A
replicação, registrada antes de rodar e sem reajustar a tabela, deu ganho
negativo. **Rejeitada.** O ganho de 8% era variação de amostra.

## O que se sustenta

- Respostas completas com duas ou mais famílias de fonte e sem lacuna anotada
  acertaram 85% (52 de 61), 82% (27 de 33) e 80% (32 de 40) nas três medições
  independentes.
  É o estado mais estável e o mais próximo da configuração do produto.
- Os estados "com lacuna" variam de 38% a 67% entre amostras: não há base para
  dar um número a eles.
- Entre as respostas a perguntas que têm resposta, o avaliador considerou
  corretas 87% nas partes 1–5 (201 de 231) e 78% nas partes 6–7 (77 de 99). A maior parte do erro vem de
  responder a consultas que o conjunto marca como sem resposta.

## Limites

- A correção das respostas é a nota de um modelo, não julgamento humano.
- O rótulo "sem resposta" do MS MARCO tem ruído, e a proporção desses casos
  na amostra (cerca de metade) fixa a taxa-base. Outra mistura muda todas as
  probabilidades.
- Trechos de conjunto de dados, não recuperação ao vivo. Inglês, um modelo de
  pipeline, um avaliador.
- Vídeo, áudio e imagem não foram tratados: o sistema processa texto.

## Próximo passo, se a probabilidade continuar sendo objetivo

Mais desfechos por estado e sinais mais estáveis que a nota de lacuna, por
exemplo o histórico de acerto por fonte, que é o que o motor foi desenhado
para usar e ainda não existe. Qualquer tabela nova passa pelo mesmo
critério, em dados que ela não viu.

Artefatos em `outputs/validation-20261009/`:
`answer-calibration-msmarco-qa-v1-families-only.json`,
`answer-calibration-msmarco-qa-v2-rejected.json`, os desfechos resolvidos
(`*-outcomes.jsonl`) e os relatórios `answer-eval-gate-haiku-5.5-msmarco-cal-*.json`.
