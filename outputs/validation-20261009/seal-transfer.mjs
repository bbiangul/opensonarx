import fs from 'node:fs';
import crypto from 'node:crypto';
const root = 'outputs/validation-20261009/';
const priorPath = root + 'workload-v1.json';
const priorBytes = fs.readFileSync(priorPath);
const prior = JSON.parse(priorBytes);
const groups = {
  navigation: ['Audacity official website', 'GIMP site oficial', 'Krita official website', 'Obsidian site oficial', 'OpenStreetMap official website'],
  transactions: ['download Audacity for Windows official', 'baixar GIMP para Linux oficial', 'apply for a New Zealand passport official', 'solicitar certidão de nascimento online oficial', 'renew a Canadian passport official application'],
  factual: ['who invented the World Wide Web', 'quem fundou a Wikimedia Foundation', 'who founded CERN', 'quem criou a linguagem Lua', 'who invented the first working laser'],
  current: ['latest stable release of Node.js', 'versão estável atual do Debian', 'current president of Argentina official', 'quem é o primeiro ministro do Reino Unido atualmente', 'latest release of WordPress'],
  diligence: ['is a token staking reward proof that a platform is trustworthy', 'essa plataforma de investimentos é confiável sem auditoria', 'is a charities reputation enough to trust its finances', 'como verificar se uma oferta de investimento é legítima', 'does a trusted publisher guarantee every claim is true'],
  polls: ['what does a polling confidence interval mean', 'o que significa ponderação em uma pesquisa eleitoral', 'how to identify the sample size of an election poll', 'como identificar a taxa de não resposta em uma pesquisa eleitoral', 'what is the difference between a poll and an exit poll'],
  high_impact: ['what should a medical guideline disclose about evidence certainty', 'como conferir se uma orientação de saúde está atualizada', 'how can a taxpayer identify their relevant official tax authority', 'como verificar a jurisdição de uma decisão judicial', 'what distinguishes a preprint from a peer reviewed medical study'],
  exploratory: ['compare relational and document databases for small teams', 'vantagens e limites de arquivos digitais comunitários', 'how do open source communities maintain documentation quality', 'diferenças entre repositórios acadêmicos e revistas científicas', 'compare citation indexing and full text academic search'],
};
const queries = Object.entries(groups).flatMap(([stratum, texts]) => texts.map((query, i) => ({
  id: `transfer-oct09-${stratum}-${i}`, query, stratum,
  source: 'implementer_authored_transfer_diagnostic', source_id: `${stratum}-${i}`,
  language: i % 2 ? 'pt' : 'en', locale: i % 2 ? 'pt-BR' : 'en-US', country: i % 2 ? 'BR' : 'US',
})));
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const manifest = { ...prior, name: 'oct09-transfer-diagnostic-v2', sealed_at: new Date().toISOString(), queries,
  selection: { ...prior.selection, independent_selection: false,
    prior_holdouts_excluded: [...prior.selection.prior_holdouts_excluded, {path: priorPath, sha256: hash(priorBytes)}],
    purpose: 'Fresh query transfer diagnostic after V1 fixes, not independent confirmatory quality evaluation.' } };
const output = JSON.stringify(manifest, null, 2) + '\n';
const path = root + 'workload-transfer-v2.json';
fs.writeFileSync(path, output, {flag: 'wx'});
console.log(JSON.stringify({path, sha256: hash(output), queries: queries.length}));
