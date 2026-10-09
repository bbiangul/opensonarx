import fs from 'node:fs';
import crypto from 'node:crypto';
const root = 'outputs/validation-20261009/';
const previousPath = root + 'workload-transfer-v2.json';
const bytes = fs.readFileSync(previousPath);
const previous = JSON.parse(bytes);
const hash = value => crypto.createHash('sha256').update(value).digest('hex');
const groups = {
  navigation: ['Signal Messenger official website', 'Jitsi site oficial', 'KeePassXC official website', 'VLC site oficial', 'Thunderbird official website'],
  transactions: ['download Thunderbird for Linux official', 'baixar VLC para Windows oficial', 'apply for a Singapore passport official', 'agendar atendimento para título de eleitor oficial', 'renew an Irish passport official application'],
  factual: ['who invented the cyclotron', 'quem desenvolveu a linguagem Erlang', 'who founded the Internet Archive', 'quem inventou o sistema Braille', 'who developed the first electric battery'],
  current: ['latest stable release of Electron', 'versão estável atual do SQLite', 'current president of South Africa official', 'quem é o primeiro ministro da Austrália atualmente', 'latest release of NixOS'],
  diligence: ['does an audit alone prove a trading platform is trustworthy', 'um projeto é confiável apenas porque seu código é aberto', 'is a publisher trustworthy if all citations repeat the same press release', 'como verificar se um investimento é legítimo antes de transferir dinheiro', 'does a trusted account prove the identity of a charity'],
  polls: ['what does nonresponse bias mean in election polling', 'o que significa efeito de desenho em uma pesquisa eleitoral', 'how can poll weighting affect reported percentages', 'diferença entre margem de erro e erro de previsão eleitoral', 'what should a poll disclose about its sampling method'],
  high_impact: ['what is the difference between absolute and relative risk in medical evidence', 'como conferir a versão de uma recomendação clínica', 'how can a taxpayer verify the publication date of an official ruling', 'como verificar se um precedente judicial ainda está em vigor', 'what should a medical source disclose about conflicts of interest'],
  exploratory: ['compare versioned backups and archive repositories', 'vantagens e limitações de documentação colaborativa aberta', 'how do academic repositories handle corrected papers', 'diferenças entre verificação de origem e verificação de conteúdo', 'compare primary sources and independent replication in scientific reporting'],
};
const queries = Object.entries(groups).flatMap(([stratum, texts]) => texts.map((query, i) => ({
  id: `origin-transfer-v3-${stratum}-${i}`, query, stratum,
  source: 'implementer_authored_transfer_diagnostic', source_id: `${stratum}-${i}`,
  language: i % 2 ? 'pt' : 'en', locale: i % 2 ? 'pt-BR' : 'en-US', country: i % 2 ? 'BR' : 'US',
})));
const manifest = {...previous, name: 'origin-transfer-v3', sealed_at: new Date().toISOString(), queries,
  selection: {...previous.selection,
    independent_selection: false,
    purpose: 'New broad diagnostic queries after origin fixes; not independent quality proof. Exact overlap rejection does not guarantee semantic novelty.',
    prior_holdouts_excluded: [...previous.selection.prior_holdouts_excluded, {path: previousPath, sha256: hash(bytes)}],
  }};
const output = JSON.stringify(manifest, null, 2) + '\n';
const path = root + 'workload-origin-transfer-v3.json';
fs.writeFileSync(path, output, {flag: 'wx'});
console.log(JSON.stringify({path, sha256: hash(output), queries: queries.length}));
