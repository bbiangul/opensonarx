# Execução sistemática — 9 de outubro de 2026

Continuação posterior deste registro:
[EXECUTION-CYCLE2-2026-10-09.md](EXECUTION-CYCLE2-2026-10-09.md).
O segundo ciclo confirmou falta de créditos do Serper, preservou a captura
afetada e corrigiu erros adicionais de origem, cache e título versus corpo.
O restante deste arquivo descreve o primeiro ciclo, não a versão final posterior.

## Resultado deste ciclo

Foram implementadas salvaguardas e corrigido roteamento genérico. A missão de
busca geral mais confiável que o Google **não foi demonstrada**. Não houve
pagamentos, implantação financeira, lançamento de token ou ativação de cash
como sinal probabilístico. O gateway protegido P0 continua não implementado.

## Código entregue

- Checklist tipado de requisitos por afirmação, exposto em `AnswerTrace`:
  citações, texto retido, famílias editoriais, escopo de registro quando
  realmente invocado, localização ausente e verificação de diligência.
  Requisitos desconhecidos/ausentes impedem emitir uma afirmação completa.
  O checklist é estrutural, não um certificado de verdade ou independência.
- O rastro conserva a avaliação sobre toda a evidência usada pelo construtor,
  antes de reduzir os links exibidos. Reavaliar uma resposta já esvaziada
  perdia o motivo da abstenção; reavaliar só o top-k perdia citações válidas.
- Citações deixam de inventar um trecho a partir do título: usam trecho
  localizado no texto retido. A seleção lexical não é inferência semântica;
  prova de contexto/negação/qualificadores ainda exige avaliação própria.
- Proteções contra negação de relações de origem e contra uma frase genérica
  herdar uma entidade não identificada pelo título. Um teste adversarial
  falhou antes da correção para `Acme was not founded by Ada Example`.
  O guard de negação é local/lexical, não um motor geral de entailment.
- `official`/`website` são reconhecidos e removidos da identidade da entidade
  em consultas navegacionais; perguntas explicativas não viram consultas de
  resultados atuais apenas por mencionar eleições.
- Descoberta de versões/releases não é forçada para notícias datadas.
  Links sem data ficam como descoberta, não provam a versão atual. A mensagem
  final também não chama esses links de "recentes" sem comprovação.
- Prazo de descoberta: APIs disponíveis com chave têm perfil limitado a 3s;
  provedores públicos mantêm 1,75s. Timeout HTTP continua limitado a 5s.
  A escolha explicita um trade-off, não prova melhoria de SLO; a comparação
  de transferência abaixo não isola causalmente esse parâmetro.
- CLI e comentários identificam créditos de criadores como exposição
  **não monetária/não pagável**, não consumo observado ou dívida ao criador.
  Não existe conversão desses contadores em recebimentos P0.
- A auditoria do índice real isolou perda de evidência ao escolher apenas um
  trecho semântico por URL antes da extração. Foi adicionado acesso limitado
  a trechos de URLs já conhecidas e seleção de um trecho com relação completa
  por URL. Isso não cria novas fontes, não conta trechos como observações
  independentes e reaplica a política de decisão. A API de busca de documentos
  continua deduplicando URLs. O acesso a trechos é limitado a 256 no índice
  e ao orçamento de evidência no SDK; não foi demonstrado custo em web-scale.
- Um auditor local, somente leitura e sem rede, expõe as razões de rejeição
  (relação, entidade, direção, contexto, pergunta e nomes extraídos) do texto
  retido. Parênteses de pronúncia/aliases são ignorados apenas na checagem
  sintática de direção; texto citado e negações continuam intactos.

## Capturas reais, sem mocks no caminho de execução

Embeddings `arctic-embed-s` ONNX reais, binário debug, uma máquina, concorrência
2, top-k 5, índice inicialmente vazio e coleta real de páginas. A medição é
do SDK/adapter de produção, **não de uma implantação em produção ou em escala**.
O conjunto V1 contém 40 perguntas e V2 outras 40, oito estratos cada.
As perguntas foram escolhidas pelo implementador; nenhum juiz independente
avaliou as respostas. Ambos agora são conjuntos inspecionados de diagnóstico,
não holdouts confirmatórios. Seleção e hashes são conservados nos manifests.

| Rodada | Pedidos | Listas não vazias | Respostas factuais completas | Destinos | p95 SDK |
|---|---:|---:|---:|---:|---:|
| V1 provedores públicos, índice vazio | 40 | 13 | 0 | 2 | 3,07s |
| V1 Serper antes, índice vazio | 40 | 30 | 0 | 2 | 8,31s |
| V1 roteamento/checklist corrigidos, primeira passagem | 40 | 29 | 0 | 4 | 3,26s |
| V1 mesma instância, índice populado, segunda passagem | 40 | 29 | 0 | 4 | 1,61s |
| V2 perguntas novas, perfil API 3s, índice vazio | 40 | 26 | 0 | 4 | 9,49s |
| V2 descoberta de releases corrigida, novo índice vazio | 40 | 28 | 0 | 4 | 8,59s |
| V2 seleção de trechos/aliases e correção de panic, novo índice vazio | 40 | 28 | 0 | 4 | 7,82s |

São **280 pedidos completos ao SDK**, não 280 perguntas distintas, além de 80 capturas
da linha de base e smoke tests separados. Zero erro de chamada não é aprovação
de qualidade. Todas as rodadas de 40 terminaram com sucesso operacional.

Linha de base: Google **orgânico via Serper**, 40/40 listas não vazias em cada
conjunto. P95 de recuperação V1 2,07s e V2 1,58s. Não foi capturado o Google AI
Overview/ChatGPT/Claude/Perplexity. Não comparar sua recuperação simples com
o SDK que também constrói/checa respostas como se fossem endpoints idênticos.
As capturas não têm rótulos de verdade, portanto não calculam ganho de acurácia.

V1 contém um erro de metadados: idiomas/locales dos cinco exemplos de pesquisas
eleitorais foram invertidos. O arquivo selado não foi alterado depois da
captura. V2 corrigiu isso e excluiu as consultas anteriores por hash.
Também houve 429/403 em provedores públicos e deadlines do Serper; não foram
contornadas proteções. Cada captura conserva o hash do binário usado.

## Exemplos do último lote amplo

| Pergunta | Antes da correção de releases | Depois | O que não foi provado |
|---|---|---|---|
| latest stable release of Node.js | Sem links | Dois links; primeiro `https://nodejs.org/en`; abstenção | Número da versão atual |
| versão estável atual do Debian | Sem links | Cinco links; abstenção | Oficialidade do top-1 ou versão atual |
| latest release of WordPress | Sem links | Cinco links; abstenção | Versão atual, independente de popularidade |
| who founded CERN | Links e abstenção | Links e abstenção | Lista completa de fundadores |

No V2, transações continuam 0/5 com links e diligência 1/5. Os destinos
navegacionais são inferências de identidade/origem, não verificação de posse
ou certificação de site oficial. O índice populado baixou latência em V1 sem
recuperar sua cobertura. Aumentar abstenção não basta para cumprir a missão.

## Verificação local final

- SDK: 325 testes passaram, seis ignorados, zero falhas na suíte final.
- Index: 176 unidades e quatro integrações passaram.
- Types: 90 testes passaram.
- Economics: cinco cenários/testes passaram; continuam simulações.
- Comparador: três invariantes passaram (mesmo manifest, par ausente e vazio).
- CLI: `cargo check -p oi-cli --no-default-features --offline` passou.
- `git diff --check` passou. Não houve commit nem descarte de trabalho existente.
- Aviso preexistente: função `truncate_url` não utilizada.

As salvaguardas de contexto/negação, mensagem final e seleção de trechos foram
adicionadas **após** o lote V2 de releases acima. O lote anterior não deve ser
atribuído à versão final; cada arquivo conserva seu binário específico.

Os smoke tests `final-pix-regression.json` (frio, provedores mistos) e
`final-pix-resident-serper.json` (índice populado, Serper) falharam: o segundo
ainda se absteve em 3,27s, sem a citação primária exigida. Isso refutou a
hipótese de que a falha era apenas rate limiting. A auditoria do índice real
viu texto de origem fora do trecho escolhido e um aside de pronúncia barrando
a direção sintática. Não se alterou o resultado esperado para aprovar o caso.

Depois da correção estrutural, `pix-chunk-recovery-final.json` passou os
critérios existentes: resposta extrativa com Banco Central, Wikipedia e EBANX
como citações, quatro passagens, expansão primária e 6,41s. É uma regressão
conhecida com índice populado; não prova generalização ou independência das
fontes. O JSON conserva as citações, checklist e caminho seguido.

A primeira tentativa do lote de trechos abortou por panic em uma frase mais
curta que a entidade após remover um aside. A falha foi preservada em
`chunk-fix-attempt1-failure.json`, corrigida nos caminhos de corpo/título e
incluída na regressão. A repetição **com novo índice vazio** terminou 40/40
sem erros: 28 listas não vazias, zero respostas factuais completas, p95 7,82s,
p99 10,35s. O gate continua reprovado. O sucesso Pix não substitui esse lote.
Uma consulta Pix separada coincidiu com o começo da tentativa abortada, não
com a repetição final; não há afirmação de controle experimental ou escala.

Total final das suítes listadas: **600 testes passaram**, mais três invariantes
do comparador; seis testes do SDK foram ignorados. Os últimos checks da CLI e
de whitespace também passaram. A tentativa abortada não entra nos 280 pedidos
completos e não tem um relatório completo de qualidade.

## Próximos gates, sem declarar encerramento

1. Ampliar extração factual e rotas transacionais de forma geral, com razões
   de rejeição observáveis. Hoje conceitos amplos devolvem links e a extração
   factual é restrita a relações específicas; fontes coletadas não bastaram
   para respostas completas nas perguntas novas.
2. Medir aquisição de evidência, cauda fria, releases e rotas de ação em um
   novo conjunto, sem enfraquecer requisitos para fazer um teste passar.
   Executar release build/carga/falhas antes de qualquer afirmação de escala.
3. Nomear dois avaliadores externos para julgar respostas completas e erros
   materiais, preparar >=1.000 consultas e congelar poder/limiares. Os gates
   de qualidade/latência da revisão anterior não foram atingidos aqui.
4. Implementar P0 somente com fontes autorizadas, direitos/segurança revisados,
   consentimento, preço/budget, cache, replay, isolamento, recibos e refunds.
   Telemetria atual não é recibo pagável.
5. Validar demanda/custos observados e o modelo cash condicionado separado.
   Coorte com dinheiro, recrutamento e pagamentos reais dependem de aprovação
   explícita; não habilitar sinal cash em produção a partir de simulações.

Os relatórios completos por pergunta, respostas, links e caminho seguido estão
em `outputs/validation-20261009/`. `scripts/compare_live_captures.mjs` reproduz
o comparativo e recusa workloads de hashes diferentes. Arquivos `paired-*`
conservam as limitações, tempos e resultados por estrato, passagem e pergunta.
