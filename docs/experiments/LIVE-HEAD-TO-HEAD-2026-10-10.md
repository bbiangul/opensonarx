# Comparação direta na web real — 10 de outubro de 2026

## Pergunta

Com as mesmas páginas e o mesmo modelo, a camada de evidência do OpenSonarX
entrega respostas mais confiáveis do que pedir a resposta direto ao modelo?
E quanto custa em cobertura e em tempo?

## Protocolo (registrado antes da execução)

Esta seção foi escrita e commitada antes de qualquer consulta da amostra ser
executada.

### Amostra

- 120 consultas do MS MARCO QA v2.1, 12 por assunto em dez assuntos
  (rótulo do roteador de tópicos), sorteadas por hash com o rótulo
  `live-head-to-head-1`, sem interseção com nenhuma consulta já vista (3.479
  excluídas). Arquivo `cases-live-1.jsonl`, sha256
  `5909c04d8d098bf0fd255800f108fa969fb3392810cdef3c870d4f1d4e9ae919`.
- Só a consulta e as respostas de referência são usadas. Os trechos do
  conjunto não entram: as páginas vêm da web de hoje. A marca "com resposta /
  sem resposta" do conjunto não decide nada.

### Os dois sistemas

Para cada consulta, uma única aquisição: descoberta por DuckDuckGo e Brave
(HTML), busca das páginas, texto retido. Sobre os mesmos trechos:

- **Pipeline:** `search_answered` com `claude-haiku-5.5`, política
  `answer-evidence-v2` com uma família de fonte, gate de completude. Resposta
  = `extractive` (completa) ou `partial_answer`.
- **Linha de base (`direct-answer-v1`):** uma chamada ao mesmo modelo com os
  mesmos trechos, pedindo resposta em uma a três frases com até três citações,
  ou "não respondido". É a linha de base forte: pode se abster.

O que a comparação não mede: um produto concorrente com descoberta própria.

### Correção

Eu leio e corrijo cada resposta emitida pelos dois sistemas, uma a uma, sem
modelo avaliador. A folha mostra consulta, referências, resposta e trechos
citados, em ordem embaralhada, sem dizer de qual sistema veio nem o status.
O formato da resposta pode denunciar o sistema; isso é um limite.

Critério, o mesmo de READER-GRADED-SUBJECTS: certa quando responde ao pedido
como feito, é sustentada pelo que cita, não contradiz referência plausível e
não afirma nada claramente errado. Na dúvida, errada. Para fato que muda com
o tempo, vale o que as páginas de hoje dizem.

### Medidas

1. **Primária — erro com a mesma cobertura.** Nas consultas em que o pipeline
   deu resposta completa e a linha de base respondeu, taxa de erro de cada
   um. Critério do PILOT-GATES: redução relativa de pelo menos 20% com limite
   inferior positivo no bootstrap pareado de 95%. Se o intervalo incluir
   zero: **não demonstrado**.
2. Cobertura: quantas consultas cada sistema respondeu.
3. Taxa de erro de cada sistema sobre tudo o que emitiu (completas e
   parciais em separado).
4. Acerto da linha de base nas consultas em que o pipeline se absteve contra
   as em que respondeu: a abstenção aponta para onde a resposta direta erra?
5. Latência p50 e p95 de ponta a ponta, uma consulta por vez.

Nenhum parâmetro será ajustado depois de ver estes resultados. Se algo
falhar na execução, o defeito é corrigido e a amostra inteira é refeita com
consultas novas.

## Achados antes da execução (teste com consultas já vistas)

- **A descoberta padrão sem chave está quebrada.** O primeiro motor da fila é
  o Bing por scraping, e ele devolve resultados sem relação com a consulta
  (para "what size bag to carry on plane" vieram páginas de ajuda do
  Windows 11). Como ele devolve *algo*, a fila não passa para o próximo
  motor. Brave (HTML) e DuckDuckGo devolveram resultados corretos nas mesmas
  consultas. Por isso esta medição fixa `duckduckgo,brave-html`. Não havia
  chave do Serper na máquina.
- **Ordem dos campos no esquema da linha de base.** Com `required` em ordem
  diferente da de `properties`, o modelo às vezes devolvia "respondido" com
  texto vazio. Alinhei as duas ordens e a linha de base repete a chamada
  quando isso ocorre. Os esquemas do pipeline têm a mesma divergência, mas a
  repetição de 16 extrações reais, quatro vezes em cada ordem, deu o mesmo
  número de extrações vazias (25 de 64 nas duas): o pipeline não é afetado.

## Resultados

Pendente.
