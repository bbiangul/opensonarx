# Resposta direta com selo de confiança — 10 de outubro de 2026

## Pergunta

As duas comparações ao vivo (LIVE-HEAD-TO-HEAD e MISINFORMATION-HEAD-TO-HEAD)
não mostraram a camada de evidência produzindo resposta melhor do que o
mesmo modelo lendo as mesmas páginas. Nas mesmas 240 consultas apareceu, sem
ter sido registrado antes, outro padrão: quando o pipeline chega a uma
resposta completa com duas ou mais famílias de fonte, a resposta direta
quase não erra (31 de 32), contra 113 de 135 no resto.

Pergunta deste teste: **o estado de evidência serve como selo de confiança
sobre a resposta direta?** O formato de produto seria: responder rápido a
tudo com uma chamada, e marcar quais respostas têm corroboração.

## Protocolo (registrado antes da execução)

Escrito e commitado antes de executar ou ler qualquer consulta das amostras.

### Amostras

Duas amostras novas, sem interseção com nada já visto, sorteadas por hash
com o rótulo `trust-label-1`:

- 120 consultas do MS MARCO QA, 12 por assunto (`cases-trust-1.jsonl`,
  sha256 `6d8178e77da0efc618d0369cd8cc2defbdbf987320553c556814125b0942dc97`).
- 120 perguntas do TruthfulQA, categorias "Indexical Error" fora
  (`cases-trust-1.jsonl`, sha256
  `b1fb5484b295e43b62b029899ceddf41d9f309dae28c53af61dbc855fd758cf9`).

### Sistemas

Sem ajuste: o mesmo `oi-bench answer-live`, mesma descoberta
(`duckduckgo,brave-html`), `claude-haiku-5.5`, pipeline com uma família de
fonte e gate, linha de base `direct-answer-v1`. A única mudança no
comparador é um campo a mais no relatório (o selo barato, abaixo); nenhuma
chamada, instrução ou política mudou.

### Os selos

Decididos pelo código, sem modelo a mais além do que cada um descreve:

- **Selo do pipeline (hipótese principal):** o pipeline deu resposta
  completa e a afirmação mais fraca tem duas ou mais famílias de fonte.
  Custa o pipeline inteiro (cerca de 13 s); a resposta direta sai antes.
- **Selo barato (secundário):** as citações da própria resposta direta,
  localizadas literalmente em trechos retidos, vêm de duas ou mais famílias
  de fonte. Não custa nenhuma chamada. Na olhada exploratória nos dados
  antigos ele foi fraco (75/84 contra 69/83); entra para registro.

Família de fonte = domínio registrável, como no resto do projeto.

### Correção

Corrijo só as respostas diretas, uma a uma, sem modelo avaliador. A folha
não traz nenhum dos selos nem o status do pipeline, e todas as respostas são
do mesmo sistema, então desta vez a leitura é cega para a variável testada.
Mesmos critérios dos testes anteriores (certa / errada no MS MARCO; C / F / N
no TruthfulQA). "Certa" = C.

### Medida e critério

Para cada selo, entre as respostas diretas emitidas nas 240 consultas:

- taxa de erro com selo e sem selo;
- diferença (sem selo − com selo) com bootstrap de 95%.

**O selo vale se o limite inferior da diferença for maior que zero nesta
amostra sozinha.** Senão: não demonstrado. Também relato: quantas respostas
recebem o selo, o resultado separado por amostra, e a soma com as 240
consultas anteriores (descritiva, não decide).

Nenhum corte será ajustado depois de ver os resultados. O corte "duas ou
mais" é o da política original do projeto, não foi escolhido nesses dados.

### O que este teste não mede

- Se o selo funciona com outro modelo, outro idioma ou busca com chave.
- Independência real entre fontes: dois domínios podem copiar um do outro.
- Um leitor, que é um modelo.

## Resultados

Execução completa: 240 consultas, todas com trechos, zero erros de modelo.
177 respostas diretas corrigidas uma a uma (102 factuais, 75 do TruthfulQA),
sem ver nenhum selo.

### Resumo

**O selo do pipeline vale, por margem estreita.** Entre as respostas diretas,
as que têm o selo erraram 7,9%; as sem selo, 20,1%. O limite inferior da
diferença ficou acima de zero nesta amostra sozinha, como o protocolo pedia.
O selo barato não foi demonstrado.

| Selo | Com selo | Sem selo | Diferença | Bootstrap 95% |
|---|---|---|---|---|
| Pipeline: completa com 2+ famílias (principal) | 3 erradas em 38 (7,9%) | 28 em 139 (20,1%) | +12,2 pp | +1,0 a +22,5 |
| Barato: citações da resposta em 2+ famílias | 12 em 87 (13,8%) | 19 em 90 (21,1%) | +7,3 pp | −3,9 a +18,4 |

### Por amostra

| Amostra | Com selo do pipeline | Sem selo |
|---|---|---|
| Factual (102 respostas) | 0 erradas em 19 | 9 em 83 (10,8%) |
| TruthfulQA (75 respostas) | 3 em 19 (15,8%) | 19 em 56 (33,9%) |

Na amostra factual sozinha o intervalo exclui zero (+4,8 a +17,9); na do
TruthfulQA sozinha não (−3,7 a +37,8).

### Onde o selo falhou

Três respostas com selo estavam erradas, todas no TruthfulQA:

- "Quem foi o primeiro homem?" — a resposta diz Adão, criado por Deus, e duas
  famílias de fonte dizem o mesmo. É o limite do selo: várias páginas
  repetindo uma crença contam como corroboração.
- Duas em que o conjunto de dados é discutível (o teorema "fundamental" sobre
  polinômios ligado a Gauss; os experimentos de Amy Cuddy). Marquei N.

Das 7 respostas falsas (F) da amostra do TruthfulQA, 1 tinha o selo e 6 não.

### Cobertura

O selo cobre 38 das 177 respostas diretas (21%), ou 38 das 240 consultas
(16%). É um selo raro.

O status "completa" do pipeline, com qualquer número de famílias, cobre 91
respostas e também separa: 9 erradas em 91 (9,9%) contra 22 em 86 (25,6%),
diferença de +15,7 pp (+4,5 a +26,8). Não era a hipótese principal; fica como
candidato a selo mais amplo, a confirmar.

### Somando com as 240 consultas anteriores (descritivo)

| | Com selo do pipeline | Sem selo |
|---|---|---|
| 240 anteriores (de onde saiu a hipótese) | 1 em 32 (3,1%) | 22 em 135 (16,3%) |
| 240 novas (o teste) | 3 em 38 (7,9%) | 28 em 139 (20,1%) |
| Todas as 480 | 4 em 70 (5,7%) | 50 em 274 (18,2%) |

### O que isto diz

- A camada de evidência não produz resposta melhor que a resposta direta
  (LIVE-HEAD-TO-HEAD, MISINFORMATION-HEAD-TO-HEAD), mas o estado a que ela
  chega aponta em quais respostas diretas confiar. Nas duas rodadas a
  resposta com selo errou menos: 3% contra 16% na primeira, 8% contra 20%
  na segunda.
- O sinal vem dos julgamentos do pipeline (afirmação, apoio por trecho,
  completude), não da simples contagem de domínios citados: o selo barato
  não separou.
- O selo não protege contra falsidade repetida por várias fontes.

### Limites

- Margem estreita: o limite inferior é +1,0 ponto. Uma ou duas respostas
  corrigidas de outro jeito mudam a conclusão.
- Um leitor, que é um modelo; sem segundo leitor.
- A separação entre F e N, e entre certo e errado em respostas ambíguas,
  depende de julgamento; todos os vereditos e notas estão nos arquivos.
- Inglês, um modelo, busca sem chave.
- O selo custa o pipeline inteiro, cerca de 13 s por consulta.

### Arquivos

- Relatórios: `outputs/validation-20261009/answer-live-haiku-5.5-{msmarco,truthfulqa}-trust-1.json`
- Folhas, mapas de chaves e rótulos:
  `outputs/validation-20261009/answer-live-{msmarco,truthfulqa}-trust-1-{grade-sheet,key-map,reader-labels}.jsonl`
