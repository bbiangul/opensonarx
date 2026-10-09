# Execução — segundo ciclo, 9 de outubro de 2026

## Status

Correções implementadas e testadas. A tese de superioridade geral **continua
não demonstrada**. O Serper ficou sem créditos durante a captura V3; uma única
consulta de diagnóstico confirmou HTTP 400 / `Not enough credits`. Não houve
compra, recarga ou mudança de provedor para contornar o bloqueio. Novas capturas
pagas estão suspensas até o usuário restabelecer o acesso.

## Correções gerais

1. O normalizador já tratava invenção como relação de origem, mas o extrator
   não aceitava todas essas formas no texto. Foram alinhados `invented`/
   `inventou` e `developed`/`desenvolveu`; classificação de desenvolvimento
   só usa essas formas adicionais em perguntas `who`/`quem`, não em tópicos
   como países desenvolvidos. Testes fictícios incluem afirmação e negação.
2. Uma enumeração de fundadores de outra entidade podia tornar incompleta
   a resposta sobre o alvo. Agora a relação deve participar da entidade
   solicitada no contexto delimitado. Continua conservadora para listas
   explicitamente parciais e para frases como “um grupo saiu para fundar”.
   Uma regressão existente falhou na primeira versão da correção e foi
   preservada/corrigida, sem mudar seu resultado esperado.
3. Seleção de trechos completos reaplicada depois das fases de recuperação e
   corroboração: um refresh não deve substituir a relação completa por um
   trecho semanticamente próximo, mas parcial. Limites e contagem de fontes
   permanecem; vários trechos de uma URL não viram fontes independentes.
4. Lookup de metadata/frescor foi separado da seleção de evidência factual.
   Uma página recém-coletada sem resposta ainda é uma página recém-coletada;
   não deve disparar recrawl só porque o extrator falhou. Regressão local
   confirma isso. O rastro passou a incluir contadores de URLs conhecidas,
   URLs com relação e famílias editoriais por tentativa de seleção.
5. **Falha adversarial confirmada e corrigida:** um título “Acme foi fundada
   por Alice” podia vencer o corpo “Acme não foi fundada por Alice”. Títulos
   deixaram de ser candidatos factuais isolados. Continuam como pistas de
   identidade/contexto, não evidência positiva independente do texto retido.
6. Serper passou a implementar a interface `search_*_checked`: HTTP/transport
   e JSON malformado retornam falha, não uma busca vazia saudável. Falta de
   créditos identificada no corpo HTTP desativa o adapter nesta instância,
   impedindo repetir pedidos sem orçamento. Após recarga, recriar o cliente.
   Erros são classificados sem imprimir corpos arbitrários ou credenciais.
   Wrappers existentes podem agora registrar a falha corretamente.
7. A resposta informa degradação quando provedores estão indisponíveis ao
   término, sem tratar isso como ausência de fatos. É um snapshot de
   disponibilidade, **não** diagnóstico completo por chamada. O comparador
   passou a expor a reprovação de cobertura da linha de base.

Nenhum nome de pessoa, resposta ou domínio dos exemplos foi incorporado às
regras de produção. A seleção permanece heurística/estrutural: não é um
motor semântico geral, certificado de verdade ou prova de independência.

## Capturas preservadas

| Captura | Consultas | Listas não vazias | Respostas extrativas | p95 SDK | Interpretação |
|---|---:|---:|---:|---:|---|
| `transfer-origin-cycle2-v2.json` | 40 | 28 | 0 | 10,14s | Regressão de diagnóstico já inspecionada. Não demonstrou ganho e latência piorou nesta amostra. |
| `origin-transfer-cold-v3.json` | 40 novas | 18 | 0 | 6,35s | Afetada pela falta de créditos; tempo menor não é melhoria validada. |

V2 conserva SHA do binário `327b4159985dfe045e4e3440b13ffddd10c9bb4dc0ae0a7dcacd6ee6ade311f8`.
V3 conserva SHA do binário `c6f740e15d2f8921158b10d65c1ff180e95556dafdd67e0306ddea9f7f264f31`.
O tratamento novo de quota e o bloqueio de título isolado vieram **depois**
dessas capturas; não atribuir resultados antigos ao binário final.

V3: oito estratos, inglês/português, queries escolhidas pelo implementador,
sem juízes ou rótulos independentes. Manifest selado antes da captura, SHA
`1dad08cb74c55daebd3a4deb542b6b70a187c44c08e210ad9d1c2881f8e6864c`.
Checagem de overlap exato não prova novidade semântica. Este conjunto já foi
inspecionado e não pode ser reapresentado como confirmação independente.

`market-origin-transfer-v3.json` recuperou 34/40 listas e **reprovou** seu
gate de cobertura (exit 2). É Google orgânico via Serper, não Google AI
Overview. SDK e baseline têm endpoints/custos de trabalho distintos. Os
números não permitem concluir superioridade, ganho de verdade ou vitória
de latência. Falhas de API precisam permanecer na avaliação operacional.
O novo `paired-origin-transfer-v3-with-gate.json` explicita a comparação
incompleta; o arquivo anterior foi conservado como histórico.

Neste ciclo: mais 80 pedidos completos ao SDK e 40 capturas de baseline.
Com o ciclo anterior: 360 pedidos completos ao SDK, 120 capturas de baseline,
120 perguntas distintas de diagnóstico, além de smoke tests/probes separados.
Número de consultas ao SDK não é número de chamadas faturáveis ao provedor:
corroboração pode emitir vários pedidos. Não foi medida uma fatura real.

## Verificação final

- SDK: 330 testes passaram, seis ignorados, nenhuma falha na última suíte.
- Index: 176 unidades e quatro integrações passaram.
- Types: 90 testes passaram.
- Economics: cinco testes de cenários passaram; sem dinheiro real.
- Total: **605 testes**, mais três invariantes do comparador.
- CLI check, benchmark ONNX build e `git diff --check` passaram.
- Aviso preexistente de `truncate_url` não utilizado permanece.
- Tratamento novo de quota/título possui regressões locais; a validação
  ampla do binário final com serviço disponível **ainda não foi executada**.

## Para retomar

Restabelecer o Serper ou configurar outro provedor contratado autorizado.
Repetir, com novo índice vazio, o lote afetado e um conjunto novo, preservando
os resultados anteriores e congelando código/budget antes da captura.
Avaliar respostas completas com dois revisores externos; uma lista de links
ou abstenção não conta como resposta correta. A extração geral, tipagem de
entidade/qualificadores, cauda de aquisição de evidência, rotas transacionais,
P0 protegido, custo observado e cash condicionado continuam pendentes.

Não lançar token, mover dinheiro, recrutar/pagar terceiros ou ativar benefício
cash com base nessas regressões. O acesso à API permite testes, não validação
financeira nem autorização para compras adicionais.
