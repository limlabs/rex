import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);

// Load the native binding directly
const binding = require('../rex-napi.darwin-arm64.node');
const { createRex } = binding;

const FIXTURE_ROOT = resolve(__dirname, '../../../fixtures/basic');

describe('createRex', () => {
  let rex;

  before(async () => {
    rex = await createRex({ root: FIXTURE_ROOT });
  });

  after(async () => {
    if (rex) await rex.close();
  });

  it('creates an instance', () => {
    assert.ok(rex, 'createRex should return an instance');
  });

  it('isDev defaults to false', () => {
    assert.strictEqual(rex.isDev, false);
  });

  it('has a buildId', () => {
    assert.ok(rex.buildId, 'should have a buildId');
    assert.strictEqual(typeof rex.buildId, 'string');
  });

  it('has a staticDir', () => {
    assert.ok(rex.staticDir, 'should have a staticDir');
    assert.strictEqual(typeof rex.staticDir, 'string');
  });
});

describe('matchRoute', () => {
  let rex;

  before(async () => {
    rex = await createRex({ root: FIXTURE_ROOT });
  });

  after(async () => {
    if (rex) await rex.close();
  });

  it('matches a static route', () => {
    const match = rex.matchRoute('/');
    assert.ok(match, 'should match /');
    assert.strictEqual(match.pattern, '/');
    assert.strictEqual(match.moduleName, 'index');
    assert.deepStrictEqual(match.params, {});
  });

  it('matches /about', () => {
    const match = rex.matchRoute('/about');
    assert.ok(match, 'should match /about');
    assert.strictEqual(match.moduleName, 'about');
  });

  it('matches dynamic route with params', () => {
    const match = rex.matchRoute('/blog/hello-world');
    assert.ok(match, 'should match /blog/:slug');
    assert.strictEqual(match.pattern, '/blog/:slug');
    assert.strictEqual(match.params.slug, 'hello-world');
  });

  it('returns null for unmatched route', () => {
    const match = rex.matchRoute('/nonexistent/deep/path');
    assert.strictEqual(match, null);
  });
});

describe('getServerSideProps', () => {
  let rex;

  before(async () => {
    rex = await createRex({ root: FIXTURE_ROOT });
  });

  after(async () => {
    if (rex) await rex.close();
  });

  it('returns props from GSSP page', async () => {
    const result = await rex.getServerSideProps('/');
    assert.ok(result.props, 'should have props');
    assert.strictEqual(result.props.message, 'Hello from Rex!');
    assert.ok(result.props.timestamp, 'should have timestamp');
  });

  it('returns props with dynamic params', async () => {
    const result = await rex.getServerSideProps('/blog/test-post');
    assert.ok(result.props, 'should have props');
    assert.strictEqual(result.props.slug, 'test-post');
    assert.strictEqual(result.props.title, 'Blog Post: test-post');
  });

  it('returns props from getStaticProps page', async () => {
    const result = await rex.getServerSideProps('/about');
    assert.ok(result.props, 'should have props');
    assert.strictEqual(
      result.props.description,
      'Rex is a Next.js Pages Router reimplemented in Rust.'
    );
  });

  it('throws for non-existent route', async () => {
    await assert.rejects(
      () => rex.getServerSideProps('/nonexistent'),
      /No route matches/
    );
  });
});

describe('renderToString', () => {
  let rex;

  before(async () => {
    rex = await createRex({ root: FIXTURE_ROOT });
  });

  after(async () => {
    if (rex) await rex.close();
  });

  it('renders page HTML with given props', async () => {
    const html = await rex.renderToString('/', { message: 'Test message', timestamp: 0 });
    assert.ok(html.includes('Test message'), `should contain props: ${html}`);
  });

  it('renders dynamic page', async () => {
    const html = await rex.renderToString('/blog/my-slug', {
      slug: 'my-slug',
      title: 'My Title',
    });
    assert.ok(html.includes('My Title'), `should contain title: ${html}`);
    assert.ok(html.includes('my-slug'), `should contain slug: ${html}`);
  });
});

describe('renderPage', () => {
  let rex;

  before(async () => {
    rex = await createRex({ root: FIXTURE_ROOT });
  });

  after(async () => {
    if (rex) await rex.close();
  });

  it('returns full HTML document for /', async () => {
    const result = await rex.renderPage('/');
    assert.strictEqual(result.status, 200);
    assert.ok(result.html.includes('<!DOCTYPE html>'), 'should have doctype');
    assert.ok(result.html.includes('<div id="__rex">'), 'should have __rex div');
    assert.ok(result.html.includes('__REX_DATA__'), 'should have data script');
    assert.ok(result.html.includes('Hello from Rex!'), 'should have SSR content');
  });

  it('returns 404 for non-existent page', async () => {
    const result = await rex.renderPage('/nonexistent');
    assert.strictEqual(result.status, 404);
  });

  it('returns correct headers', async () => {
    const result = await rex.renderPage('/');
    const contentType = result.headers.find(h => h.key === 'content-type');
    assert.ok(contentType, 'should have content-type header');
    assert.ok(contentType.value.includes('text/html'), 'should be text/html');
  });
});

describe('getRequestHandler', () => {
  let rex;
  let handler;

  before(async () => {
    rex = await createRex({ root: FIXTURE_ROOT });
    handler = rex.getRequestHandler();
  });

  after(async () => {
    if (rex) await rex.close();
  });

  it('returns a function', () => {
    assert.strictEqual(typeof handler, 'function');
  });

  it('handles page request (200)', async () => {
    const req = new Request('http://localhost:3000/');
    const resp = await handler(req);
    assert.strictEqual(resp.status, 200);
    const html = await resp.text();
    assert.ok(html.includes('<!DOCTYPE html>'), 'should have doctype');
    assert.ok(html.includes('Hello from Rex!'), 'should have SSR content');
  });

  it('handles 404 for unknown path', async () => {
    const req = new Request('http://localhost:3000/definitely-not-a-page');
    const resp = await handler(req);
    assert.strictEqual(resp.status, 404);
  });

  it('handles /_rex/router.js', async () => {
    const req = new Request('http://localhost:3000/_rex/router.js');
    const resp = await handler(req);
    assert.strictEqual(resp.status, 200);
    const text = await resp.text();
    assert.ok(text.includes('__REX_ROUTER'), 'should contain router code');
  });

  it('handles data endpoint', async () => {
    const buildId = rex.buildId;
    // Client router fetches /_rex/data/{buildId}{pathname}.json — for /about that's /about.json
    const req = new Request(`http://localhost:3000/_rex/data/${buildId}/about.json`);
    const resp = await handler(req);
    assert.strictEqual(resp.status, 200);
    const json = JSON.parse(await resp.text());
    assert.ok(json.props, 'should have props');
    assert.strictEqual(
      json.props.description,
      'Rex is a Next.js Pages Router reimplemented in Rust.'
    );
  });

  it('handles data endpoint with stale buildId', async () => {
    const req = new Request('http://localhost:3000/_rex/data/wrong-build-id/index.json');
    const resp = await handler(req);
    assert.strictEqual(resp.status, 404);
  });

  it('handles API route', async () => {
    const req = new Request('http://localhost:3000/api/hello');
    const resp = await handler(req);
    assert.strictEqual(resp.status, 200);
    const json = JSON.parse(await resp.text());
    assert.strictEqual(json.message, 'Hello from Rex API!');
  });

  it('handles concurrent requests', async () => {
    const urls = [
      'http://localhost:3000/',
      'http://localhost:3000/about',
      'http://localhost:3000/blog/post-1',
      'http://localhost:3000/blog/post-2',
      'http://localhost:3000/api/hello',
      'http://localhost:3000/_rex/router.js',
    ];

    const results = await Promise.all(
      urls.map(url => handler(new Request(url)))
    );

    for (const resp of results) {
      assert.ok(resp.status === 200, `Expected 200 for concurrent request, got ${resp.status}`);
    }
  });
});

describe('close and cleanup', () => {
  it('can create and close without requests', async () => {
    const instance = await createRex({ root: FIXTURE_ROOT });
    assert.ok(instance.buildId);
    await instance.close();
  });

  it('rejects matchRoute after close', async () => {
    const instance = await createRex({ root: FIXTURE_ROOT });
    await instance.close();
    const match = instance.matchRoute('/');
    assert.strictEqual(match, null, 'matchRoute should return null after close');
  });
});
