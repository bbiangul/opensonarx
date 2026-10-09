# Plano de reestruturação — camada de evidência e resposta

**9 de outubro de 2026. Status: em execução na branch `restructure`.**

| Fase | Estado |
|---|---|
| 0 — Salvar | Feito localmente (commits na branch). **Push não feito.** |
| 1 — Rede de segurança | Parcial: `oi-bench answer-eval` com 20 casos. Conjuntos externos (NQ, XNLI-pt, FEVER) pendentes. |
| 2 — Costuras | Feito, mas sem a implementação `legacy_lexical`: por decisão do dono, as regras foram apagadas em vez de preservadas atrás dos traits. |
| 3 — Modelos | Extração e stance por modelo, feitos. `IntentRouter` por embedding e agrupamento de afirmações por NLI pendentes. |
| 4 — Apagar o legado | Feito. CI com lint de strings de benchmark pendente. |
| 5 — Revalidar | Pendente: holdout selado, latência contra os gates. README e CLAUDE.md atualizados. |

Medições e perdas conscientes: [experiments/RESTRUCTURE-2026-10-09.md](experiments/RESTRUCTURE-2026-10-09.md).
O texto abaixo é o plano original.

## 1. A missão, como eu a entendi

Busca geral para pessoas e agentes de IA que (a) encontra fontes relevantes,
(b) mostra o que sustenta ou contradiz cada afirmação e (c) diz quando a
evidência não basta. Três planos separados (whitepaper §1.4):

1. **Retrieval** — amplo, rápido, cego a stake e reputação.
2. **Evidência** — por afirmação: proveniência, linhagem (N relatos de uma
   medição = 1 observação), frescor, suporte, contradição, o que falta.
3. **Decisão** — incerteza por tarefa (`DETERMINISTIC` / `CALIBRATED` /
   `EVIDENTIAL_ONLY`), rerank dentro da banda de relevância, citar ou abster.

Em volta disso: o criador escolhe as condições de uso (livre, atribuição,
pago), e capital só se move em predicado determinístico. Token, treino,
multimídia e rede global ficam fora do primeiro produto, que é uma API
operada de texto/dados para equipes com agentes.

## 2. O diagnóstico em uma frase

O whitepaper (§4) já separa **Grade A** (predicado determinístico,
reproduzível) de **Grade B** (julgamento semântico por modelo, probabilístico
e versionado). O código implementou o trabalho Grade B com `contains()` e
`matches!`.

Intenção, entidade, relação, negação, direção sujeito/objeto, completude e
equivalência entre afirmações são semântica. Hoje vivem em ~12 mil linhas de
produção em `answer.rs`, `query_intent.rs` e `client.rs`. O resultado medido:
0 respostas completas em 360 pedidos, e 1 de 7 perguntas triviais respondidas
na sonda offline. Enquanto isso o repo já tem embedder ONNX, cross-encoder
(`oi-embed/reranker.rs`) e GLiNER (`oi-facts`), e nenhum deles é usado no
caminho de resposta.

## 3. Regras da reestruturação

- **R1.** Código determinístico só para Grade A: hash, span exato no texto
  retido, datas, URL/host, dedup, joins com registros estruturados,
  aritmética.
- **R2.** Toda decisão semântica passa por um modelo atrás de um trait. A
  saída carrega `model_version` e entra como evidência, nunca como fato.
- **R3.** Modelo propõe, determinístico verifica. Uma afirmação extraída só
  vale se o span citado existir literalmente no texto retido.
- **R4.** Lista de domínios ou termos que sobrar é dado de política
  versionado (arquivo com `policy_id` no trace), nunca literal no código.
- **R5.** Nenhuma string de benchmark em `crates/`. Verificado por CI.
- **R6.** Uma regra nova só entra se melhorar o eval externo, não um caso.

## 4. Arquitetura alvo

```
query
  └─ IntentRouter (modelo)                    → intent + confiança
  └─ retrieval Stage 1 (como está)            → candidatos
  └─ hydrate (como está)                      → texto retido + hash
  └─ ClaimExtractor (modelo)                  → afirmação + span citado
  └─ SpanVerifier (determinístico)            → descarta span não literal
  └─ StanceJudge (modelo NLI)                 → apoia / contradiz / neutro
  └─ LineageResolver (determinístico)         → famílias, near-dup, registro
  └─ ChecklistPolicy (dados de política)      → requisitos críticos
  └─ OpenUncertainty (como está, oi-index)    → support/conflict/ignorance
  └─ render                                   → resposta, slate ou abstenção
```

Traits novos (em `oi-types`, seguindo a convenção do repo):

| Trait | Substitui | Implementação alvo |
|---|---|---|
| `IntentRouter` | `classify_query` e os ~40 `is_*_query` | Protótipos sobre o embedding da query, que já é calculado; fallback para LLM quando a margem for baixa |
| `ClaimExtractor` | `factual_answer`, `best_factual_sentence`, `has_required_relation_cue`, `relation_adjacent_answer_keys`, extratores de cargo e de pesquisa | LLM com saída estruturada no formato `EvidenceUnit` (§7.1); GLiNER para tipar entidades |
| `StanceJudge` | `has_negated_origin_relation`, `factual_relation_direction_matches_entity`, `factual_candidates_compatible`, `office_names_compatible` | Cross-encoder NLI multilíngue em ONNX, reaproveitando a infra de `reranker.rs` |
| `StructuredSource` | Ramos `brasil && presidente`, `bitcoin`, PesqEle e market_data dentro de `client.rs` | Plugin escolhido pelo tipo da afirmação extraída, não por termos da query |

`search_answered` (1.439 linhas) vira um orquestrador que chama esses
estágios em ordem, com orçamento de latência por estágio.

## 5. O que acontece com cada peça de hoje

| Peça | Destino |
|---|---|
| `canonical_equivalent_query`, `canonical_answer_query`, `recommendation_recovery_queries` | **Apagar.** São as queries canônicas do `live_search_bench`. O objetivo legítimo (requisições equivalentes compartilharem o mesmo voo de cache) passa a usar chave por similaridade de embedding |
| `lula.com.br`, institutos de pesquisa, `brazilian_presidential_round_model`, `is_dengue_guidance_query` | **Apagar.** O que for regra de jurisdição vira dado do plugin de pesquisas |
| Tabelas de termos em `query_intent.rs` | **Substituir** por `IntentRouter` |
| Listas de domínios em `evidence.rs` e `query_intent.rs` (papéis de fonte, autoridades de saúde) | **Mover** para arquivo de política versionado; no longo prazo, calibração tipada de fonte (§5.8.2) |
| `retained_citation_excerpt`, `source_family`, datas, hosts, `navigation_cache` | **Manter.** É Grade A |
| `checklist.rs` | **Manter**, com requisitos vindos de política por tipo de afirmação em vez de reclassificar a query |
| `uncertainty.rs`, Stage 1 de retrieval, SimHash, crawl | **Manter** sem mudança |
| `pesqele.rs`, `market_data.rs` | **Manter** como plugins `StructuredSource`; congelar evolução |
| ~220 testes que afirmam comportamento lexical | **Converter** em casos do dataset de avaliação (fase 1) |

## 6. Fases

Cada fase termina com suíte verde e um commit. Prazos são estimativas
grosseiras para uma pessoa.

### Fase 0 — Salvar o que existe (1 dia)

- Commit só de `cargo fmt`, depois commits por crate, depois docs e outputs.
- Push dos 32 commits locais mais os novos. Tag `pre-restructure`.
- Branch `restructure`. Daqui em diante toda captura referencia um SHA de
  commit, não só o hash do binário.

### Fase 1 — Rede de segurança (3 a 4 dias)

- Subcomando offline `oi-bench answer-eval`: recebe casos
  `{query, documentos, esperado}` e mede acurácia de intenção, cobertura de
  resposta, taxa de resposta errada e erros de negação/entidade.
- Dataset de regressão: os testes lexicais atuais convertidos em JSONL, mais
  os casos da sonda (Stripe, Dom Casmurro, Canberra, penicilina, "it is a
  myth that…", "allegedly… denies").
- Dataset externo, que você não escreveu: Natural Questions e SQuAD para
  extração, XNLI (pt/en) e FEVER para suporte/contradição, um conjunto
  rotulado de intenção (ORCAS-I ou similar). Uma parte selada, sem inspeção.
- Registrar a linha de base do código atual nos três conjuntos.

### Fase 2 — Costuras, sem mudar comportamento (≈1 semana)

- Criar os traits e mover as regras atuais para uma implementação
  `legacy_lexical` atrás deles.
- Quebrar `client.rs` e `answer.rs` em módulos por estágio; `search_answered`
  vira orquestrador.
- Apagar `canonical_*_query` e os domínios/nomes hardcoded. Re-rodar a
  matriz de 96 para ter o número sem vazamento.
- Mover listas de domínio para arquivo de política.
- Gate: testes verdes e `answer-eval` igual à linha de base, exceto onde o
  vazamento foi removido.

### Fase 3 — Trocar Grade B por modelos (2 a 3 semanas)

Um trait por vez. Cada troca só entra se superar `legacy_lexical` no dataset
externo e não piorar o de regressão.

1. **`StanceJudge`** primeiro: é o menor, resolve o bug de negação e já tem
   infra ONNX pronta.
2. **`IntentRouter`**: sem custo extra de latência, porque o embedding da
   query já existe.
3. **`ClaimExtractor`** + `SpanVerifier`: o maior ganho de cobertura. Cache
   por `evidence_root` + `model_version`, como o whitepaper já especifica.
4. **Corroboração**: agrupar afirmações por equivalência NLI e entidade
   normalizada, em vez de `answer_keys`. Linhagem continua por família
   editorial + near-dup.

Sem backend de modelo configurado, o sistema devolve o pacote de evidência
(passagens, localizadores, linhagem, checklist) e não tenta responder.

### Fase 4 — Apagar o legado (2 a 3 dias)

- Remover `legacy_lexical` e os testes que só existiam para ele.
- Meta: `answer.rs` + `query_intent.rs` + `client.rs` de ~12 mil para menos
  de 4 mil linhas de produção.
- CI: `clippy -D warnings`, `fmt --check`, lint que falha se alguma query de
  `bench/` aparecer em `crates/`, limite de tamanho de função.

### Fase 5 — Revalidar (≈1 semana)

- Holdout novo e selado, binário de um commit congelado.
- Latência por estágio contra os gates do PILOT-GATES (resposta completa
  p95 ≤ 5 s).
- Reescrever o README: o que funciona hoje em cima; diário de auditoria em
  `docs/experiments/`.
- Atualizar o `CLAUDE.md` (o programa é `solana_program` nativo, não Anchor).

## 7. Congelado durante a reestruturação

Programa Solana, economia e modelo de cash, gateway P0, token, whitepaper,
pitch, novos tipos de afirmação, novos plugins estruturados. Nada disso
melhora o número que importa agora, que é a cobertura de resposta com zero
erro grave.

## 8. Critérios de saída

| Métrica | Hoje | Alvo |
|---|---|---|
| Respostas completas nas 120 perguntas de diagnóstico | 0 | > 0 em todo estrato factual, sem erro grave |
| Sonda de perguntas triviais com resposta no texto | 1/7 | ≥ 6/7 |
| Negação/atribuição falsa emitida como `extractive` | 2/2 casos | 0 no conjunto adversarial |
| Intenção em conjunto externo rotulado | não medido | medido e acima do legado |
| Strings de benchmark em `crates/` | ≥ 6 | 0, garantido por CI |
| Linhas de produção nos três arquivos | ~12 mil | < 4 mil |
| p95 resposta completa fria | 3–10 s | ≤ 5 s |

## 9. Riscos

- **Injeção de prompt em conteúdo rastreado.** O whitepaper já aponta.
  Mitigação: R3 (span literal) e julgamento de stance por modelo separado do
  extrator.
- **Custo e latência de LLM.** Mitigação: cache por raiz de evidência,
  extração só no top-K, orçamento por estágio.
- **Não determinismo.** Saída de modelo é Grade B por definição: nunca
  aciona slash nem vira fato de consenso (P1).
- **pt-BR.** Validar os modelos NLI e de extração em português antes de
  adotar; XNLI-pt entra na fase 1 por isso.

## 10. Decisões que são suas

1. **Backend do `ClaimExtractor`.** Recomendo LLM via API atrás do trait,
   com fallback "só pacote de evidência" quando não houver chave. Um modelo
   local de QA extrativo é possível, mas cobre menos e adia o resultado.
2. **Pesquisas eleitorais/TSE.** Recomendo manter como plugin e congelar.
   Reescrever agora não ajuda a generalização.
3. **Congelamento da seção 7.** O plano assume que você aceita parar
   economia, whitepaper e pitch por cerca de seis semanas.
