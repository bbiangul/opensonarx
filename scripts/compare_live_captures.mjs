// Paired availability/latency diagnostics. Neither overlap nor abstention is
// a truth/quality label. Run external blinded review separately.
import fs from 'node:fs';
import assert from 'node:assert/strict';

export function compare(sdk, market) {
  assert.equal(sdk.manifest_sha256, market.manifest_sha256, 'different sealed workloads');
  const baseline = new Map(market.cases.map(c => [c.manifest.id, c]));
  const rows = sdk.captures.map(c => {
    const b = baseline.get(c.manifest.id);
    assert.ok(b, `missing market query: ${c.manifest.id}`);
    assert.equal(c.manifest.query, b.manifest.query);
    const checklist = c.trace?.evidence_checklist;
    const urls = new Set(b.results.map(r => r.url));
    return {
      id: c.manifest.id, pass: c.pass, query: c.manifest.query,
      stratum: c.manifest.stratum, intent: c.classified_intent,
      sdk_results: c.results.length, market_results: b.results.length,
      sdk_status: c.answer?.status ?? 'error', sdk_summary: c.answer?.summary ?? null,
      sdk_ms: c.search_ms, market_retrieval_ms: b.search_ms,
      checklist_complete: checklist?.complete_answer_supported ?? null,
      missing_requirements: checklist?.requirements.filter(r => r.critical && r.status !== 'satisfied').map(r => r.id) ?? [],
      sdk_top_url: c.results[0]?.url ?? null, market_top_url: b.results[0]?.url ?? null,
      exact_url_overlap: c.results.filter(r => urls.has(r.url)).length,
      cache_hit: c.trace?.fresh_answer_cache_hit ?? false,
      hydration_requested: (c.trace?.initial_hydration_requested ?? 0) + (c.trace?.corroboration_hydration_requested ?? 0),
      hydrated_pages: c.hydrated_pages,
      factual_chunk_selections: c.trace?.factual_chunk_selections ?? [],
      unavailable_discovery_providers: c.trace?.unavailable_discovery_providers ?? [],
    };
  });
  const count = rows.length;
  const latency = subset => {
    const values = subset.map(r => r.sdk_ms).sort((a, b) => a - b);
    const percentile = p => values[Math.max(0, Math.ceil(p * values.length) - 1)] ?? null;
    return {count: values.length, p50_ms: percentile(.5), p95_ms: percentile(.95), p99_ms: percentile(.99)};
  };
  const passes = Object.fromEntries([...new Set(rows.map(r => r.pass))].map(pass => {
    const subset = rows.filter(r => r.pass === pass);
    return [pass, {requests: subset.length, sdk_nonempty: subset.filter(r => r.sdk_results > 0).length,
      cache_hits: subset.filter(r => r.cache_hit).length, latency: latency(subset)}];
  }));
  const strata = Object.fromEntries([...new Set(rows.map(r => r.stratum))].map(stratum => {
    const subset = rows.filter(r => r.stratum === stratum);
    return [stratum, { requests: subset.length,
      sdk_nonempty: subset.filter(r => r.sdk_results > 0).length,
      market_nonempty: subset.filter(r => r.market_results > 0).length,
      extractive_answers: subset.filter(r => r.sdk_status === 'extractive').length,
      destinations: subset.filter(r => r.sdk_status === 'destination').length }];
  }));
  return {
    experiment: 'paired-live-diagnostic-not-quality-proof', manifest_sha256: sdk.manifest_sha256,
    evaluated_binary_sha256: sdk.evaluated_binary_sha256, requests: count, strata, passes,
    sdk_nonempty: rows.filter(r => r.sdk_results > 0).length,
    market_nonempty: rows.filter(r => r.market_results > 0).length,
    sdk_latency: sdk.search_latency, market_retrieval_latency: market.latency,
    answer_status_counts: sdk.answer_status_counts,
    market_coverage_gate_passed: market.complete_coverage_gate_passed ?? null,
    limitations: [
      'Implementer-authored, inspected diagnostic set; no independent truth labels or superiority test.',
      'Market is Google organic results via Serper, not Google AI Overview or an end-user product answer.',
      'SDK timing includes answer/evidence work; market timing is retrieval only. No like-for-like latency victory claim.',
      'Debug binary, concurrency 2, single-machine conditions. Not production scale.',
      ...(market.complete_coverage_gate_passed === false ? ['Market capture coverage gate failed; comparison is incomplete and cannot support a superiority conclusion.'] : []),
      ...(sdk.manifest_sha256 === 'bb85bbdc0416f7883aa5381b20335891acdb5f23509e38fd369ca59b88f7aec5'
        ? ['V1 polling-stratum locale/language metadata is reversed relative to query text. Sealed original is preserved; fixed in transfer V2.'] : []),
      'Results-only is not a full answer. An abstention can be safe or unnecessarily restrictive; no labels here distinguish them.',
      'Reporting-family count and retained citation excerpts are structural checks, not proof of independent evidence or truth.',
    ], rows,
  };
}

if (process.argv.includes('--self-test')) {
  const c = { manifest: { id: 'x', query: 'q', stratum: 's' }, results: [], pass: 1, search_ms: 1 };
  const s = { manifest_sha256: 'same', captures: [c] };
  const m = { manifest_sha256: 'same', cases: [c] };
  assert.equal(compare(s, m).sdk_nonempty, 0);
  assert.throws(() => compare(s, { ...m, manifest_sha256: 'other' }));
  assert.throws(() => compare(s, { ...m, cases: [] }));
  console.log('3 diagnostic invariants passed');
} else {
  const [sdkPath, marketPath, outputPath] = process.argv.slice(2);
  assert.ok(sdkPath && marketPath, 'usage: node scripts/compare_live_captures.mjs SDK.json MARKET.json [NEW_OUTPUT.json]');
  const report = compare(JSON.parse(fs.readFileSync(sdkPath, 'utf8')), JSON.parse(fs.readFileSync(marketPath, 'utf8')));
  if (outputPath) fs.writeFileSync(outputPath, JSON.stringify(report, null, 2) + '\n', { flag: 'wx' });
  console.log(JSON.stringify({ ...report, rows: undefined }, null, 2));
}
