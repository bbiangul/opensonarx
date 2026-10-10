# Resposta direta com selo de confiança — 10 de outubro de 2026

## Pergunta

As duas comparações ao vivo (LIVE-HEAD-TO-HEAD e MISINFORMATION-HEAD-TO-HEAD)
não mostraram a camada de evidência produzindo resposta melhor do que o
mesmo modelo lendo as mesmas páginas. Nas mesmas 240 consultas apareceu, sem
ter sido registrado antes, outro padrão: quando o pipeline chega a uma
resposta completa com duas ou mais famílias de fonte, a resposta direta
quase não erra (31 de 32), contra 113 de 135 no resto.

Pergunta deste teste: **o estado de evidência serve como selo de confiança
sobre a resposta direta?** O formato de produto seria: responder rápido a
tudo com uma chamada, e marcar quais respostas têm corroboração.

## Protocolo (registrado antes da execução)

Escrito e commitado antes de executar ou ler qualquer consulta das amostras.

### Amostras

Duas amostras novas, sem interseção com nada já visto, sorteadas por hash
com o rótulo `trust-label-1`:

- 120 consultas do MS MARCO QA, 12 por assunto (`cases-trust-1.jsonl`,
  sha256 `6d8178e77da0efc618d0369cd8cc2defbdbf987320553c556814125b0942dc97`).
- 120 perguntas do TruthfulQA, categorias "Indexical Error" fora
  (`cases-trust-1.jsonl`, sha256
  `b1fb5484b295e43b62b029899ceddf41d9f309dae28c53af61dbc855fd758cf9`).

### Sistemas

Sem ajuste: o mesmo `oi-bench answer-live`, mesma descoberta
(`duckduckgo,brave-html`), `claude-haiku-5.5`, pipeline com uma família de
fonte e gate, linha de base `direct-answer-v1`. A única mudança no
comparador é um campo a mais no relatório (o selo barato, abaixo); nenhuma
chamada, instrução ou política mudou.

### Os selos

Decididos pelo código, sem modelo a mais além do que cada um descreve:

- **Selo do pipeline (hipótese principal):** o pipeline deu resposta
  completa e a afirmação mais fraca tem duas ou mais famílias de fonte.
  Custa o pipeline inteiro (cerca de 13 s); a resposta direta sai antes.
- **Selo barato (secundário):** as citações da própria resposta direta,
  localizadas literalmente em trechos retidos, vêm de duas ou mais famílias
  de fonte. Não custa nenhuma chamada. Na olhada exploratória nos dados
  antigos ele foi fraco (75/84 contra 69/83); entra para registro.

Família de fonte = domínio registrável, como no resto do projeto.

### Correção

Corrijo só as respostas diretas, uma a uma, sem modelo avaliador. A folha
não traz nenhum dos selos nem o status do pipeline, e todas as respostas são
do mesmo sistema, então desta vez a leitura é cega para a variável testada.
Mesmos critérios dos testes anteriores (certa / errada no MS MARCO; C / F / N
no TruthfulQA). "Certa" = C.

### Medida e critério

Para cada selo, entre as respostas diretas emitidas nas 240 consultas:

- taxa de erro com selo e sem selo;
- diferença (sem selo − com selo) com bootstrap de 95%.

**O selo vale se o limite inferior da diferença for maior que zero nesta
amostra sozinha.** Senão: não demonstrado. Também relato: quantas respostas
recebem o selo, o resultado separado por amostra, e a soma com as 240
consultas anteriores (descritiva, não decide).

Nenhum corte será ajustado depois de ver os resultados. O corte "duas ou
mais" é o da política original do projeto, não foi escolhido nesses dados.

### O que este teste não mede

- Se o selo funciona com outro modelo, outro idioma ou busca com chave.
- Independência real entre fontes: dois domínios podem copiar um do outro.
- Um leitor, que é um modelo.

## Resultados

Pendente.
