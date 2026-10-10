# Crawling, entidades e afirmações: comparação local — 9 outubro 2026

## Escopo e reprodutibilidade

Piloto exploratório com componentes reais, não teste confirmatório de produção.
Quatro URLs de documentação (MDN inglês/português, Python, Rust), oito perguntas
escolhidas pelo implementador e três variantes. Não compara Google ou P2P.

- A: texto original, chunks e embeddings.
- B: A mais entidades extraídas por GLiNER real.
- C: A mais unidades de recuperação propostas por modelo, cuja citação foi
  localizada literalmente. O embedding adicional representa a afirmação, mas
  o conteúdo indexado continua sendo contexto original da fonte. Não indexamos
  a geração do modelo como se fosse texto original ou verdade comprovada.

Cada página foi baixada uma vez pelo fetcher do projeto, após checar robots;
HTML e Markdown foram preservados. A rodada corrigida reutiliza esses mesmos
HTMLs. São os primeiros oito chunks por página, de 256 palavras com overlap 64:
32 dos 83 chunks gerados. É orçamento fixo comum, não avaliação da página inteira.

Embeddings Arctic Embed S INT8 ONNX, sem fallback mock. Entidades:
GLiNER multi-v2.1 ONNX **quantizado**, threshold 0,5. Não é o modelo full-precision
default. Afirmações e julgamentos: Ollama local `gpt-oss:20b`, `think=low`.
Chamadas pagas de modelo/Serper: nenhuma. Custo de compute/energia não foi medido.

O SDK real faz busca local, seleção semântica de passagens, extração, stance,
responsividade e montagem. Não há descoberta externa, hidratação adicional ou
rede P2P durante as perguntas. Política de uma família de fonte para esse corpus;
limite padrão de três afirmações; ordem das variantes rotacionada por pergunta.
Não há correções de produção, prompts ou thresholds feitas para passar os casos.

```sh
cargo build -p oi-bench --example crawl_ablation --example ner_smoke_measure --features onnx,facts-onnx --offline
target/debug/examples/crawl_ablation bench/fixtures/crawl-ablation-local-v1.json /private/tmp/opensonarx-gliner-measure outputs/NOVO-DIRETORIO
```

Para repetir sobre os mesmos bytes, passar como quarto argumento o diretório
de snapshots da primeira rodada. O comando recusa sobrescrever uma rodada existente.

Artefatos:

- `outputs/crawl-ablation-20261009-local-v1/`: aquisição real e diagnóstico inicial.
- `outputs/crawl-ablation-20261009-local-v2/`: repetição corrigida sobre os mesmos bytes.
- `outputs/crawl-ablation-20261009-local-v3/`: guard detectou ausência de projeções
  ativas; interrompido antes das respostas, sem relatório de qualidade.
- `outputs/crawl-ablation-20261009-local-v4/`: C efetivamente indexável, replay
  das propostas reais da v2; oito perguntas repetidas, sem reextrair afirmações.
- Harness: `bench/examples/crawl_ablation.rs`; controles: `ner_smoke_measure.rs`.

SHA256 do binário v2: `e3d8f18035370e887985757c3a2ee8647a8b08c729b25e4099b6ae8e4fa6c9a3`.
SHA256 do binário v4: `55cf9988582b65340a7e5fcb5d8562b57132f27e9e52b16ef8464d5f8299d4a0`.
Manifesto: `e34e8667250c333224a88f688cf4e52b72cd7127117a5b716b8aa2f3f50e21a8`.
Pesos GLiNER: `995058c82c5f570601dd8a0ba74ee60a392f764268bc5f628455e44dd3b476ec`.
Tokenizer: `914bd3c8fb7b525af9e23b60d0ec7b1248ddb2b99014efd9c02ebeb022f8cab7`.
Modelo Ollama observado: digest `17052f91a42e97930aa6e28a6c6c06a983e6a58dbb00434885a0cf5313e376f7`.

## Tempos observados de ingestão

Download inicial: média 300,94 ms/página, faixa 48,24–573,77 ms. Exclui checagem
de robots e não é orçamento completo de crawling. Tempos a seguir são médias
da v2 sobre o HTML retido, sem download, startup dos modelos ou inserção no índice:

| Etapa | Média por página, oito chunks |
|---|---:|
| HTML → texto, qualidade, hash e chunking | 118,52 ms |
| Embeddings comuns | 236,05 ms |
| Entidades, custo adicional B | 2.211,75 ms |
| Afirmações, custo adicional C | 32.769,39 ms |
| Embeddings das afirmações, adicional C | 141,02 ms |

Logo, processamento A ~0,35 s; B ~2,57 s; C ~33,26 s, nas condições desta máquina
e execução. Não somar o download de outra rodada e chamar isso de latência E2E.
São quatro observações, uma execução por etapa: não um SLO, teste de carga ou
estimativa financeira. O modelo local, caches e atividade concorrente da máquina
influenciam os tempos; a v1 teve extração de afirmações de 12,14 a 26,79 s/página.

Bytes de chunks serializados (incluem vetores, não arquivo de índice nem RSS):
A 220.846; B ~223 mil; C-v4 208.459. C-v2 tinha 373.260 bytes de objetos
tentados, mas as unidades adicionais não entravam no índice. Não é consumo
real de índice nem evidência de um índice maior funcionando.

## Qualidade da extração

**B falhou o controle positivo.** Zero entidades nos 32 chunks, sem erro técnico.
Em quatro textos de controle EN/PT, encontrou zero das 12 menções esperadas:
Microsoft, Bill Gates, Paul Allen; Barack Obama, Honolulu, Hawaii; Petrobras,
Rio de Janeiro, Brasil; Maria Silva, Microsoft, São Paulo. O extrator é real;
os textos são controles sintéticos, não mocks. Resultado detalhado em
`v1/ner-positive-controls.json`. A causa não foi diagnosticada; não generalizar
esse resultado para GLiNER full-precision ou outras implementações.

Assim, B neste teste acrescentou custo e metadados vazios, não entidades úteis.
Diferenças de resposta A/B não demonstram benefício de NER, pois o modelo de
resposta pode variar mesmo com evidência equivalente.

**C localizou citações para 22 das 25 propostas processadas; rejeitou três.**
Isso mede existência de citação, não precisão factual ou fidelidade semântica
da afirmação. A página Rust forneceu só uma unidade adicional, mostrando que
extração genérica e limite de orçamento não garantem cobertura de todas as relações.

## Integridade do experimento

A rodada v1 foi interrompida durante as respostas: o harness tinha usado limite
de duas afirmações, mas uma pergunta pedia três estados. Foi erro de desenho,
não falha que possa ser atribuída à política padrão. Preservamos os artefatos e
repetimos com três na v2, sem mudar manifesto ou prompts. Não agregamos as
respostas v1 com v2 nem reportamos v1 como comparação completa.

Caso 7, Rust especificamente em Marte, tem rótulo negativo discutível: a regra
da linguagem pode ser universal. Deve aparecer separado, não na taxa principal
de respostas indevidas em negativos inequívocos. Esse problema foi registrado
antes de receber suas respostas na v2. Os outros negativos pedem história/taxas
financeiras que os documentos não informam.

## Perguntas e respostas

**Auditoria da representação:** o índice limita ingestão a oito chunks por URL
e rejeita hashes repetidos. Na v2, anexar projeções dos mesmos chunks depois dos
oito originais não as torna recuperáveis. O replay auditado tentou 54 unidades
e reteve 31, sem projeções. Não usar C-v2 para concluir sobre utilidade das afirmações.

A v4 respeita o limite de oito slots por URL: até quatro contextos derivados
das primeiras propostas aceitas, com os demais slots para chunks ordinários.
O contexto contém a citação e até 300 bytes de cada lado, com fronteiras Unicode
válidas, hash do conteúdo real e URL original. Não falsificamos hash para burlar
dedup nem desativamos o cap/SimHash. O critério não depende da pergunta/resultado.
Na auditoria: A/B 31 chunks ativos de 32 tentados; C 32 ativos, incluindo **13
projeções de afirmação**. Os 22 candidatos citados não significam 22 projeções ativas.

Comparação final: A/B da v2, C da v4. Mesmo HTML, mesmo manifesto, mesmo SDK
`1d09bfa`, mesmo modelo e política de resposta. C foi executado depois, não em
pareamento simultâneo; warming, ordenação e nondeterminismo não foram isolados.

| Pergunta respondível | A texto | B entidades | C contextos de afirmações |
|---|---|---|---|
| Três estados de Promise, inglês | Incompleta: só pending/fulfilled | Mesma incompletude | Três estados |
| Três estados de Promise, português | Três estados | Três estados | Três estados |
| Quando o else do try em Python é executado? | Resposta correta | Resposta correta | Abstenção |
| Quantos owners um valor Rust pode ter por vez? | Um | Um | Um |

Resultado qualitativo lido pelo implementador: **3/4 respostas completas corretas
em cada variante**. Não é avaliação independente. O match de keywords do harness
também dá 3/4; não foi usado sozinho como prova semântica. A/B emitiram uma
resposta parcial e **marcaram `complete_answer_supported=true`** no caso inglês.
C trocou esse erro por uma abstenção em Python, não obteve ganho agregado.

Nos três negativos inequívocos, as três variantes abstiveram-se: 0/3 respostas
indevidas em cada uma. Não estima risco populacional com três casos. O caso
ambíguo de Marte: A/B emitiram a regra universal de Rust; C absteve-se. Excluído
das taxas principais conforme anotação anterior às respostas desse caso.

| Latência observada só da consulta local + resposta | A | B | C |
|---|---:|---:|---:|
| Mediana, oito consultas | 15,11 s | 14,77 s | 2,65 s |
| p95 nearest-rank, oito consultas | 46,39 s | 34,90 s | 17,43 s |

Com oito observações, esse p95 é o máximo observado, não um SLA. C absteve-se
em cinco consultas versus três em A/B, tem contextos menores e rodou depois.
Não atribuir causalmente toda a redução ao pré-processamento. O custo de extrair
afirmações na ingestão (~32,77 s/página nesta v2) não está nesses tempos de consulta.

Nenhum erro de modelo nas 24 respostas da comparação final. Nenhum recurso
mockado foi usado no fluxo. A v4 reutiliza propostas geradas pelo modelo real,
não afirma que essa extração aconteceu novamente em alguns milissegundos.

## Verificação dos artefatos e limites restantes

Harness compilado e executado com ONNX/Ollama reais; JSONs preservados por pedido.
`git diff --check` passou. O warning preexistente de import `Fact` não utilizado
permanece. As duas correções do harness (limite de afirmações e efetividade da
projeção no índice) estão documentadas; não ocultamos as rodadas invalidadas.
Não houve commit automático nem alteração no código de produção.

São 32 pedidos completos de SDK nas v2/v4: 24 compõem a comparação final e
oito C-v2 são diagnósticos de uma representação inefetiva. V1 parcial e V3
interrompida não entram na comparação. Faltam repetições e julgamento externo.

## O que a medição permite concluir

Aquisição de texto, reconhecimento de entidades e extração de afirmações são
operações diferentes com custos e condições de qualidade diferentes. Não
substituir ausência de erro de runtime por validação do extrator. Citação
literal, por sua vez, não prova que a afirmação está completa, responde à
pergunta ou expressa verdade no mundo.

Os dados atuais justificam avaliar extração de afirmações seletiva/assíncrona e
cache versionado, não colocá-la indiscriminadamente no caminho de cada busca.
Para julgar ganho geral, faltam corpus diverso, anotação independente de
entidades/afirmações/qualificadores, repetições, carga, modelos alternativos e
avaliação com a política de produção. Não houve mudança em ranking, pagamentos
ou confiança financeira em função deste piloto.
