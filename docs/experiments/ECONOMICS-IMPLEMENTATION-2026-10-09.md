# Correções econômicas locais — 9 outubro 2026

## Entregue neste ciclo

A revisão deixou de ser só uma planilha: há controles Rust executáveis,
sem ativar pagamentos, token rewards ou cash confidence.

1. **Alocações inválidas falham:** queima/sentinel acima de 100% e percentuais
   de emissão inválidos retornam erro. Não normalizar percentuais silenciosamente.
   Teste exaustivo de todas as combinações válidas em seis valores, incluindo u64::MAX.
2. **Overflow não vira saldo:** cálculos de yield verificam multiplicação e
   conversão; valores não representáveis retornam erro em vez de truncar.
3. **Contrato legado protegido:** split do canal usa u128 e valida a configuração
   antes de pagar. Evita overflow de `actual_spent * percentage` e underflow do resto.
   Não houve implantação; é uma correção do código local do contrato.
4. **Preço em unidades inteiras:** `minimum_service_price` considera custos
   variáveis de todas as tentativas, custo fixo do mesmo período, fração faturável,
   reembolsos, processamento e margem. Arredonda preço para cima. Subsídio e
   valorização de token não entram como receita sustentável.
5. **Pagamento aos participantes:** rejeita pagamentos inferiores a custos
   declarados de operação/capital/perdas mais surplus positivo. Isso valida a
   conta fornecida, não comprova que o custo declarado foi observado.
6. **Incentivo para fiscalização:** valor esperado inclui ambiguidade, precisão
   condicionada à resolução, investigação, taxa e capital. Não prever precisão
   por reputação nem tomar um EV hipotético como evidência empírica.
7. **Bônus de conluio:** política rejeita cenários em que uma coalizão pode empatar
   ou lucrar, incluindo recuperação da reserva e todos os bônus declarados.
8. **Admissão de garantias:** `CollateralBook` trava um bond por compromisso,
   respeita capital disponível e limite de exposição por cluster. Inconclusivo
   devolve o bond; procedente o retira do capital disponível. IDs encerrados não
   podem reabrir e uma liquidação não pode ser repetida.
9. **Distribuição inteira:** `dispute_payout` conserva o total já provisionado
   em todos os desfechos, sem inventar indenização ao consumidor.

Código: `crates/oi-staking/src/pilot_economics.rs`.

## Fluxo executável e resultado

```sh
cargo run -p oi-staking --example economics_pilot --offline --quiet
```

O exemplo chama o código da biblioteca, não um motor financeiro mockado.
Dados, funding e desfechos são cenários sintéticos: isso não é um teste de
pagamentos reais nem o fluxo completo de busca/licença/entrega.

| Etapa | Resultado observado |
|---|---|
| Preço: 1M tentativas, custo 0,0107, fixo US$ 10 mil, 50% faturável, processamento 3%, margem 30% | US$ 0,061792 por evento faturável; preço antigo 0,03 não cobre o cenário |
| Nó recebe 0,002 com custo 0,003 | Recusado |
| Bond 100, bounty 30, bônus recuperável 85 | Recusado como cenário lucrativo de conluio |
| Fundo 200 e dois bonds 100 em clusters distintos | Admitidos, capital disponível zero |
| Terceiro bond sem funding | Recusado |
| Primeiro compromisso inconclusivo | Bond 100 liberado |
| Segundo compromisso contestado com procedência | Bond 100 forfeitado, não reutilizável |
| Repetir liquidação do segundo compromisso | Recusado |
| Escrow 112, contestação procedente | Contestador 40, verificador 2, reserva restrita 70 |
| Saldo do pool de autor ao final | Disponível 100, travado 0, forfeitado 100 |

O escrow 112 inclui challenge e fee provisionados **fora** do pool de bonds do
autor. O exemplo não registra recebimento desses fundos nem transferência aos
beneficiários. Não confundir allocation book com carteira ou ledger de custódia.
O tempo local registrado foi ~163 microssegundos; inclui prints e só aritmética/
transições. Não é latência de busca, carga, liquidação blockchain ou produção.

## Mudança de API

Métodos econômicos de burn, split, emissão split e yield agora retornam `Result`.
Callers Rust devem propagar/tratar erro. Os callers encontrados no repositório
foram atualizados; usuários externos da biblioteca precisarão migrar. Emissões
e disputas legadas mantêm a semântica anterior; não substituímos júri pelo novo piloto.

## Limites que NÃO estão resolvidos

- `CollateralBook` é in-memory, single-owner; sem journal durável ou recovery.
  O caller ainda precisa verificar funding, autoridade e desfecho; não há
  verificador determinístico de fatos implementado aqui. Reiniciar não pode
  apagar obrigações em um sistema real.
- Cluster fornecido pelo caller não detecta common owner/funder. Identidades
  falsas, hedges externos e bônus omitidos podem evadir a análise de conluio.
- Cotação calcula um piso sob hipóteses; não garante conversão, volume ou margem
  observada. Nem preço maior nem recusar operação transforma demanda em realidade.
- Entrega P0 protegida, licença autenticada, recibo autorizado, billing durável,
  liquidação do piloto e remuneração real continuam pendentes.
- Não há indenização do consumidor. Não vender penalidade como seguro.
- Comparação com mercado, custos observados, revisão independente e experimento
  de participantes reais não foram substituídos por testes locais.

Direção mantida: preço de serviço em unidade estável; nenhuma queima de stablecoin
no piso do piloto; nenhum bônus automático no núcleo econômico. Token continua
opcional e separado de confiança/ranking. Não houve lançamento ou alteração on-chain.

## Verificação

- oi-staking: 36 testes aprovados; oi-types: 91.
- SDK: 330 aprovados, seis ignorados; CLI check passou.
- Contrato opensonarx-staking: 22 testes locais aprovados.
- Modelo Python: 14 testes aprovados.
- Total dessas suítes: 493 aprovados, nenhuma falha.
- Exemplo local de economics passou, incluindo a repetição após o diff final.
- Dois testes adicionais no runtime Solana local (`ProgramTest`, processor
  nativo, não rede pública/SBF): `test_open_and_settle_channel` passou em 0,24 s
  e `test_settle_stale_nonce_fails` em 0,22 s. São tempos dos testes, não SLA.
- Total incluindo os dois testes de runtime: 495 aprovados; `git diff --check`
  passou. Os 82 outros testes desse binário não foram executados neste ciclo.

Aviso preexistente `truncate_url` não usado continua. Não há nova captura Serper,
nem pagamentos reais. Nenhum resultado local autoriza ativar cash confidence.
O runtime local também emitiu avisos de OpenTelemetry ausente; testes passaram.

Antes do commit, a seleção do index foi exportada para um diretório temporário,
sem as demais alterações de busca/documentação da árvore de trabalho. Nessa
seleção isolada, oi-staking (36), oi-types (91), contrato (22) e Python (14)
passaram novamente: 163 testes. Os números de SDK/runtime acima descrevem a
verificação anterior na árvore de trabalho, não uma nova execução no snapshot.
