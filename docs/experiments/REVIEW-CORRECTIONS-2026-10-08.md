# Correções da revisão crítica do OpenSonarX

**8 de outubro de 2026. Whitepaper v2.8, apresentação v7.2.**

Este registro separa correção de código, especificação e prova empírica.
Não houve implantação financeira nem nova captura de buscadores de mercado.

| Problema | Correção entregue | O que ainda falta |
|---|---|---|
| Dinheiro contava competência/seleção novamente | O estimador desativa a atualização marginal, preserva associação apenas como diagnóstico e expõe o motivo da contribuição zero. A especificação exige modelo conjunto/condicionado sobre a mesma base sem cash. | Implementar e validar esse modelo em uma coorte real pré-registrada. Desativação não prova efeito financeiro. |
| Volume diluía a evidência ausente | Massa observada limitada, tratamento conservador de cobertura inválida e flag genérica de requisito crítico. Testes impedem probabilidade apesar de histórico forte ou cash. A política de busca recebe nova versão. | Integrar o checklist crítico tipado de ponta a ponta na busca e testar cobertura privada/paga. A flag genérica não descobre sozinha os requisitos. |
| Contestador perdia mesmo acertando | Nova proposta de recompensa US$30, US$18 líquidos após custo ilustrativo US$10. Fórmula inclui precisão e casos inconclusivos. Testes conservam os US$112 em todos os resultados. | Medir custos/precisão reais, independência dos participantes, colusão, acesso à reserva e financiar auditorias. Contratos e pagamentos não foram implementados. |
| Garantia objetiva confundia-se com verdade geral | Escopo explícito, probabilidade condicionada, medição da fração elegível e comparação com gastar o mesmo orçamento em evidência primária. | Demonstrar utilidade incremental em consultas reais. Um registro ou extração fiel não certifica verdade no mundo. |
| Política de criadores não escolhia evento cobrável | Perfil P0 de entrega autorizada, janela de cache, destinatários/valores aceitos, recibos não pagáveis para aberto/citação, direitos e refunds. Custos separados por papel. | Implementar gateway protegido, revisão de direitos, autorização, isolamento, replay/refund e reconciliação. Não há mercado pago validado. |
| Escopo excessivo e dependências | Primeiro produto: API operada sobre texto/dados, workflow concreto e fontes autorizadas. Token, treino, multimídia e rede global fora da primeira entrega. | Recrutar compradores e executar piloto. A ambição de busca geral permanece e exige holdouts amplos. |
| Economia e superioridade não demonstradas | Modelo de sensibilidade refaz margens com custo adicional e detecção integral. Critérios numéricos de qualidade, cobertura, latência, demanda e cash separados. Resultados históricos permanecem identificados. | Custos observados, reuso pago e avaliação externa com respostas completas. Nenhuma superioridade ou novidade científica foi estabelecida. |
| Pitch longo e demo fraca | 12 slides principais, 27 de apêndice. Inclui o traço eleitoral histórico, mantém a abstenção Gama, critérios finitos e números corrigidos/editáveis. | Demo ao vivo comparável com respostas completas e benefício mensurável. Traço histórico não substitui uma nova comparação. |

## Verificação local realizada

- `cargo test -p oi-index --offline`: 175 testes de unidade e quatro testes de integração passaram. Inclui 15 testes de uncertainty.
- `cargo test -p oi-sdk --no-default-features evidence::tests --lib --offline`: 19 testes passaram. Houve um aviso preexistente de função não utilizada em `client.rs`.
- `cargo test -p oi-bench --no-default-features uncertainty --offline`: dois testes passaram.
- `python3 scripts/economics_pilot.py --test`: cinco testes passaram.
- Total dessas suítes: 205 testes, sem falhas. Não é a suíte completa de todos os crates.
- Benchmark sintético corrigido: 20.000 casos de treino e 20.000 de avaliação. Brier 0,2175147929 e log loss 0,6274750431 em ambas as variantes. Ganho cash: exatamente zero, com status explícito de modelo desativado. Mede a salvaguarda, não valida um modelo condicionado.
- Apresentação: 39 slides, 17 tabelas nativas e um gráfico nativo. Validação de pacote/layout sem achados. Cada slide foi inspecionado em renderização e o gráfico corrigido recebeu nova conferência. Não houve inspeção no PowerPoint nativo.
- Verificação cruzada mantém os números históricos de agosto, conserva distribuições, verifica a escala e os valores do gráfico e confirma a preservação da apresentação fonte.

## Ordem do trabalho restante

1. Integrar checklist tipado e entregar a API de texto/dados com rastros verificáveis.
2. Recrutar os participantes e nomear os avaliadores independentes antes de congelar o piloto.
3. Executar holdouts de respostas completas, medir custos de ciclo de vida e obter reuso pago.
4. Implementar P0 protegido após aprovação de direitos/segurança.
5. Desenvolver e testar o modelo condicionado de cash e, somente depois de aprovações próprias, a liquidação financeira.

O protocolo completo e seus critérios de interrupção estão em
[PILOT-GATES-2026-10-08.md](PILOT-GATES-2026-10-08.md).

## Atualização de execução — 9 de outubro

O checklist tipado foi integrado ao construtor e ao rastro da busca. Houve
capturas reais em 80 perguntas distintas, repetidas em ciclos de diagnóstico,
correções de roteamento e proteção contra negação/contexto indevido.
**Não** foi demonstrada superioridade, pagamento P0, ganho cash ou escala.
O registro de 8 de outubro acima continua sendo histórico. Medições, limites
e próximos gates estão em [EXECUTION-2026-10-09.md](EXECUTION-2026-10-09.md).
