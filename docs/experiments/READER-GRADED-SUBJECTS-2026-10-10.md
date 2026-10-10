# Respostas corrigidas uma a uma, por assunto — 10 de outubro de 2026

## Resultado

- 400 consultas novas do MS MARCO QA, 40 por assunto em dez assuntos. O
  pipeline com gate, na variante de uma fonte, emitiu 185 respostas (127
  completas, 58 parciais) e se absteve em 215.
- As 185 foram lidas e corrigidas uma a uma, sem modelo avaliador: **163
  certas (88,1%)**.
- **Completa: 121/127 (95,3%). Parcial: 42/58 (72,4%).** O rótulo que o gate
  de completude já emite é o que separa as respostas boas das fracas.
- Dentro das completas, número de famílias de fonte e nota de lacuna não
  separam nada. A tabela de probabilidade ajustada nelas **foi rejeitada de
  novo** (ganho de Brier −3,0%). O sistema continua sem exibir probabilidade.
- A menor taxa por assunto foi a de ciência (71% no rótulo do roteador, 78%
  no meu). As amostras por assunto são pequenas demais para comparar assuntos
  entre si.

## Quem corrigiu e como

As respostas foram corrigidas pelo assistente (Claude) na sessão de trabalho,
lendo cada item. Não é julgamento humano e não houve segundo leitor; a
tendência do leitor a ser brando ou severo não foi medida.

A folha de correção (`oi-bench answer-grade-sheet`) mostra só o pedido, as
respostas de referência, a resposta e os trechos citados, em ordem
embaralhada. Ela esconde status, famílias de fonte, nota de lacuna e assunto,
que são justamente as variáveis analisadas depois.

Critério, fixado antes da leitura. Uma resposta é **certa** quando:

1. responde ao pedido como foi feito;
2. é sustentada pelos trechos que cita;
3. não contradiz uma referência plausível (referência malformada ou
   implausível é ignorada; caso que o conjunto marca como "sem resposta" é
   julgado pelo mérito);
4. não afirma nada claramente errado segundo os trechos ou conhecimento bem
   estabelecido.

Pedido aberto ("quais são alguns…") aceita lista parcial correta; pedido por
cada membro de um conjunto exige cobertura. Na dúvida, **errada**.

## Dados

- MS MARCO QA v2.1, 4.400 linhas novas, sem interseção com amostras
  anteriores. Cada consulta recebeu um dos dez assuntos do roteador de tópicos
  dos nós (`oi-bench topic-label`). Sorteio por hash: 20 consultas com
  resposta e 20 sem, por assunto (`cases-subjects.jsonl`, sha256
  `d3bffc46…579d`), divididas em duas metades de 200 antes da execução.
- Pipeline: `claude-haiku-5.5`, política `answer-evidence-v2`, gate de
  completude, dez trechos por caso, zero erros de modelo.

## Por estado de evidência

| Estado (variante de uma fonte) | Certas | Taxa |
|---|---|---|
| Completa, 1 família, com nota de lacuna | 65/70 | 92,9% |
| Completa, 1 família, sem nota | 17/17 | 100% |
| Completa, 2+ famílias, com nota | 22/23 | 95,7% |
| Completa, 2+ famílias, sem nota | 17/17 | 100% |
| Parcial, 1 família | 36/51 | 70,6% |
| Parcial, 2+ famílias | 6/7 | 85,7% |

As duas metades concordam: completas 59/62 e 62/65; parciais 28/37 e 14/21.

## Regra de duas fontes, com estes rótulos

| Respostas | Certas | Taxa |
|---|---|---|
| Também emitidas com duas fontes (47) | 45/47 | 95,7% |
| Só emitidas com uma fonte, completas (87) | 82/87 | 94,3% |
| Só emitidas com uma fonte, parciais (51) | 36/51 | 70,6% |

Exigir duas famílias descarta 138 respostas; 118 delas estavam certas. As
completas de uma fonte acertaram quase tanto quanto as de duas. Nesta amostra
o gate de completude faz o trabalho que a regra de duas fontes fazia, com
muito menos abstenção. É uma amostra e um leitor; falta repetir.

## Casos que o conjunto marca como "sem resposta"

Das 57 respostas a esses casos, **46 estavam certas** pelo critério acima: os
trechos respondiam ao pedido e o anotador do conjunto não marcou resposta.
As medições anteriores contavam toda resposta a esses casos como falsa
(TWO-SOURCE-RULE, GATE-REMEASURE, ANSWER-CALIBRATION). Aquelas contagens de
"respostas falsas" superestimam o erro real, e as taxas de acerto de lá não
são comparáveis com as daqui: o alvo é outro.

## Por assunto

Rótulo do roteador de tópicos (o do sorteio) e rótulo que atribuí lendo cada
pedido:

| Assunto | Roteador | Meu rótulo |
|---|---|---|
| business | 14/14 | 19/22 |
| education | 16/18 | 17/19 |
| entertainment | 17/19 | 11/12 |
| general | 17/20 | 34/36 |
| health | 26/28 | 27/30 |
| lifestyle | 18/19 | 20/24 |
| news | 20/22 | 1/2 |
| science | 10/14 | 14/18 |
| sports | 11/14 | 3/3 |
| technology | 14/17 | 17/19 |

- O roteador concordou com o meu rótulo em **74 de 185 (40%)**. Em "news"
  concordou em 1 de 22 e em "sports" em 1 de 14: para consultas curtas ele
  não classifica assunto de forma confiável. O MS MARCO quase não tem
  notícia nem esporte, então esses dois assuntos seguem sem teste.
- Com 12 a 36 respostas por assunto, diferenças de dez pontos cabem no ruído.
  O que dá para dizer: nenhum assunto com mais de três respostas ficou
  abaixo de 70%.
- Ciência teve a menor taxa nos dois rótulos (10/14 e 14/18). Os erros foram
  respostas que definem os termos sem dar a relação pedida.

## Onde errou (22 respostas)

- 16 das 22 eram parciais.
- Padrões recorrentes: responde a um pedido vizinho ao que foi feito; define
  cada termo de uma comparação sem dizer a diferença; apresenta como fato um
  ponto em que as fontes ou a referência divergem; lê o trecho errado.
- A lista completa, com uma nota por erro, está no arquivo de rótulos.

## Probabilidade

Tabela `answer-correctness/msmarco-subjects-reader-v1`: respostas completas,
bins por famílias de fonte, ajustada na metade 1 e medida na metade 2, com os
meus rótulos.

| | Valor |
|---|---|
| Taxa-base no treino | 95,2% |
| Brier da tabela / da taxa-base | 0,0454 / 0,0440 |
| Ganho relativo | −3,0% (bootstrap 95%: −143% a +1,3%) |
| Aceita | não |

Com a divisão adicional por nota de lacuna, só um bin tem 20 desfechos e o
ganho é +1,2% (bootstrap −233% a +5,5%): rejeitada também.

O motivo é teto: 95% das completas estão certas e nenhum dos sinais
disponíveis identifica os 5% restantes.

### Hipótese para a próxima medição, não um resultado

A diferença entre completa e parcial (95% contra 72%) apareceu depois de eu
ver estes dados. Por isso ela **não** foi ajustada nem aceita aqui, e nenhum
código novo foi escrito para ela. Como conta exploratória: uma tabela só com
esse corte, ajustada na metade 1, teria ganho de 13% na metade 2. Para valer,
precisa de amostra nova, corrigida do mesmo jeito, com o critério de
aceitação já registrado.

Simulação com as taxas observadas: 185 respostas novas passam no critério em
cerca de metade das vezes; 370 em cerca de 72%; 500 em cerca de 90%. Se a
diferença real for a metade da observada, a tabela praticamente nunca passa.
Uma replicação que decida precisa de umas 500 respostas corrigidas, cerca de
1.100 consultas.

## Limites

- Um leitor, que é um modelo; sem segundo leitor nem medida de concordância.
- Trechos vêm do conjunto, não da recuperação ao vivo.
- Inglês, um modelo de pipeline, um conjunto de dados.
- Notícia e esporte praticamente ausentes.
- Preço, idade e outros fatos que mudam com o tempo foram julgados contra os
  trechos, não contra o valor de hoje.

## Arquivos

- Relatórios: `outputs/validation-20261009/answer-eval-gate-haiku-5.5-msmarco-subjects-{1,2}.json`
- Folha de correção: `outputs/validation-20261009/reader-grade-sheet-msmarco-subjects.jsonl`
- Rótulos (com assunto e nota por erro): `outputs/validation-20261009/reader-labels-msmarco-subjects.jsonl`
- Tabela rejeitada: `outputs/validation-20261009/answer-calibration-msmarco-subjects-reader-v1-rejected.json`
