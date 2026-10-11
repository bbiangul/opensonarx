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
- **Não há divisão por localização** em lugar nenhum do código.
- **Sub-compartimentos.** O crawler, quando tem um roteador, grava cada
  trecho em `assunto/N`, com N saindo do hash do conteúdo (dez por assunto).
  O caminho de ingestão do cliente grava só o assunto. (Uma versão anterior
  deste documento dizia que nada usava essa função; estava errado.)

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

### Dois nodes em processos separados

Até aqui os nodes rodavam dentro de um processo só. Esta checagem usa o
binário do produto (`oi network node`) duas vezes na mesma máquina, cada um
com sua pasta de dados e sua porta. O node A tem o extrator ligado
(`OI_EXTRACT_CLAIMS=1`); o node B não tem modelo nenhum. Não é uma medição
com amostra: é uma verificação de que o caminho funciona de ponta a ponta.

O que foi acrescentado para isso:

- `POST /claims` na API do node: devolve as afirmações guardadas mais
  próximas da consulta, as famílias de fonte e o tempo da leitura.
- O node lê as páginas que aprende numa busca, depois de responder.
- Nodes do mesmo compartimento pedem réplica um ao outro a cada rodada, a
  partir de onde pararam. Antes só existia a cópia para compartimentos com
  poucas réplicas e uma recuperação que disparava quando o outro node tinha
  o dobro do conteúdo (e no mínimo 100 trechos), então dois nodes pequenos
  do mesmo compartimento nunca trocavam nada.

Resultado: os dois nodes ficaram 45 s parados, o node A recebeu uma busca
("qual é o maior deserto do mundo"), aprendeu as páginas e extraiu 109
afirmações em cerca de 20 s. O node B, sem modelo e sem ter buscado nada,
tinha as mesmas 109 afirmações 71 s depois da busca (a rodada de réplica é
de 60 s). Para três consultas os dois devolveram as mesmas afirmações na
mesma ordem, em 0,2 a 8 ms. Como o node B só guarda uma afirmação cuja
citação ele encontra no texto da página que recebeu, as 109 passaram por
essa conferência.

Três defeitos apareceram só com processos de verdade, e nenhum teste os
pegava:

1. **O sinal de vida apagava os assuntos do node.** A cada 30 s um node
   avisa que está vivo. Quem recebia esse aviso regravava o registro do
   outro com a lista de compartimentos vazia, e o anúncio de compartimentos
   só é enviado quando um node se conecta ou muda de compartimentos. Depois
   de meio minuto o node B não sabia mais de quem pedir réplica (observado);
   pelo código, o repasse de consulta lê a mesma lista. Os testes
   terminavam antes do primeiro aviso, porque o intervalo configurado
   (`heartbeat_interval_secs`) não era usado pelo aviso, que estava fixo em
   30 s. Agora o aviso respeita a configuração e mantém a lista; o teste de
   dois nodes falha sem a correção e passa com ela.
2. **Dois nodes na mesma máquina dividiam o arquivo da tabela de peers.**
   Ele ficava em `~/.opensonarx`, fora da pasta de dados. Um node carregava
   peers de outra execução e discava um node que não existia mais. Agora o
   arquivo fica na pasta de dados do node.
3. **A sincronização escolhia sempre o mesmo peer por compartimento.** Se
   esse peer estivesse morto, ela nunca tentava outro. Agora ela alterna
   entre todos os peers conhecidos daquele compartimento.

O exemplo repetiu o limite já registrado: para "qual é o maior deserto do
mundo" a afirmação guardada mais próxima foi "o deserto da Síria é o décimo
maior do mundo", e as três famílias contadas são as que têm afirmações
parecidas com essa. Para "quem pintou a Mona Lisa", sobre a qual o índice
não tem nada, o node devolveu zero famílias.

### O que continua faltando na rede

- O roteador do produto ainda usa as descrições fixas; os centros
  aprendidos só existem na medição (passos 4 e 6). Para ir ao produto falta
  decidir quem publica os centros: todos os nodes precisam usar os mesmos,
  senão o mesmo nome de compartimento quer dizer coisas diferentes.
- Quem recebe confia no texto da página que o outro node mandou. Conferir
  buscando a página de novo foi medido no passo 5 e não está ligado à
  réplica.
- Nodes em máquinas diferentes. Os dois processos acima estão na mesma
  máquina, sem latência nem perda de rede.
- A sincronização entre nodes do mesmo compartimento compara o
  identificador exato; um node que serve `assunto` e outro que serve
  `assunto/3` não se sincronizam por esse caminho.
- Recompensa e punição na cadeia pelo trabalho de ingestão.

## Passo 6: a divisão como o produto faz (a afirmação fica com a página)

### Por que medir de novo

No passo 4 cada afirmação foi para o compartimento mais próximo do
embedding **dela**. O produto não faz isso. No produto quem é classificado
é o trecho da página; as afirmações ficam no node que tem a página, e é
assim que elas são replicadas (a réplica precisa do texto da página para
conferir a citação). Uma página com trechos em dois compartimentos leva
todas as suas afirmações para os dois. Os números do passo 4 (232 de 240
por assunto, 236 de 240 com centros aprendidos) são de uma divisão que o
produto não usa.

Lendo o código da junção apareceu também um defeito: se dois nodes
devolvem a mesma afirmação, o node de entrada e a função de corroboração
ficam com as duas cópias, e elas tiram lugar de outras afirmações nas 20
mais próximas. Com a divisão do passo 4 isso nunca acontecia, porque cada
afirmação estava num lugar só.

### Protocolo (registrado antes de rodar)

- **Dados e consultas.** As mesmas 12.396 afirmações e as mesmas 240
  consultas de teste do passo 4. A referência é a mesma varredura exaustiva.
  Os rótulos de certo e errado não entram.
- **Divisão `page-topic`.** Cada trecho de página é classificado pelo
  roteador do produto (descrições fixas) a partir do embedding do texto do
  trecho. Todas as afirmações de um URL vão para todo compartimento que
  recebeu algum trecho daquele URL.
- **Divisão `page-learned`.** O mesmo, com dez centros aprendidos dos
  embeddings dos trechos (o que um node tem em mãos), mesmo k-means do passo
  4 (dez grupos, vinte rodadas, início fixo).
- **Consulta.** Como no passo 4: os compartimentos são perguntados na ordem
  em que o roteador os classifica para a consulta, 1, 2, 3, 5 ou 10 deles,
  cada um devolve suas 20 afirmações mais próximas, e o resultado passa pela
  função de corroboração do produto.
- **Medidas.** As do passo 4: quantas das 20 afirmações da varredura
  exaustiva foram recuperadas, e em quantas das 240 consultas o selo é o
  mesmo. Mais: tamanho do maior compartimento e quantas cópias de cada
  afirmação existem em média.
- **Duas rodadas.** Uma com a junção como está (cópias repetidas contam),
  outra depois de corrigir para ficar com uma cópia de cada afirmação
  idêntica (mesmo URL e mesmo texto). As duas são relatadas.
- **Leitura.** Comparo perguntando três compartimentos com o passo 4. Nada
  é ajustado; não há limiar novo.

### Resultado

12.396 afirmações de 1.198 URLs, 1.255 trechos classificados, 240
consultas. Cada célula é "afirmações recuperadas / consultas com o mesmo
selo da varredura exaustiva". Números da rodada com a junção corrigida.

| Divisão | Maior compartimento | Cópias por afirmação | 1 compartimento | 3 | 5 | os 10 |
|---|---|---|---|---|---|---|
| Passo 4, descrições fixas (afirmação por si) | 27,2% | 1,00 | 0,43 / 205 | 0,75 / 232 | 0,86 / 237 | 0,96 / 240 |
| **Produto, descrições fixas** (`page-topic`) | 18,3% | 1,04 | 0,48 / 199 | 0,79 / 231 | 0,88 / 237 | 0,97 / 240 |
| Passo 4, centros aprendidos (afirmação por si) | 16,3% | 1,00 | 0,56 / 218 | 0,82 / 236 | 0,91 / 237 | 0,96 / 238 |
| **Produto, centros aprendidos** (`page-learned`) | 15,7% | 1,02 | 0,65 / 227 | 0,85 / 236 | 0,93 / 239 | 0,97 / 240 |

- **A divisão que o produto faz se sustenta.** Perguntando três
  compartimentos, o selo é o mesmo em 231 de 240 consultas com as
  descrições fixas e em 236 de 240 com centros aprendidos dos trechos.
  São os números do passo 4 (232 e 236), agora na divisão de verdade.
- **Os centros aprendidos ganham mais com um compartimento só**: 227 de 240
  contra 199.
- **O erro tem uma direção só.** Perguntando três, todas as discordâncias
  foram selos que a varredura exaustiva dava e a consulta parcial perdeu (9
  com descrições fixas, 4 com centros aprendidos). Nenhuma consulta ganhou
  um selo que a varredura completa não dava.
- **Cópias repetidas.** Com a junção como estava, perguntando os dez
  compartimentos a recuperação ficava em 0,95 (fixas) e 0,96 (aprendidos);
  corrigida, 0,97 nos dois. Nenhuma contagem de selo mudou. O efeito é
  pequeno aqui porque há pouca repetição: 1,02 a 1,04 cópia por afirmação.
- Os números já publicados do passo 4 se repetem iguais nas duas rodadas
  (a correção mexe na terceira casa da recuperação, porque 14 das 12.396
  afirmações guardadas eram repetidas).

A correção está no produto: a função de corroboração e a junção do node de
entrada ficam com uma cópia de cada afirmação idêntica (mesmo URL e mesmo
texto).

### Limites do passo 6

- Os trechos são os que o pipeline de resposta reteve, um ou dois por URL,
  não os trechos que um node corta da página inteira (até oito). Com mais
  trechos por página haveria mais páginas em dois compartimentos e mais
  cópias; não medi isso.
- Os centros foram aprendidos do mesmo conteúdo em que as consultas são
  procuradas. Nenhum rótulo entra, mas um índice que cresce precisaria
  refazer os centros ou conviver com centros antigos.
- Sem rede: cada compartimento é um índice no mesmo processo.
- Uma primeira rodada saiu com o embedder falso (binário compilado sem o
  modelo) e foi descartada antes de qualquer leitura.

## Passo 5: conferir uma afirmação buscando a página por conta própria

### Pergunta

Hoje a réplica confere a citação no texto da página que o próprio node de
origem mandou. Um node desonesto pode mandar página e afirmações inventadas
que combinam entre si. A defesa é o conferente buscar a página na web por
conta própria. A pergunta é quanto isso custa a um node **honesto**: se a
página mudou, saiu do ar ou é servida diferente a cada visitante, as
afirmações honestas dele falham na conferência. Qualquer punição na cadeia
por "citação não encontrada" depende desse número.

### Protocolo (registrado antes de rodar)

- **Dados.** As 12.396 afirmações de `ingest-claims-live-480.jsonl`,
  extraídas em 10 e 11 de outubro por um caminho honesto; todas tiveram a
  citação conferida no trecho na hora da extração.
- **Amostra.** Os URLs distintos em ordem do SHA-256 do URL; os 300
  primeiros. Entram todas as afirmações desses URLs. A ordem por hash não
  depende de nada que eu tenha visto nos resultados.
- **Conferente.** `oi_crawl::Distiller::crawl(url)`, o caminho que o produto
  usa para buscar e limpar uma página, rodado agora (um a dois dias depois
  da extração). Nenhum modelo. Uma tentativa por URL, 30 s de limite.
- **Conferência.** `locate_quote` (a função que a réplica usa, mínimo de 20
  caracteres) sobre o texto limpo da página inteira que o conferente
  buscou. Como leitura secundária, a mesma conferência trecho a trecho, que
  é como a réplica faz.
- **Controle: afirmação que não é da página.** Para cada URL auditado, as
  citações do próximo URL na mesma ordem que seja de outra família de fonte
  são conferidas contra esta página. Elas deveriam falhar. Isso mede se a
  conferência distingue alguma coisa.
- **Medidas.** (a) fração dos URLs que o conferente conseguiu buscar;
  (b) entre os buscados, fração das afirmações honestas cuja citação foi
  encontrada, e fração dos URLs com todas encontradas; (c) fração das
  citações de outra página que foram aceitas. Intervalos de 95% por
  reamostragem de URLs.
- **Leitura combinada antes de ver o resultado.** Punir por afirmação
  isolada só se sustenta se (b) for pelo menos 99% e (c) no máximo 1%.
  Abaixo disso a conferência serve como sinal estatístico (reputação,
  amostragem repetida, vários conferentes), não como prova contra uma
  afirmação. Nenhum limiar é ajustado depois.
- **O que esta medição não é.** Não simula um node desonesto. Não diz se a
  afirmação é verdadeira, só se a citação está na página. Um conferente só,
  de um endereço só, num momento só.

### Resultado

300 URLs de 269 famílias de fonte, 3.106 afirmações. Conferente rodado em
11 de outubro.

| Medida | Resultado |
|---|---|
| (a) URLs que o conferente conseguiu buscar | 294 de 300 (98,0%); 5 responderam 403 e 1 estourou o tempo |
| (b) afirmações honestas com citação encontrada na página | 3.004 de 3.033 = 99,0% [98,3%, 99,6%] |
| (b) a mesma conferência trecho a trecho, como a réplica faz | 3.015 de 3.033 = 99,4% [98,9%, 99,8%] |
| URLs com todas as afirmações encontradas | 282 de 294 (95,9%) |
| URLs com nenhuma afirmação encontrada | 0 |
| (c) citações de outra página aceitas | 0 de 3.056 |

Leitura pela regra combinada:

- **(c) passa com folga.** Nenhuma das 3.056 citações tiradas de outra
  página foi aceita. A conferência rejeita o que não está na página.
- **(b) não sustenta punição por afirmação isolada.** A estimativa cai em
  cima da linha (99,0%) e o intervalo desce a 98,3%; não dá para afirmar
  que é pelo menos 99%. Em números de operação: um node honesto com dez mil
  afirmações teria por volta de cem reprovadas um dia depois, e 4 em cada
  100 páginas dele teriam alguma afirmação reprovada.
- Além disso, 2% das páginas o conferente não consegue buscar. Uma regra
  de punição teria de tratar isso como "sem conclusão", e isso abre uma
  brecha: citar páginas que bloqueiam quem confere.

O que isso sustenta, como desenho e não como código: a conferência serve
para rejeitar na entrada (como a réplica já faz) e como sinal acumulado por
node. Um node cuja taxa de reprovação fica muito acima de 1% em muitas
páginas e para mais de um conferente é diferente de um node com uma página
que mudou. Nada disso está construído.

### Observação depois do resultado (não registrada antes)

Rodei o conferente uma segunda vez, minutos depois, para saber se as
reprovações eram ruído de busca. Dos 294 URLs buscados nas duas vezes, 293
deram a mesma contagem, e os 12 URLs com reprovação foram os mesmos. Então
não é ruído: são páginas cujo texto já não é o que foi lido na extração.
Pelos endereços, são páginas de previsão do tempo do dia, verbetes de
dicionário com exemplos rotativos, tabela de salários, página de música e
um contador de leis; essa caracterização é leitura minha dos endereços, não
medida.

A leitura por página inteira reprovou 11 afirmações a mais que a leitura
trecho a trecho (três URLs passam inteiros por trecho e falham pela página).
As duas usam a mesma função sobre textos que o mesmo `Distiller` produz;
não investiguei por que diferem.

### Limites do passo 5

- Um conferente, um endereço de rede, um a dois dias depois da extração. A
  taxa de reprovação honesta deve subir com o tempo; não medi quanto.
- Não há node desonesto no experimento. O controle mede citação de outra
  página, não uma página forjada, nem uma citação real usada para sustentar
  uma afirmação que ela não sustenta.
- As afirmações vieram de 480 consultas; páginas que aparecem em busca não
  são uma amostra da web.

## Passo 7: um modelo julga, na ingestão, se duas afirmações dizem a mesma coisa

### Pergunta

O selo de hoje conta famílias de fonte com afirmações **parecidas** com a
afirmação mais próxima da consulta (parecidas pelo embedding, acima de
0,841). Parecido não é igual: "o Saara é o maior deserto quente" e "o
deserto da Arábia é o maior da Ásia" passam nesse corte. A pergunta é se
trocar esse corte pelo julgamento de um modelo, feito uma vez quando a
afirmação entra no índice, dá um selo que separa melhor certo de errado, e
se o julgamento em si é confiável.

Continua não havendo modelo na consulta: o julgamento é entre duas
afirmações guardadas, não vê a consulta, e o resultado fica guardado.

### O que é construído (código do produto)

- `ClaimRelationJudge` em `oi-types`: recebe uma afirmação e uma lista de
  outras e devolve, para cada uma, `same` (o mesmo fato), `contradicts` (não
  podem ser as duas verdadeiras) ou `different`. A decisão é do modelo; o
  código não tem regra de linguagem.
- O índice de afirmações guarda essas relações e responde, para uma
  afirmação, quantas famílias de fonte dizem o mesmo e quantas contradizem.
- Nesta etapa as relações ficam em memória. Gravar em disco, replicar e
  ligar à ingestão do cliente só se a medição se sustentar.

### Protocolo (registrado antes de rodar)

- **Dados.** As mesmas 12.396 afirmações, as mesmas 480 consultas e os
  mesmos rótulos de leitor dos passos anteriores. Desenvolvimento: as 240
  de `live-1` e `misinfo-1`. Teste: as 240 de `trust-1`.
- **Sinal `judged-v1`.** A afirmação de referência é a guardada mais próxima
  da consulta. Se a semelhança dela com a consulta é menor que 0,725, não há
  selo. Senão, o modelo julga a referência contra as 20 afirmações de outras
  famílias mais próximas **dela** (procuradas entre as 60 mais próximas).
  O selo existe quando pelo menos duas famílias, contando a da referência,
  têm afirmação julgada `same`.
- **Nenhum número é escolhido olhando resultado.** 0,725 e "duas famílias"
  vêm do passo 1 (escolhidos no desenvolvimento). 20 é o mesmo tamanho de
  busca que o produto já usa; 60 é três vezes isso, fixado agora.
- **O texto do pedido ao modelo** é escrito antes de rodar. Posso testar o
  formato em cinco consultas de desenvolvimento; depois disso só mudo se a
  resposta vier malformada, e registro a mudança.
- **Modelo.** `claude-haiku-5.5` pelo OpenRouter, o mesmo da extração. Uma
  chamada por afirmação de referência.
- **Medida principal, no teste.** Taxa de erro das respostas diretas com
  selo contra sem selo, com intervalo por reamostragem, como no passo 1. O
  critério é o mesmo de lá. Ao lado, o selo atual nas mesmas respostas
  (5 de 61 erradas com selo, 26 de 116 sem).
- **Conferência do juiz por leitura minha.** 120 pares do teste (referência,
  candidata): 60 que o modelo julgou `same` e 60 que não, em ordem de hash,
  embaralhados, sem o julgamento do modelo à vista. Eu marco `same`,
  `contradicts` ou `different` e só depois comparo. Se houver menos de 60
  `same` no teste, entram todos.
- **Descritivo.** Quantas consultas têm referência contradita por outra
  família, e a taxa de erro nelas.
- **O que precisa ficar dito.** É a segunda vez que o conjunto de teste é
  usado para avaliar um selo. Nenhum parâmetro novo é ajustado nele, mas não
  é uma amostra nova. Na medição o modelo julga só as referências das 480
  consultas; um node faria isso para toda afirmação que entra, o que aqui
  seriam 12 mil chamadas.

### Resultado

O modelo julgou as 320 afirmações de referência que passam no corte de
0,725 (das 480 consultas), 6.445 pares: 153 `same`, 44 `contradicts`, 6.248
`different`. Nenhuma chamada falhou; cada uma levou 3,3 s na mediana. A
leitura do selo a partir das relações guardadas leva 0,2 ms.

**O selo julgado não se sustentou.** Respostas diretas, erradas / total:

| | Com selo | Sem selo | Diferença | Cobertura |
|---|---|---|---|---|
| **Teste, selo julgado pelo modelo** | 3 / 31 (9,7%) | 28 / 146 (19,2%) | +9,5 pontos [−3,8, +20,9] | 18% |
| Teste, selo por semelhança (o atual) | 5 / 61 (8,2%) | 26 / 116 (22,4%) | +14,2 pontos [+4,1, +24,3] | 34% |
| Desenvolvimento, selo julgado | 2 / 33 (6,1%) | 21 / 134 (15,7%) | +9,6 pontos [−1,2, +19,3] | 20% |
| Desenvolvimento, selo por semelhança | 6 / 61 (9,8%) | 17 / 106 (16,0%) | +6,2 pontos [−4,3, +16,2] | 37% |

- No teste o intervalo do selo julgado inclui zero. Pelo critério do passo
  1 ele não passa, e cobre metade das respostas que o selo atual cobre.
- O selo atual repete no teste o número já publicado (5 de 61 contra 26 de
  116). Trocar o corte de semelhança pelo julgamento do modelo não melhorou
  o selo. O julgamento **não** foi ligado à ingestão do produto.
- O selo atual, no desenvolvimento, também tem intervalo que inclui zero;
  isso já estava no passo 1 (os limiares foram escolhidos ali).

**Conferência do juiz, por leitura minha, sem ver o julgamento** (120 pares
do teste):

| Modelo \ leitor | same | contradicts | different |
|---|---|---|---|
| same (60) | 47 | 0 | 13 |
| contradicts (0) | 0 | 0 | 0 |
| different (60) | 1 | 0 | 59 |

- Dos 60 pares que o modelo julgou iguais, concordei em 47 (78%). Dos 60
  que ele julgou não iguais, achei 1 igual.
- As 13 discordâncias vêm de 5 afirmações de referência. Nove são da mesma
  ("a principal diferença entre mudança química e física é o que acontece
  com a composição"), contra afirmações que definem só um dos dois lados;
  eu marquei `different` em todas, e outro leitor poderia marcar `same`.
  Três são casos de fronteira. Uma é erro claro do modelo: o título de um
  vídeo do YouTube julgado igual a "a proibição de Baise-moi foi suspensa
  em 2001".
- Nenhum par `contradicts` caiu na amostra, então a leitura não diz nada
  sobre essa categoria.

**Descritivo, visto depois do resultado:**

- As três respostas erradas com selo julgado no teste: numa, as fontes
  concordam entre si sobre uma afirmação emoldurada ("segundo a Bíblia, o
  primeiro homem foi Adão") e a resposta certa era outra; noutra, as fontes
  concordam e estão certas, e a resposta escrita errou por outro motivo; a
  terceira é o erro do modelo citado acima. Fontes concordando não é a
  resposta estar certa.
- Onde os dois selos coincidem, 0 de 25 respostas erradas no teste e 1 de
  26 no desenvolvimento. Isso apareceu depois de olhar e não é um resultado:
  precisaria de uma amostra nova, com a regra escrita antes.
- Referência contradita por outra família: 12 respostas no teste, 1 errada;
  13 no desenvolvimento, 2 erradas. Não distingue nada aqui.

### O que fica no código

A interface `ClaimRelationJudge`, a implementação com modelo e o
armazenamento de relações no índice ficam, com testes, e nada no caminho do
produto os chama. O selo servido continua sendo o `ingest-claims-v1`.

### Limites do passo 7

- Segundo uso do mesmo conjunto de teste. 31 respostas com selo é pouco: o
  intervalo é largo demais para dizer que o selo julgado é pior que o
  atual; o que dá para dizer é que ele não mostrou ser melhor.
- Um modelo, um texto de pedido, escrito uma vez. Outro modelo ou outra
  definição de "mesmo fato" pode dar outro resultado.
- Só as referências das consultas foram julgadas, contra as 20 vizinhas
  mais próximas de outras famílias. Uma afirmação igual que esteja mais
  longe que isso no embedding não é vista.
- A conferência do juiz foi feita por um leitor só (eu), e as amostras de
  pares se concentram em poucas afirmações de referência.

### Limites do passo 2

- Todos os nodes num processo só, numa máquina só, em localhost. Não há
  latência de rede de verdade.
- A variante com reordenação foi configurada depois de ver a primeira; os
  números dela contra os rótulos são descritivos.

### Arquivos

- Passo 2: `outputs/validation-20261009/ingest-claims-nodes-{1,3}-test.json`
  e `ingest-claims-nodes-{1,3}-rerank-test.json`
- Passo 4: `outputs/validation-20261009/ingest-claims-routing-test.json`
- Passo 7: `outputs/validation-20261009/ingest-claims-relations.jsonl` (o que
  o modelo julgou), `ingest-claims-judged-480.json` (o selo por consulta),
  `ingest-claims-relations-test-{sheet,key,reader-labels}.jsonl` (a leitura
  cega); `scripts/claim_relation_sheet.py` e o modo `judged` de
  `scripts/ingest_claims_analysis.py`
- Passo 6: `outputs/validation-20261009/ingest-claims-routing-page-asis-test.json`
  (junção como estava) e `ingest-claims-routing-page-dedup-test.json`
  (corrigida)
- Passo 5: `outputs/validation-20261009/ingest-claims-audit-300.json` e
  `ingest-claims-audit-300-second.json` (segunda rodada)
- Passo 3: `outputs/validation-20261009/ingest-claims-nodes-{1,3}-protocol-test.json`;
  a varredura exaustiva está no campo `exact` de `ingest-claims-signal-test.json`
- Afirmações: `outputs/validation-20261009/ingest-claims-live-480.jsonl`
- Sinal no desenvolvimento e no teste:
  `outputs/validation-20261009/ingest-claims-signal-{dev,test}.json`
- Análise: `scripts/ingest_claims_analysis.py`
