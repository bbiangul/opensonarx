# Reestruturação da camada de resposta — 9 de outubro de 2026

Branch `restructure`. Plano: [RESTRUCTURE-PLAN.md](../RESTRUCTURE-PLAN.md).

## Resultado

As regras lexicais que decidiam significado foram removidas do código de
produção. Entender a pergunta, achar a afirmação num texto e julgar apoio ou
contradição passou a ser trabalho de um modelo atrás de dois traits. O código
determinístico ficou só com o que é reproduzível a partir dos bytes retidos.

A superioridade geral sobre busca de mercado **continua não demonstrada**. O
caminho novo tem só validação de desenvolvimento, descrita abaixo.

## O que saiu

| Onde | O que era |
|---|---|
| `oi-sdk/query_intent.rs` (4.526 linhas, apagado) | Classificação de intenção por palavra-chave, ~40 funções `is_*_query`, sinônimos de relação, palpites de domínio, rotas de login/preço |
| `oi-sdk/answer.rs` (6.509 → 1.178 linhas, com testes) | Extração por listas de "fundou/criou/inventou", guard de negação por janela de tokens, extrator de presidente, heurísticas de pesquisa eleitoral |
| `oi-sdk/client.rs` (8.127 → 4.094 linhas, com testes) | `canonical_equivalent_query` (devolvia as queries do `live_search_bench`), cascatas por intenção, cache de navegação, sonda de rotas transacionais |
| `oi-sdk/evidence.rs` (1.773 → 402 linhas, com testes) | Papéis de fonte por lista de domínios, massas de evidência por intenção, marcadores de press release |
| `oi-sdk/engines/mod.rs` | Listas de "predicados genéricos" e de palavras de pergunta no filtro de lote do provedor |
| `oi-sdk/engines/wikipedia.rs` | Detecção de português por 8 palavras |
| `oi-sdk/market_data.rs` | Gatilho só para "bitcoin" na query |
| `oi-sdk/pesqele.rs` | Escopo "brasil + presidente" e descoberta fixa em "Eleições Gerais 2026" |
| `oi-index/hybrid.rs` | Boost de 1,5× por palavra-chave → domínio de documentação (typescript, rust, react, python…) |
| `oi-crawl/quality.rs` | Lista de 20 frases de "AI slop" e lista de 48 stopwords em inglês |
| `oi-facts/gliner.rs` | Fato `related_to` fabricado entre entidades vizinhas |
| `bench/live_search_bench.rs` (apagado) | Suíte de 24 casos cujas queries tinham vazado para produção |

Várias dessas listas coincidiam item a item com queries e URLs dos
benchmarks, inclusive dos holdouts v10 a v13. Os números medidos antes desta
data sobre a camada antiga ficam como histórico e não descrevem o sistema
atual.

As 96 queries da suíte aposentada estão em
`bench/fixtures/development-queries-seen.json`. O capturador de holdout
continua recusando qualquer manifesto que repita uma delas.

## O que entrou

- **Traits em `oi-types/evidence.rs`:** `ClaimExtractor` (propõe afirmações com
  citações) e `StanceJudge` (rotula cada passagem como apoia, contradiz ou
  neutro, com citação).
- **`oi-sdk/answer.rs`:** localiza cada citação literalmente no texto retido
  (após normalizar espaços e aspas tipográficas), descarta a que não achar,
  conta famílias de fonte pelo domínio registrável e aplica a política
  numérica `answer-evidence-v2` (2 famílias, citação de 20+ caracteres).
  Status por afirmação: `corroborated`, `single_source`, `contested`.
- **`oi-sdk/evidence.rs`:** Stage 2 mantém a ordem do retrieval enquanto não há
  afirmação avaliada; com stance, reordena só dentro da banda de relevância.
- **`oi-sdk/chat_model.rs`:** `OllamaChatModel` implementa os dois traits.
  Na CLI, `OI_ANSWER_MODEL` liga o modelo. Sem modelo, `--answer` devolve a
  lista de resultados e não afirma nada.
- **Join do TSE:** dispara quando uma página citada imprime um número de
  registro no formato oficial, sem olhar a query.
- **`oi-bench answer-eval`:** roda o caminho de resposta sobre casos em JSONL.

Produção nos módulos da camada de resposta: de ~13,5 mil para ~3,3 mil linhas.

## Medições

Uma máquina, Ollama local, `gpt-oss:20b` com `think=low`.

| Verificação | Resultado |
|---|---|
| `cargo test --workspace` | 981 passam, 0 falham, 23 ignorados (eram 1.172; os testes que afirmavam o comportamento lexical saíram junto com ele) |
| BEIR scifact nDCG@10 | 0,6980, igual a antes |
| Agent-cost, palavra-chave no top 5, com o boost de domínio | 47/50 |
| Agent-cost, sem o boost de domínio | 47/50, mesmos acertos e erros por pergunta |
| `answer-eval` v1 (20 casos) | 19/20; 9/9 respondíveis; 1/10 resposta indevida |
| `answer-eval` v2, depois de ajustar o prompt | 20/20; 9/9; 0/10; p50 5,8 s, máx. 9,1 s |
| Ponta a ponta, web real, índice vazio | 3 perguntas: 2 respostas completas com 2 a 4 famílias citadas, 1 abstenção correta com 1 fonte; 22 a 41 s |

O boost de domínio não tinha efeito mensurável nesse benchmark.

A falha da v1: para "quem é o CEO da Initech?", com fontes dizendo "ex-CEO
até 2019", o modelo emitiu "foi CEO até 2019". A afirmação é verdadeira e
qualificada, mas não responde à pergunta. O prompt passou a exigir a mesma
entidade e o mesmo período. Como o ajuste veio depois de ver o caso, a v2 é
regressão de desenvolvimento.

Relatórios: `outputs/validation-20261009/answer-eval-gpt-oss-20b-low-v1.json`
e `-v2.json`.

## O que não foi validado

- Nenhum holdout selado rodou no caminho novo. Os 20 casos foram escritos e
  inspecionados por quem implementou.
- Um modelo só, uma execução. A saída de modelo não é determinística.
- Latência: 22 a 41 s ponta a ponta com modelo local, contra o teto proposto
  de 5 s. Um modelo mais rápido ou hospedado não foi medido.
- Recuperação com índice populado, carga e concorrência não foram medidas
  depois da mudança.
- Os conjuntos externos do plano (Natural Questions, XNLI-pt, FEVER, intenção
  rotulada) não foram integrados.

## Perdas conscientes

- **Papéis de fonte.** A ordenação por listas de domínio (referência editada,
  UGC, autoridade) tinha levado o v12/v13 de derrota a paridade com o Serper.
  Saiu por ser lista hardcoded com entradas vindas dos próprios pacotes de
  avaliação. Se voltar, volta como arquivo de política versionado ou como
  calibração tipada de fonte.
- **Navegação e ações.** Cache de destino, palpite de domínio e sonda de
  rotas saíram. O modelo ainda pode indicar um destino entre os resultados.
- **Cotações.** `market_data` virou utilitário com par explícito; nada no
  pipeline o chama.
- **Descoberta de registros do TSE** por eleição fixa saiu; o join por ID
  ficou.

## Listas que ficaram, para decisão

Não decidem significado, mas são dados embutidos em código:

- `oi-crawl/extractor.rs`: nomes de classe de boilerplate e seletores de
  conteúdo, alguns específicos de um site (Wikipedia, handbook do TypeScript).
- `oi-crawl/fetcher.rs`: ramo próprio para `bcb.gov.br` dentro do fetcher
  genérico; deveria ser um adapter como os de Reddit e YouTube.
- `oi-crawl/discovery.rs`: segmentos de URL pulados (login, cart, admin…).
- `oi-types/entity.rs`: plataformas de UGC para stake por criador.
- `oi-types/shard.rs`: 10 tópicos com descrições que viram centróides.
- `oi-embed/corpus.rs`: 500 frases de calibração da projeção do CLIP.
- `oi-sdk/engines/mod.rs`: parâmetros de tracking removidos de URLs.

## Feito depois, sem rodar modelo

- **Teste de vazamento** (`tests/integration/benchmark_leakage.rs`): falha se
  uma query de `bench/fixtures/` com três ou mais palavras aparecer na parte
  de produção de qualquer arquivo em `crates/`. Roda com a suíte, logo no CI.
- **Menos espera pelo modelo:** o stance de cada afirmação é julgado em
  paralelo, e nenhum modelo é chamado quando não há passagem para ler ou
  texto retido para julgar. Os prompts não mudaram, então a medição do
  `answer-eval` v2 continua descrevendo o que está no código.
- **Dados externos:** `oi-bench answer-eval-import-squad` converte um arquivo
  SQuAD 2.0 em casos, metade com resposta no texto e metade sem. A amostra
  sai do hash do id com um rótulo, então ninguém escolhe perguntas olhando
  para elas. Como cada caso tem um documento só, rodar com
  `--min-source-families 1`. O conversor foi testado com um arquivo de
  exemplo no formato; **o SQuAD real não foi baixado nem avaliado**.

Suíte depois disso: 990 passam, 0 falham.

## Avaliação externa com modelo hospedado

Backend do OpenRouter adicionado (`ChatModel`, chave lida de
`OPENROUTER_API_KEY`, fora da configuração serializável). Modelo:
`anthropic/claude-haiku-5.5`. O modelo local não foi mais usado.

Dados: SQuAD 2.0 dev (11.873 perguntas, sha256 `80a5225e…182d8`). Cada amostra
tem 200 casos, 100 com resposta no parágrafo e 100 sem, sorteados pelo hash
do id com um rótulo; as três amostras não se intersectam. Cada caso tem um
documento só, então a política rodou com 1 família de fonte em vez de 2. Os
arquivos de casos ficam em `bench/datasets/` (fora do git, texto CC BY-SA);
os relatórios estão em `outputs/validation-20261009/`.

| Amostra | Pipeline | Com resposta: corretas / string diferente / abstenções | Sem resposta: respondidas mesmo assim |
|---|---|---|---|
| A | extração + stance | 85 / 7 / 8 | **34** de 100 |
| B | extração + stance | 92 / 2 / 6 | 24 de 100 |
| B | + responsividade v1 | 71 / 7 / 22 | 15 de 100 |
| C | extração + stance | 87 / 9 / 4 | 22 de 100 |
| C | + responsividade v2 | 87 / 9 / 4 | **17** de 100 |

"String diferente" são respostas emitidas cujo texto não contém a resposta
de referência. Na amostra A, as 7 foram lidas: todas eram paráfrases com a
referência dentro do trecho citado. Em B e C a referência está no trecho
citado em 93 a 97 de 100 casos.

O que as amostras mostraram:

1. **A (primeira rodada).** O sistema acha a resposta quando ela está no
   texto. O problema é o inverso: em 34 de 100 perguntas que o parágrafo não
   responde, ele respondeu. Nos casos lidos, a afirmação era verdadeira e
   citada literalmente, mas sobre algo parecido com o perguntado ("testes de
   Mersenne" → "primos de Mersenne"; "Australian" → "Austrian"), e o próprio
   extrator anotava que não respondia à pergunta. Citações inventadas quase
   não aparecem: 2 rejeitadas em 200 casos.
2. **B (responsividade v1).** Um terceiro julgamento, "a afirmação responde
   ao que foi perguntado?", tirou 9 respostas indevidas e derrubou 22
   corretas. Rejeitava só pela redação ("as secreções são chamadas de tinta"
   para "como as secreções são chamadas?"). Resultado negativo, mantido.
3. **C (responsividade v2).** A rejeição passou a exigir o par de termos
   divergentes, um copiado da pergunta e outro da afirmação, e o código
   descarta a rejeição se os dois não existirem literalmente. Na comparação
   pareada: 6 respostas indevidas removidas, 1 nova; nas perguntas com
   resposta, 2 perdidas e 2 ganhas, que é a variação entre execuções.

A afirmação rejeitada não some: fica na lista com status `off_request`, como
evidência relacionada, e não pode ser apresentada como a resposta.

Regressão de desenvolvimento (20 casos) com o modelo hospedado: 20/20 nas
três versões. Latência por caso no harness: p50 de 4 a 6 s, p95 de 7 a 13 s.

### Limites desta avaliação

- **17 a 22% das perguntas sem resposta ainda são respondidas.** O SQuAD 2.0
  constrói essas perguntas para parecerem respondíveis, então é um teste
  duro, mas o número é o que foi medido.
- Com uma fonte por caso, a regra de duas famílias não foi exercida. Não se
  sabe quanto ela reduziria as respostas indevidas.
- 100 casos por grupo: a diferença de 22 para 17 está dentro do que uma
  amostra desse tamanho distingue mal. A amostra C só foi lida em agregado.
- As perguntas do SQuAD foram escritas olhando o parágrafo e muitas dependem
  dele ("qual era a mais importante dessas cidades?"). Consultas reais são
  autocontidas; o efeito disso não foi medido.
- Inglês, um modelo, uma execução por configuração. O modelo pode ter visto o
  SQuAD no treino; a exigência de citação literal limita, mas não elimina,
  esse efeito.
- A latência de ponta a ponta com o modelo hospedado (busca + coleta +
  modelo) não foi medida, só a do caminho de resposta.

## Próximos passos

1. Reduzir as respostas a perguntas sem resposta (hoje 17 a 22%). Medir um
   modelo maior no papel de extrator e medir o efeito da regra de duas
   fontes com um conjunto multi-documento.
2. Medir latência de ponta a ponta com o modelo hospedado contra o teto de
   5 s.
3. Avaliação em português e conjuntos para stance (XNLI-pt, FEVER).
4. Roteador de intenção por embedding, para não chamar o modelo em consultas
   de destino. Precisa de um conjunto rotulado externo.
5. Julgar o stance de todas as afirmações numa chamada só. Muda o prompt,
   então exige nova medição.
