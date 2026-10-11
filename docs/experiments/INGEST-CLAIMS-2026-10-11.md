# Afirmações extraídas na ingestão — 11 de outubro de 2026

## Pergunta

O selo de confiança que funcionou em TRUST-LABEL é calculado por um modelo
na hora da consulta e custa cerca de 13 s. A proposta do projeto é outra:
os nodes fazem o trabalho pesado quando a página entra no índice, e a
consulta vira uma busca.

Duas perguntas, medidas no menor tamanho possível:

1. **Quanto custa a consulta** quando as afirmações já foram extraídas e
   conferidas na ingestão?
2. **O sinal sobrevive?** Um selo de corroboração calculado só com o que
   está guardado, sem modelo na consulta, ainda separa resposta certa de
   errada?

## O que foi construído (código do produto)

- `oi-types`: trait `PassageClaimExtractor`. Lê um trecho, sem pergunta
  nenhuma, e lista o que ele afirma, cada afirmação com a citação.
- `oi-sdk/src/chat_model.rs`: implementação com um modelo de chat.
- `oi-sdk/src/claims.rs`:
  - `ingest_passage`: guarda a afirmação só se a citação for localizada
    literalmente no trecho, pelo mesmo conferidor determinístico do caminho
    de resposta;
  - `ClaimIndex`: afirmações com embedding, buscáveis no HNSW do
    `oi-index`;
  - `corroboration`: embedding da consulta, busca das afirmações mais
    próximas; a mais próxima é a cabeça; contam-se as famílias de fonte
    distintas entre as afirmações que dizem o mesmo que a cabeça.
- `oi-bench claim-ingest` e `claim-signal`: alimentam as páginas e medem.
  Não decidem nada.

## Protocolo (registrado antes de calcular qualquer sinal)

Esta seção foi escrita e commitada com a ingestão ainda rodando, antes de
existir qualquer número de sinal.

### Dados

Os trechos retidos das quatro execuções ao vivo já feitas (480 consultas).
Cada trecho distinto é ingerido uma vez, com `claude-haiku-5.5`. Um único
índice de afirmações com tudo, como num node.

As respostas diretas dessas consultas já estão corrigidas (344 respostas).
Nada é corrigido de novo.

- **Desenvolvimento:** as 240 consultas de LIVE-HEAD-TO-HEAD e
  MISINFORMATION-HEAD-TO-HEAD (167 respostas corrigidas).
- **Teste:** as 240 consultas de TRUST-LABEL (177 respostas corrigidas).
  Usado uma vez, com os limiares já fixados.

### Limiares

A política tem dois limiares de similaridade: da afirmação com a consulta e
da afirmação com a cabeça. Eles são escolhidos só no conjunto de
desenvolvimento:

- candidatos: para cada limiar, os quartis 25%, 50% e 75% da distribuição
  observada no desenvolvimento (olhando só similaridades, sem os rótulos de
  certo e errado). São nove combinações;
- escolha: entre as combinações em que o selo cobre de 20% a 80% das
  respostas do desenvolvimento, a de maior diferença de taxa de erro (sem
  selo − com selo); empate vai para a de maior cobertura;
- `top_k` fica em 20 e o mínimo de famílias em 2, sem ajuste.

### Critério

No conjunto de teste, com os limiares fixados e o selo vindo do código do
SDK:

- **O selo vale** se a diferença de taxa de erro (sem selo − com selo)
  tiver limite inferior do bootstrap de 95% maior que zero. Senão: não
  demonstrado.
- Latência: p50 e p95 do trabalho de consulta (um embedding da consulta
  mais uma busca vetorial), um processo, uma consulta por vez.

Também relato a cobertura do selo, a concordância com o selo calculado na
consulta (o de TRUST-LABEL) e o custo da ingestão.

### O que esta medição não é

- **Não mede cobertura.** O índice tem exatamente as páginas que essas
  mesmas consultas trouxeram. É o caso "o índice já tem a página".
- **Um node, um processo.** Nada de rede.
- Similaridade de embedding não distingue concordância de contradição
  sobre o mesmo assunto. Duas páginas que dão números diferentes para a
  mesma pergunta podem contar como corroboração.
- As respostas foram corrigidas por um modelo, sem segundo leitor.

## Resultados

### Resumo

**As duas respostas são sim.** Com as afirmações já extraídas na ingestão, o
trabalho de consulta leva cerca de 4 milissegundos, e o selo calculado assim
passou no critério registrado: no conjunto de teste, respostas com selo
erraram 8,2% e sem selo 22,4%.

| | Selo na ingestão (este teste) | Selo na consulta (TRUST-LABEL) |
|---|---|---|
| Custo por consulta | 3,6 ms (p50), 5,3 ms (p95) | cerca de 13 s |
| Modelo na consulta | nenhum | três a quatro chamadas |
| Com selo, erradas | 5 de 61 (8,2%) | 3 de 38 (7,9%) |
| Sem selo, erradas | 26 de 116 (22,4%) | 28 de 139 (20,1%) |
| Diferença | +14,2 pp (+4,1 a +24,3) | +12,2 pp (+1,0 a +22,5) |
| Respostas com selo | 34% | 21% |

As duas colunas usam as mesmas 177 respostas corrigidas do conjunto de teste.

### Ingestão

- 1.288 trechos distintos, uma chamada de modelo por trecho, mediana de
  4,2 s por chamada. Com 8 em paralelo, 19 minutos no total.
- 12.396 afirmações guardadas; 147 descartadas porque a citação não foi
  encontrada no trecho (1,2%). Nenhum erro de modelo.
- Embedding e indexação das 12.396 afirmações: 40 s.

### Consulta

Um processo, uma consulta por vez, índice com 12.396 afirmações, embedder
`arctic-embed-s`:

| Etapa | p50 | p95 | máximo |
|---|---|---|---|
| Embedding da consulta | 3,4 ms | 5,1 ms | 6,3 ms |
| Busca das afirmações e contagem | 0,27 ms | 0,47 ms | 0,89 ms |
| Total | 3,6 ms | 5,3 ms | 6,5 ms |

Isto é o custo do selo. Não inclui buscar as fontes no índice local, que as
medições antigas do projeto põem em poucos milissegundos e que não foi
medido de novo aqui.

### Escolha dos limiares (desenvolvimento)

Nove combinações, pelos quartis. Com a regra registrada, a escolhida foi
similaridade com a consulta ≥ 0,725 e com a cabeça ≥ 0,841. No
desenvolvimento ela deu 6 erradas em 61 com selo (9,8%) contra 17 em 106
sem (16,0%): diferença de +6,2 pontos, bem mais fraca do que a do teste.

Os dois números são da mesma política em amostras diferentes. O efeito real
deve estar entre eles; somando desenvolvimento e teste, 11 em 122 (9,0%)
contra 43 em 222 (19,4%).

### Por amostra, no teste

| Amostra | Com selo | Sem selo |
|---|---|---|
| Factual | 1 errada em 32 | 8 em 70 |
| TruthfulQA | 4 em 29 | 18 em 46 |

Das 7 respostas que endossavam uma falsidade (veredito F) na amostra do
TruthfulQA, nenhuma tinha o selo.

### Onde o selo errou

Cinco respostas com selo estavam erradas:

- capacidade calorífica molar do vapor d'água: duas páginas trazem o mesmo
  número errado (o da água líquida). É a falha esperada: fontes repetindo o
  mesmo erro contam como corroboração;
- quatro do TruthfulQA marcadas N (não respondem, sem afirmar falsidade), em
  que as afirmações guardadas eram sobre o assunto mas não respondiam à
  pergunta. O selo mede que fontes concordam sobre algo próximo da consulta,
  não que a pergunta foi respondida.

### Concordância com o selo calculado na consulta

Nas 177 respostas do teste: os dois selos juntos em 23, só o da ingestão em
38, só o da consulta em 15, nenhum em 101. Eles marcam conjuntos bem
diferentes de respostas e os dois separam certo de errado. Não são o mesmo
sinal medido de dois jeitos.

### O que isto diz

- A ideia de fazer o trabalho na ingestão se sustenta na menor escala
  testável: o sinal de confiança sai em milissegundos, sem modelo na
  consulta, e não ficou pior do que o calculado na hora.
- O custo sai da consulta e vai para a ingestão: cerca de 4 s de modelo por
  página, uma vez. É trabalho que pode ser feito por nodes e conferido por
  qualquer outro node, porque a citação está ou não está na página.

### Limites

- **Cobertura total por construção.** O índice tinha exatamente as páginas
  dessas consultas. Num índice real, a maioria das consultas não estará
  coberta no começo.
- O efeito no desenvolvimento foi a metade do efeito no teste. Uma terceira
  amostra diria qual dos dois está mais perto.
- Os limiares valem para este embedder.
- Contradição entre fontes não é vista por este selo.
- Um leitor, que é um modelo; 177 respostas no teste.
- Um node, um processo. O passo 2, abaixo, mede três nodes.

## Passo 2: o mesmo sinal servido por nodes da rede

Não registrado antes; é uma medição de engenharia, não um teste do selo.

### O que foi feito

As 12.396 afirmações foram repartidas por um hash da URL entre N nodes
reais (`oi_network::Node`, TCP em localhost). Cada node guarda a sua parte
num `HybridIndex` comum e a anuncia como um compartimento. A consulta vai só
ao node 0, que busca na sua parte, repassa aos outros pela rede e junta as
respostas, tudo pelo caminho de consulta que a rede já tinha. Quem consulta
conta as famílias com o `supporting_families` do SDK.

Rodado com 1 node e com 3, nas 240 consultas de teste.

### O que funcionou

- Três nodes sobem, se descobrem e o repasse funciona sem código novo na
  rede.
- O repasse custa pouco em localhost: a consulta ao node foi de 1,9 ms
  (1 node) para 4,0 ms (3 nodes) na mediana.

### O que não funcionou

**Com três nodes o resultado não é igual ao de um.**

| Configuração | Mesmo selo em 1 e 3 nodes | Mesma afirmação-cabeça |
|---|---|---|
| Caminho de busca do node como está | 224 de 240 | 140 de 240 |
| Pedindo 60 e reordenando por similaridade absoluta em quem consulta | 233 de 240 | 202 de 240 |

Dois motivos, ambos do caminho de busca de páginas que o node usa:

- cada node ordena por uma pontuação relativa aos próprios resultados
  (fusão por posição), e a junção ordena por essa pontuação, que não é
  comparável entre nodes;
- o node devolve o melhor resultado por URL, o que serve para páginas e não
  para afirmações.

**O selo servido por esse caminho ficou mais fraco.** Nas respostas do
teste, com 1 node: 5 erradas em 45 com selo (11,1%) contra 26 em 132 sem
(19,7%), intervalo de −3,1 a +19,6 pontos. Com 3 nodes: 4 em 42 contra 27 em
135, intervalo de −1,0 a +21,2. Nos dois o intervalo inclui zero. O índice
vetorial único do passo 1 deu 8,2% contra 22,4%.

**Os vetores não viajam na resposta.** Quem consulta precisa gerar de novo o
embedding das afirmações devolvidas para compará-las com a cabeça, e isso
custa cerca de 65 ms, bem mais que a consulta em si. O total ficou em torno
de 75 ms na mediana, não 4.

### O que isto diz

A divisão em nodes e o repasse funcionam. O que não serve para afirmações é
reaproveitar a busca de páginas. Para o sinal do passo 1 valer na rede
faltam duas coisas, as duas no protocolo:

1. uma consulta de afirmações no node que seja busca vetorial pura, com a
   similaridade absoluta como pontuação, e junção por ela;
2. a resposta levar o vetor de cada afirmação, ou o node 0 calcular a
   similaridade com a cabeça depois de juntar.

Nenhuma das duas foi feita aqui.

### Limites do passo 2

- Todos os nodes num processo só, numa máquina só, em localhost. Não há
  latência de rede de verdade.
- A variante com reordenação foi configurada depois de ver a primeira; os
  números dela contra os rótulos são descritivos.

### Arquivos

- Passo 2: `outputs/validation-20261009/ingest-claims-nodes-{1,3}-test.json`
  e `ingest-claims-nodes-{1,3}-rerank-test.json`
- Afirmações: `outputs/validation-20261009/ingest-claims-live-480.jsonl`
- Sinal no desenvolvimento e no teste:
  `outputs/validation-20261009/ingest-claims-signal-{dev,test}.json`
- Análise: `scripts/ingest_claims_analysis.py`
