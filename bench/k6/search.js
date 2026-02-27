import http from 'k6/http';
import { check, sleep } from 'k6';
import { Trend } from 'k6/metrics';

const BASE_URL = __ENV.BASE_URL || 'http://localhost:3333';

// Custom metric: server-side query latency
const queryLatency = new Trend('query_latency_ms');

// Pool of realistic search queries across topics
const QUERIES = [
    'rust async programming tokio',
    'python pandas dataframe tutorial',
    'kubernetes pod deployment yaml',
    'react hooks state management',
    'postgresql index optimization explain',
    'blockchain consensus mechanism proof',
    'transformer attention mechanism bert',
    'distributed systems cap theorem',
    'linux memory management kernel',
    'compiler optimization llvm ir',
    'tcp websocket protocol comparison',
    'web security xss csrf prevention',
    'serverless lambda cold start',
    'ci cd pipeline github actions',
    'haskell monad transformer stack',
    'embedded firmware rtos interrupt',
    'vulkan shader compute pipeline',
    'nlp tokenization word embeddings',
    'game physics collision detection',
    'robotics path planning slam algorithm',
];

function randomQuery() {
    return QUERIES[Math.floor(Math.random() * QUERIES.length)];
}

export const options = {
    scenarios: {
        // Scenario 1: Simple search — ramp to 100 VUs
        simple_search: {
            executor: 'ramping-vus',
            startVUs: 0,
            stages: [
                { duration: '30s', target: 100 },
                { duration: '60s', target: 100 },
                { duration: '10s', target: 0 },
            ],
            exec: 'simpleSearch',
        },
        // Scenario 2: Filtered search — constant 20 VUs
        filtered_search: {
            executor: 'constant-vus',
            vus: 20,
            duration: '60s',
            startTime: '10s',
            exec: 'filteredSearch',
        },
        // Scenario 3: Heavy search — top_k=100
        heavy_search: {
            executor: 'constant-vus',
            vus: 10,
            duration: '60s',
            startTime: '10s',
            exec: 'heavySearch',
        },
    },
    thresholds: {
        'http_req_duration{scenario:simple_search}': ['p(95)<200'],
        'http_req_duration{scenario:filtered_search}': ['p(95)<300'],
        'http_req_duration{scenario:heavy_search}': ['p(95)<1000'],
        http_req_failed: ['rate<0.01'],
        query_latency_ms: ['p(95)<150'],
    },
};

export function simpleSearch() {
    const payload = JSON.stringify({
        query: randomQuery(),
        top_k: 10,
    });

    const res = http.post(`${BASE_URL}/search`, payload, {
        headers: { 'Content-Type': 'application/json' },
        tags: { scenario: 'simple_search' },
    });

    check(res, {
        'status 200': (r) => r.status === 200,
        'has results': (r) => JSON.parse(r.body).total > 0,
    });

    if (res.status === 200) {
        const body = JSON.parse(res.body);
        queryLatency.add(body.query_ms);
    }

    sleep(0.1);
}

export function filteredSearch() {
    const payload = JSON.stringify({
        query: randomQuery(),
        top_k: 10,
        freshness: '7d',
        types: ['html'],
    });

    const res = http.post(`${BASE_URL}/search`, payload, {
        headers: { 'Content-Type': 'application/json' },
        tags: { scenario: 'filtered_search' },
    });

    check(res, {
        'status 200': (r) => r.status === 200,
    });

    if (res.status === 200) {
        const body = JSON.parse(res.body);
        queryLatency.add(body.query_ms);
    }

    sleep(0.2);
}

export function heavySearch() {
    const payload = JSON.stringify({
        query: randomQuery(),
        top_k: 100,
    });

    const res = http.post(`${BASE_URL}/search`, payload, {
        headers: { 'Content-Type': 'application/json' },
        tags: { scenario: 'heavy_search' },
    });

    check(res, {
        'status 200': (r) => r.status === 200,
    });

    if (res.status === 200) {
        const body = JSON.parse(res.body);
        queryLatency.add(body.query_ms);
    }

    sleep(0.5);
}
