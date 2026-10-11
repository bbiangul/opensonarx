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

Pendente.
