import fs from 'node:fs/promises';
import crypto from 'node:crypto';
const groups={
 navigation:['mozilla firefox official website','LibreOffice site oficial','Blender official website','Zotero site oficial','Inkscape official website'],
 transactions:['download Firefox for Linux official','baixar LibreOffice para macOS oficial','renew passport UK official application','emitir comprovante de situação cadastral CPF','apply for an Australian passport official'],
 factual:['who developed the Julia programming language','quem criou o protocolo HTTP','what is the difference between a compiler and an interpreter','quem inventou o transistor','what does the term eventual consistency mean'],
 current:['latest stable release of Rust programming language','versão estável atual do PostgreSQL','current president of Mexico official','quem é o primeiro ministro do Canadá atualmente','latest release of LibreOffice'],
 diligence:['is this cryptocurrency project audited and does an audit prove safety','como verificar se uma corretora tem autorização da CVM','does a company registration prove an investment is safe','como verificar se um instituto de pesquisa divulgou sua metodologia','what evidence is needed to verify a charities legitimacy'],
 polls:['como identificar a margem de erro de uma pesquisa eleitoral','does a polling registration certify its results','diferença entre pesquisa estimulada e espontânea','how can two articles report the same election poll','o que significa período de coleta de uma pesquisa eleitoral'],
 high_impact:['what evidence supports checking interactions before taking two medicines','como identificar o órgão oficial de uma orientação tributária','what should a source disclose about a clinical trial','como conferir a data de vigência de uma norma','what evidence distinguishes correlation from causation in a health study'],
 exploratory:['compare programming languages for scientific computing','vantagens e limitações de armazenamento distribuído','how do libraries preserve digital archives','diferenças entre mecanismos de busca acadêmicos','how do communities moderate collaboratively edited reference works'],
};
const priors=[];
for(const name of (await fs.readdir('bench/fixtures')).filter(n=>/^generalization-msmarco-holdout-v\d+\.json$/.test(n))){
 const path='bench/fixtures/'+name, bytes=await fs.readFile(path);
 priors.push({path,sha256:crypto.createHash('sha256').update(bytes).digest('hex')});
}
const queries=[];
for(const [stratum,texts] of Object.entries(groups)) for(const [i,query] of texts.entries()){
 const english=i%2===0;
 queries.push({id:`oct09-${stratum}-${i}`,query,source:'implementer_authored_diagnostic_workload',source_id:`${stratum}-${i}`,
  language:english?'en':'pt-BR',locale:english?'en-US':'pt-BR',country:english?'US':'BR',stratum});
}
const manifest={schema_version:1,name:'oct09-mixed-diagnostic-v1',sealed_at:new Date().toISOString(),
 selection:{independent_selection:false,labels_excluded_from_capture:true,selection_before_first_capture:true,
  purpose:'diagnostic workload, not independent quality confirmation; burn after inspection',prior_holdouts_excluded:priors},queries};
const body=JSON.stringify(manifest,null,2);
await fs.writeFile('outputs/validation-20261009/workload-v1.json',body,{flag:'wx'});
console.log(JSON.stringify({queries:queries.length,strata:Object.keys(groups).length,prior_manifests:priors.length,
 sha256:crypto.createHash('sha256').update(body).digest('hex')}));
