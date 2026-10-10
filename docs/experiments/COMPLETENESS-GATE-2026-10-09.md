# Gate de completude: implementação e validação de desenvolvimento

## Resultado

Implementamos separação entre apoio às afirmações e cobertura da pergunta inteira.
São correções de contrato, não uma demonstração de superioridade sobre Google.
O teste real encontrou regressões e falsos negativos que permanecem abertos.

Mudanças:

- `build_passages` usa o trecho contínuo de `content_md`, não o resumo descontínuo
  de `content_relevant`. Mantém o orçamento existente de 3.000 caracteres; não
  reconstrói o documento inteiro nem resolve contexto perdido entre chunks.
- Montagem não pode emitir resposta extractive com requisitos críticos pendentes.
  Julgamentos de stance ausentes por índice também são tratados como desconhecidos.
- Uma resposta apoiada é candidata `partial_answer` até uma checagem semântica da
  resposta final, depois de filtrar afirmações fora do pedido. Aprovação explícita
  é necessária para `extractive` e `complete_answer_supported=true`.
- Erro ou ausência do verificador não aprova completude. Erro de chamada não é
  cacheado. O checklist registra `whole_request_covered`; trace mede `completeness_ms`.
- O harness de answer-eval usa o mesmo gate e compartilha avaliações de cobertura
  somente quando o texto final é idêntico. Erros não são silenciados no relatório.

Não há regras específicas para Promise, Rust, Python ou termos das perguntas.
A checagem é modelada e falível, não uma prova lógica nem garantia de verdade.

## Execução

14 perguntas pelo SDK real: 8 regressões e 6 novas perguntas. Embeddings Arctic
S INT8 ONNX e Ollama `gpt-oss:20b`, `think=low`, sem mocks nas duas rodadas E2E.
Os testes unitários isolam contratos com implementações controladas; não são
contados como validação de qualidade real.

Reutilizamos os mesmos HTMLs reais de quatro documentos, os primeiros oito
chunks por URL, uma família de fonte por política de teste e nenhuma descoberta
externa. Afirmações de ingestão foram replay de captura real anterior; extração,
stance, responsividade e completude da consulta foram novas chamadas reais.
Apenas a variante plain foi consultada; NER foi carregado e processado pelo
harness, mas não faz parte das evidências usadas nessas respostas.

As seis perguntas novas foram congeladas antes da execução em
`bench/fixtures/crawl-completeness-transfer-v1.json`. São casos de desenvolvimento
no mesmo corpus conhecido, não holdout selado nem amostra representativa da web.

## Qualidade observada, inspeção manual

Regressão de oito casos:

- Promise EN agora apresentou os três estados; antes omitia rejected e dizia
  estar completa. Promise PT e cláusula else de Python também responderam.
- Rust, um dono por vez: absteve-se apesar da regra aparecer no trecho recuperado.
  Não há trace suficiente dos julgamentos brutos para atribuir com certeza a
  causa a extração ou stance. Isso é regressão de cobertura, não vitória.
- Três perguntas sem base (invenção por Ada Lovelace e taxas fictícias) abstiveram.
- O caso ambíguo “em Marte” apresentou a regra geral como **parcial**, não completa.
- Resultado das quatro perguntas respondíveis: 3/4 completas corretas. A rodada
  anterior também tinha 3/4 completas corretas, com falha em outra pergunta.

Transferência, seis perguntas novas:

| Pergunta | Resultado | Avaliação |
|---|---|---|
| Três regras de Rust, EN | Sem resposta | Recuperou chunk que começa no final da lista; perdeu as duas regras anteriores |
| Três regras de Rust, PT | Completa | Três regras presentes; respondeu em inglês, limitação de idioma/UX |
| Python: quantos handlers e exceções em outros handlers | Completa | Respondeu às duas exigências |
| Promise resolved implica fulfilled? | Parcial | Texto responde corretamente; o gate de cobertura rejeitou indevidamente |
| Estados de Promise e taxas anuais | Sem resposta | Não inventou taxa nem aprovou completude |
| Regras de Rust e datas exatas de adoção | Sem resposta | Não inventou datas nem aprovou completude |

Portanto 2/4 perguntas novas respondíveis tiveram resposta completa correta;
uma resposta correta foi rebaixada pelo novo verificador. As duas perguntas
parcialmente respondíveis não tiveram completude falsa. Isso não estabelece
uma taxa de segurança: a amostra é muito pequena e foi escolhida pelo implementador.

## Latência observada da consulta

| Rodada | n | Mediana | Máximo / p95 nearest rank |
|---|---:|---:|---:|
| Regressões | 8 | 10,72 s | 26,33 s |
| Perguntas novas | 6 | 24,09 s | 73,38 s |

Checagem adicional de cobertura: 1,21–1,75 s em quatro candidatos da primeira
rodada; 3,71–8,59 s em três candidatos da segunda. Outros casos não chegaram
ao gate. Não comparar essas medianas causalmente: perguntas, número de claims,
warmup e carga diferem. Compilações/testes ocorreram na mesma máquina durante
as execuções. Não é um SLO, benchmark de carga ou avaliação de produção.

## Verificação e artefatos

- SDK: 158 aprovados, 5 ignorados; tipos: 91 aprovados.
- `cargo check -p oi-bench --offline`: passou; build ONNX do example: passou.
- `git diff --check`: passou. Não executamos suíte inteira do workspace.
- `outputs/crawl-ablation-20261009-completeness-v1/report.json` e `case-*.json`.
- `outputs/crawl-completeness-transfer-20261009-v1/report.json` e `case-*.json`.
- Patch final: `outputs/crawl-completeness-transfer-20261009-v1/final-source-changes.patch`.
- HEAD base: `1d09bfa`; execução com alterações locais, não commit de release.
- Binário da rodada de transferência: SHA256
  `1f163d9ec5b18fb6022b07af94ef05ff682ece72eebb70d03461634e75398c09`.
- Manifesto de transferência: SHA256
  `d66aa44b25d5aefc4af7adda1786f936619d2c2df71b56a7d3812307f8e30f52`.

O patch final inclui testes adicionados após o build. A primeira rodada rodou
um build anterior à formalização do requisito de cobertura como desconhecido
na montagem; retenção de contexto, chamada de cobertura e promoção final já
estavam presentes. Preservamos ambas as rodadas, sem sobrescrever resultados.

## Próximos bloqueios

1. Recuperar contexto adjacente de chunks sem misturar versões/documentos,
   mantendo orçamento, rastreabilidade e deduplicação de famílias de fontes.
2. Diagnosticar falsos negativos dos modelos com rastros de decisão suficientes;
   validar lógica de alternativas e perguntas multipartes em novos corpus.
3. Reduzir latência com medidas repetidas e carga controlada, sem remover gates.
4. Diagnosticar GLiNER quantizado separadamente; a falha de controle positivo
   anterior continua aberta. Nada nesta rodada validou sua extração.
5. Depois, corpus holdout mais amplo, fontes adversariais e comparação externa.
   Não testamos rede P2P, incentivos econômicos, staking, pagamentos ou disputas.
