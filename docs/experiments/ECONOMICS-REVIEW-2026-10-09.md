# Economics: revisão adversarial e cenários locais — 9 outubro 2026

## Conclusão

A quota Serper bloqueia novas comparações com esse provedor, não a execução
local de economics. A matemática não prova viabilidade comercial: custos,
demanda, precisão dos contestadores e frequência de erros abaixo são hipóteses.
Não houve pagamentos, implantação, emissão de tokens ou validação financeira real.

Modelo reproduzível: `python3 scripts/economics_pilot.py --test` e
`python3 scripts/economics_pilot.py`. Captura deste ciclo:
`outputs/validation-20261009/economics-stress-v2.json`.

## Serviço: quem paga e quanto sobra

Hipótese base: US$ 0,03 por evento faturável, US$ 0,0107 por tentativa e
US$ 10 mil/mês de custo fixo. Todas as tentativas custam; nem todas geram receita.
O custo histórico agregado usado no modelo não é uma fatura observada nem uma
decomposição auditada. Não somar pagamentos a nós já incluídos nesse agregado.

| Cenário | Resultado operacional mensal, sem subsídio |
|---|---:|
| 10 mil tentativas, todas faturáveis | −US$ 9.807 |
| 100 mil tentativas, todas faturáveis | −US$ 8.070 |
| 1 milhão, todas faturáveis | +US$ 9.300 |
| 1 milhão, apenas metade faturável | −US$ 5.700 |
| 1 milhão, todas faturáveis, processamento 3% e queima 10% | +US$ 5.400 |

No caso base, equilíbrio exige aproximadamente 518 mil tentativas faturáveis/mês.
Com processamento 3% e queima 10%, a margem cai de 31% para 18%; para margem
30% naquele volume, preço necessário sobe para aproximadamente US$ 0,0363.
Queima de receita é um cenário de estresse, não uma proposta de queimar USDC.

Uma segunda decomposição **ilustrativa e separada** acrescenta ao operador
US$ 0,0107: nós de entrega 0,002; criador licenciado 0,004; verificação 0,001;
blockchain/faturamento 0,001 por tentativa. Com processamento 3%, sobra apenas
US$ 400/mês em 1 milhão de tentativas, margem 1,33%. Para 30%, preço ~US$ 0,0428.
Isso só é válido se esses custos forem realmente distintos, não dupla contagem.

## Personas e condições para participar

| Participante | Remuneração proposta | Condição econômica necessária |
|---|---|---|
| Consumidor/agente/empresa | Paga evento de uso explicitamente contratado | Valor da resposta deve superar preço; abstenção não vira cobrança silenciosa |
| Operador | Receita residual após custos e obrigações | Margem positiva sem emissão/subsídio; financiar tentativas não faturáveis |
| Nó de entrega | Pagamento por entrega aceita | Pagamento > compute, banda, armazenamento, capital e perdas esperadas |
| Criador | Licença de uso escolhida; conteúdo livre pode remunerar zero | Receita ou benefício de audiência justificam participação; autoria/licença não provam verdade |
| Verificador | Taxa por verificação admissível | Taxa cobre verificação independente; US$ 2 recebidos com custo US$ 3 dá prejuízo |
| Contestador | Recompensa por contestação procedente | Valor esperado positivo após investigação, taxa, ambiguidade e capital imobilizado |
| Autor/garantidor | Prêmio por compromisso elegível | Prêmio cobre verificação, risco, capital e reposição; não pode vender a mesma garantia várias vezes |
| Reserva | Recursos restritos, não dividendos | Regra de financiamento e prioridade de pagamentos explícitas; não recuperável pelo atacante |

Exemplo de nó: recebe US$ 0,002 por unidade. Com custo 0,001 ganha 0,001;
com custo 0,003 perde 0,001. Um bônus de token de 0,002 pode esconder a perda,
mas não torna a operação sustentável. O modelo não prevê preço do token.

## Contestações: incentivo, ambiguidade e conluio

Exemplo proposto, ainda não contrato implantado: autor deposita US$ 100;
contestador deposita US$ 10 mais taxa US$ 2. Total US$ 112.

| Desfecho | Autor | Contestador | Verificador | Reserva restrita |
|---|---:|---:|---:|---:|
| Contestação procedente | 0 | 40 | 2 | 70 |
| Improcedente | 102 | 0 | 2 | 8 |
| Inconclusiva | 100 | 10 | 2 | 0 |

Todas as linhas conservam US$ 112. Precisão q é condicionada aos casos
resolvidos; u é a probabilidade de não resolver. Com investigação US$ 10:
EV = (1 − u) × (40q − 10) − 12. Sem ambiguidade, q mínimo = 55%.
Em q=90%: EV US$ 14 sem inconclusivos, US$ 1 com 50% inconclusivos e
−US$ 5,50 com 75%. Custo de capital/tempo do contestador ainda não foi precificado.
Publicar recompensa não basta: sem resolução barata, ninguém racional fiscaliza.

Coalizão controlando autor, contestador e verificador perde US$ 80 no cenário
base (reserva inacessível e custo externo US$ 10). Porém bônus de US$ 85
transforma isso em ganho de US$ 5. Com recuperação de 80% da reserva e bônus
US$ 25, ganho US$ 1. Portanto emissões, rebates e propriedade comum precisam
entrar no teste adversarial. Não existe prova geral de resistência a farming.

## Garantia: capital, admissão e o que o usuário NÃO recebe

Hipóteses: 5 mil compromissos/mês, bond US$ 100, trava 2 dias, buffer 20%,
custo anual de capital 10%, erro 0,1%, detecção 100%, verificação US$ 0,005.
Capital médio com buffer US$ 40 mil; subtotal US$ 858,33/mês; com markup
de custo 20%, prêmio US$ 1.030/mês, ou US$ 0,206 por compromisso elegível.
Markup 20% não é margem 20%. Com erro 1%, prêmio sobe para US$ 1,286.
Não precifica cauda, correlação, administração, custódia, tributos ou indenização.

US$ 40 mil cobrem no máximo 400 bonds simultâneos de US$ 100, antes de uma
política adicional de buffer. Pedido de 500 compromissos simultâneos deve ser
recusado: não é um cenário admissível de garantia integral. Dentro da capacidade,
400 perdas simultâneas custam US$ 40 mil; reserva separada de US$ 30 mil fica
US$ 10 mil abaixo da reposição. Nenhuma probabilidade foi estimada para esse evento.
Reserva separada de US$ 40 mil acrescentaria US$ 333,33/mês de custo de capital
na hipótese anual 10%; esse valor não está no prêmio base.

Uma reserva de perdas só é necessária ao participante que assume reposição ou
obrigações adicionais: definir quem assume esse risco antes de somar capitais.
Não contar os mesmos dólares como bond disponível, reserva e indenização.
No fluxo de disputa acima, **o consumidor recebe US$ 0**. Garantia de compromisso
com penalidade não é seguro da resposta nem compensação por dano ao usuário.

## Blockchain e token: fronteira com a implementação

O default local de `GovernableParams.burn_pct` era 45%, divergindo dos 10%
do inicializador do contrato. Corrigido para 10%, com regressão. O canal legado
destina, nos defaults, 85% ao payee, 5% sentinel e 10% queima; não reutilizar esse
split como política de serviço stablecoin. Payee pode ser um nó, não necessariamente
um criador. As contestações legadas por júri não implementam este piloto determinístico.

Solana cobra SOL na transação: base documentada de 5.000 lamports por assinatura,
mais prioridade opcional. [Fonte oficial](https://solana.com/docs/core/fees).
Com SOL hipotético US$ 100, três transações de uma assinatura custariam
US$ 0,0015 em basefee; lote 100 dilui para US$ 0,000015 por consulta.
RPC, rent, custódia, prioridade e retries pagos estão fora. Isso não mede latência
nem prova a segurança de liquidação em lote.

Recomendação de desenho: pagamento do serviço em unidade estável; token opcional
apenas com função e orçamento explícitos. Não exigir token para descobrir conteúdo,
não usar valorização esperada para fechar a conta e não ativar bônus que façam
uma coalizão lucrar. Blockchain pode liquidar compromissos verificáveis; não
transforma evidência fraca em verdade e não deve entrar no caminho de cada busca.

## Restante do trabalho: independente de créditos vs dependente

- **Executado localmente neste ciclo:** modelo de faturabilidade, volume, atores,
  subsídios, ambiguidade, conluio, admissão e cauda; default legado alinhado.
- **Ainda implementável localmente:** P0 protegido com autorização, recibo e
  termos de uso; contabilidade em unidades inteiras; admissão com exposição por
  evento/origem/funder; estados elegível/inconclusivo/resolvido; prevenção de
  replay/dupla cobrança; retenção de artefatos e rastreamento de custos.
- **Não entregue:** esses fluxos P0 e liquidação determinística de ponta a ponta;
  o modelo Python usa floats de cenário, não serve como ledger financeiro.
- **Pendência adicional:** helper legado `compute_revenue_shares` aceita percentuais
  arbitrários; soma >100 pode provocar underflow. Precisa validação fail-closed,
  não ajuste silencioso de percentuais nem tratamento como configuração válida.
  **Resolvida no ciclo subsequente:** o helper retorna erro explícito, e o split
  local do contrato também valida percentuais/usa u128. Ver
  [implementação e limites](ECONOMICS-IMPLEMENTATION-2026-10-09.md).
- **Depende de recursos externos:** comparação nova com buscador disponível,
  custos observados, revisão independente, demanda/pagamento de clientes e
  experimento financeiro autorizado. Créditos não resolvem essas outras evidências.

Critério de avanço: serviço e participantes economicamente sustentáveis sem bônus;
coalizões não lucrativas incluindo todos os benefícios; nenhuma promessa sem
capital/escopo; qualidade fora da amostra superior ao baseline antes de afirmar
que dinheiro aumenta confiança. A tese continua aberta, não invalidada nem provada.

## Verificação deste ciclo

- Modelo econômico: 14 testes passaram.
- Rust oi-types: 91 testes passaram; oi-staking: 26 passaram.
- SDK sem features default: 330 passaram, seis ignorados.
- Total dessas suítes: 461 testes aprovados, sem falhas.
- JSON de cenários válido e `git diff --check` passou.
- Aviso preexistente de `truncate_url` não utilizado permanece.

São testes locais de regressão/cenários. Não são 461 testes financeiros de
produção nem nova comparação com Google; nenhum dinheiro real foi movimentado.
