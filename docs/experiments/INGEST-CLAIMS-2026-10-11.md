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

## Passo 3: consulta de afirmações no protocolo

Feito depois do passo 2, para resolver os dois pontos que ele apontou. Não
registrado antes; os números contra os rótulos são descritivos.

### O que mudou no código do produto

- `oi-types/src/claims.rs`: os tipos de afirmação e o cálculo de
  corroboração desceram do SDK para os tipos compartilhados, para a rede
  poder usá-los. Uma afirmação viaja como um resultado comum, com a
  similaridade absoluta como nota e com o próprio vetor.
- `oi-index`: `ClaimIndex` agora mora aqui, e o `HybridIndex` de um node
  guarda as afirmações das suas páginas (`ingest_claims`, `fetch_claims`).
  Ainda só em memória: o `save` do índice não as grava.
- `proto/opensonarx.proto`: `QueryRequest.claims` e `SearchResult.embedding`.
- `oi-network`: um pedido com `claims` é respondido por busca vetorial pura
  nas afirmações do node, e o node de entrada junta as respostas por
  similaridade absoluta, sem o corte por URL (`build_merged_claims`). O
  repasse entre nodes não mudou.

### Resultado

As mesmas 240 consultas de teste, com as afirmações repartidas entre os
nodes como no passo 2.

| | Índice no processo (passo 1) | 1 node, pela rede | 3 nodes, pela rede |
|---|---|---|---|
| Tempo total, p50 | 3,6 ms | 3,8 ms | 4,7 ms |
| Tempo total, p95 | 5,3 ms | 5,5 ms | 6,4 ms |
| Consulta ao node, p50 | 0,27 ms | 0,30 ms | 1,23 ms |
| Embedding refeito por quem consulta | não | não | não |
| Com selo, erradas (teste) | 5 de 61 | 5 de 61 | 4 de 60 |
| Sem selo, erradas (teste) | 26 de 116 | 26 de 116 | 27 de 117 |

- **Um node pela rede dá exatamente o mesmo que o índice no processo:** as
  240 listas de afirmações são idênticas. O caminho de rede não altera nada.
- **Três nodes custam 1 ms a mais** em localhost e não exigem embedding
  nenhum além do da consulta.
- **O selo com três nodes se mantém:** 6,7% contra 23,1%, diferença de
  +16,4 pontos (+6,6 a +26,3).

### Por que um node e três ainda diferem em 5 consultas

A diferença não vem da rede; vem da busca vetorial ser aproximada (HNSW).
Comparando cada configuração com uma varredura exaustiva de todas as
afirmações:

| | Mesmas 20 mais próximas (recall) | Mesma afirmação-cabeça | Mesmo selo |
|---|---|---|---|
| Índice único (12.396 afirmações) | 0,914 | 218 de 240 | 236 de 240 |
| Três nodes (cerca de 4.100 cada) | 0,950 | 226 de 240 | 235 de 240 |

Três índices menores erram menos do que um grande, então a versão
distribuída fica mais perto do resultado exato do que a de um node só. O
selo da varredura exaustiva, no teste: 6 erradas em 65 contra 25 em 112
(+13,1 pontos, +2,7 a +23,4).

O índice vetorial de afirmações usa os parâmetros padrão do HNSW. Subir a
precisão dele é um ajuste de índice que ficou por fazer.

### O que continua faltando

- Persistência das afirmações no disco e replicação entre nodes (feitas
  no passo 4).
- Ligar a extração ao crawler: hoje a ingestão é acionada pelo banco de
  testes.
- Nodes em máquinas diferentes. Tudo aqui é um processo em localhost.
- Conferência do trabalho de um node por outro, e o que vai na cadeia.

## Passo 4: como dividir o trabalho entre nodes

Não registrado antes; números contra os rótulos são descritivos.

### Como o código divide hoje

- **Por assunto.** Cada trecho é classificado num de dez compartimentos de
  assunto (`TopicRouter`: o embedding do trecho contra o embedding da
  descrição de cada assunto). Um node escolhe quais compartimentos serve e
  os anuncia.
- **Consulta.** O node de entrada busca nos seus compartimentos e repassa a
  consulta a um node de cada um dos outros compartimentos que o roteador
  aponta para aquela consulta (três por padrão).
- **Réplica.** Um compartimento com menos de três nodes é copiado para
  outros.
- **Não há divisão por localização** em lugar nenhum do código. Existe uma
  função de sub-compartimentos por hash dentro de cada assunto, mas nada a
  usa.

### Medição: assunto contra hash

As 12.396 afirmações foram divididas em dez compartimentos de duas formas,
e cada consulta de teste perguntou a só N deles. A referência é a varredura
exaustiva de todas as afirmações.

| Compartimentos consultados | Por assunto: 20 mais próximas achadas | Por assunto: mesmo selo | Por hash: 20 mais próximas achadas | Por hash: mesmo selo |
|---|---|---|---|---|
| 1 de 10 | 43% | 205 de 240 | 9% | 163 de 240 |
| 2 de 10 | 63% | 223 de 240 | 17% | 167 de 240 |
| 3 de 10 | 75% | 232 de 240 | 27% | 175 de 240 |
| 5 de 10 | 86% | 237 de 240 | 47% | 193 de 240 |
| 10 de 10 | 96% | 240 de 240 | 97% | 240 de 240 |

O selo contra os rótulos do teste:

| Divisão e consulta | Com selo, erradas | Sem selo, erradas | Diferença |
|---|---|---|---|
| Varredura exaustiva | 6 de 65 | 25 de 112 | +13,1 (+2,9 a +23,4) |
| Assunto, 1 de 10 | 2 de 38 | 29 de 139 | +15,6 (+4,9 a +24,8) |
| Assunto, 3 de 10 | 6 de 59 | 25 de 118 | +11,0 (+0,4 a +21,2) |
| Assunto, 5 de 10 | 6 de 63 | 25 de 114 | +12,4 (+2,0 a +22,6) |
| Hash, 3 de 10 | 2 de 11 | 29 de 166 | −0,7 (−26,4 a +20,2) |
| Hash, 5 de 10 | 3 de 26 | 28 de 151 | +7,0 (−7,3 a +19,9) |

- **Dividir por assunto permite perguntar a poucos nodes.** Com 3 de 10, o
  selo sai igual ao exato em 232 de 240 consultas e continua separando certo
  de errado. O que se perde é cobertura: menos respostas recebem o selo (59
  contra 65).
- **Dividir por hash obriga a perguntar a todos.** Nada na consulta indica
  qual compartimento tem o que ela precisa; com 3 de 10 o sinal some.
- **Os compartimentos por assunto ficam desbalanceados.** "news" recebeu
  3.368 afirmações e "education" 394. O roteador usa só a descrição de cada
  assunto como centro; já tinha concordado com o meu rótulo de assunto em
  apenas 40% das consultas (READER-GRADED-SUBJECTS).

### Centros aprendidos do conteúdo

O mesmo roteador do produto, trocando as dez descrições fixas por dez
centros aprendidos das próprias afirmações guardadas (k-means esférico,
início fixo, sem usar rótulo nenhum).

| | Assuntos fixos | Centros aprendidos |
|---|---|---|
| Maior compartimento | 27% das afirmações | 16% |
| Menor compartimento | 394 | 578 |
| Consultando 1 de 10: 20 mais próximas achadas | 43% | 56% |
| Consultando 3 de 10: 20 mais próximas achadas | 75% | 82% |
| Consultando 3 de 10: mesmo selo que o exato | 232 de 240 | 236 de 240 |
| Consultando 3 de 10: selo contra os rótulos | 6 em 59 contra 25 em 118 | 5 em 61 contra 26 em 116 |
| Consultando 5 de 10: 20 mais próximas achadas | 86% | 91% |

Os centros aprendidos equilibram melhor e acham mais com o mesmo número de
nodes consultados, em todos os tamanhos de consulta. Os centros foram
ajustados nas mesmas afirmações em que as consultas buscam; num índice que
cresce, eles teriam de ser refeitos de tempos em tempos ou herdados de um
retrato anterior, e isso não foi medido.

### O que foi construído na rede

- **Gravação em disco.** As afirmações de um node são salvas com o índice
  (`claims.bin`) e reconstruídas na carga.
- **Réplica.** Quando um node copia páginas de outro, as afirmações dessas
  páginas vão junto.
- **Conferência por quem recebe.** O node que recebe só guarda a afirmação se
  encontrar a citação no texto da página que ele mesmo tem. A família de
  fonte é derivada da URL por quem recebe; o que o remetente escreveu não é
  usado. Teste com dois nodes reais: o primeiro guarda uma afirmação honesta
  e uma inventada; o segundo replica e fica só com a honesta, e passa a
  respondê-la.
- Para isso, o conferidor de citação e a função de família de fonte desceram
  para `oi-types`, onde todas as camadas usam a mesma versão.

### Defeito encontrado na réplica de páginas

O índice guarda o texto de cada trecho com o título na frente, e era esse
texto que ia para a réplica. O hash de conteúdo, calculado sem o título,
deixava de bater, e o node que recebia **rejeitava toda página que tivesse
título**. Corrigido: a réplica recebe o trecho como foi ingerido.

Um segundo defeito, visto no mesmo teste, também foi corrigido: quando um
lote inteiro era rejeitado, a marca de progresso da réplica pulava para o
fim do que o outro node tinha, e tudo o que ainda não tinha sido enviado
ficava para trás. Agora a marca só avança pelo que foi de fato recebido.

### Feito depois: afirmações tardias e ligação ao cliente

- **Afirmações extraídas depois da página.** Cada pedido de réplica informa
  até qual afirmação o node já recebeu, e a resposta traz as mais novas. No
  teste com dois nodes, uma afirmação extraída depois de a página já ter
  sido copiada chega sozinha na rodada seguinte.
- **Extração ligada à ingestão do cliente.** Com um extrator configurado, as
  páginas que o cliente indexa entram numa fila e são lidas em segundo
  plano, depois de os resultados já terem sido entregues. Pela linha de
  comando: `OI_EXTRACT_CLAIMS=1` liga a extração e `oi search --claims`
  mostra o que as afirmações guardadas dizem e quanto tempo a leitura
  levou. Numa primeira busca com índice vazio, duas páginas renderam 67
  afirmações em 8 s; na segunda busca a leitura levou 3,6 ms.
- **Um limite que esse exemplo mostrou.** Para "qual é o maior deserto do
  mundo", as duas afirmações "parecidas" encontradas foram "o deserto da
  Arábia é o maior da Ásia" e "o Saara é o maior deserto quente". São
  próximas no embedding e não dizem a mesma coisa, nem respondem à
  pergunta. O selo separa certo de errado em média, mas o conjunto que ele
  conta não é concordância. Por isso a linha de comando lista as afirmações
  por fonte e não escreve "corroborado". Julgar equivalência entre
  afirmações precisa de modelo; o lugar natural é a ingestão, comparando a
  afirmação nova com as vizinhas, e isso não foi feito.

### O que continua faltando na rede

- O roteador do produto ainda usa as descrições fixas; os centros
  aprendidos só existem na medição.
- Quem recebe confia no texto da página que o outro node mandou. Conferir a
  página exige buscá-la de novo na web.
- Nodes em máquinas diferentes. Tudo aqui roda em localhost.
- Recompensa e punição na cadeia pelo trabalho de ingestão.

### Limites do passo 2

- Todos os nodes num processo só, numa máquina só, em localhost. Não há
  latência de rede de verdade.
- A variante com reordenação foi configurada depois de ver a primeira; os
  números dela contra os rótulos são descritivos.

### Arquivos

- Passo 2: `outputs/validation-20261009/ingest-claims-nodes-{1,3}-test.json`
  e `ingest-claims-nodes-{1,3}-rerank-test.json`
- Passo 4: `outputs/validation-20261009/ingest-claims-routing-test.json`
- Passo 3: `outputs/validation-20261009/ingest-claims-nodes-{1,3}-protocol-test.json`;
  a varredura exaustiva está no campo `exact` de `ingest-claims-signal-test.json`
- Afirmações: `outputs/validation-20261009/ingest-claims-live-480.jsonl`
- Sinal no desenvolvimento e no teste:
  `outputs/validation-20261009/ingest-claims-signal-{dev,test}.json`
- Análise: `scripts/ingest_claims_analysis.py`
