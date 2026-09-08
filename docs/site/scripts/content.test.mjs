import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import { resolveLink, transformPage } from './content.mjs';

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'lightspeed-docs-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const file of ['docs/documentation/index.md', 'docs/documentation/guide/start.md',
    'docs/documentation/reference/environment-variables.md', 'crates/protocol/src/lib.rs', 'docs/images/agent.png']) {
    mkdirSync(dirname(join(root, file)), { recursive: true });
    writeFileSync(join(root, file), 'fixture');
  }
  const pages = [
    { source: 'docs/documentation/index.md', slug: '' },
    { source: 'docs/documentation/guide/start.md', slug: 'guide/start' },
    { source: 'docs/documentation/reference/environment-variables.md', slug: 'reference/environment-variables' },
  ];
  return { root, pages, page: pages[0] };
}

test('manual and authoritative reference links resolve to published routes and preserve fragments', (t) => {
  const { root, pages, page } = fixture(t);
  assert.equal(resolveLink('guide/start.md#configure', page.source, pages, root), '/docs/guide/start/#configure');
  assert.equal(resolveLink('reference/environment-variables.md?view=all#runtime', page.source, pages, root), '/docs/reference/environment-variables/?view=all#runtime');
  assert.equal(resolveLink('../index.md', pages[1].source, pages, root), '/docs/');
});

test('repository source and directories link to GitHub without exposing them as site pages', (t) => {
  const { root, pages, page } = fixture(t);
  assert.equal(resolveLink('../../crates/protocol/src/lib.rs', page.source, pages, root),
    'https://github.com/smartcomputer-ai/lightspeed/blob/main/crates/protocol/src/lib.rs');
  assert.equal(resolveLink('../../crates/protocol/', page.source, pages, root),
    'https://github.com/smartcomputer-ai/lightspeed/tree/main/crates/protocol');
});

test('missing files and paths outside the repository fail instead of creating broken links', (t) => {
  const { root, pages, page } = fixture(t);
  assert.throws(() => resolveLink('missing.md', page.source, pages, root), Error);
  assert.throws(() => resolveLink('../../../outside.md', page.source, pages, root), Error);
});

test('external links, existing site URLs, and same-page fragments are preserved', (t) => {
  const { root, pages, page } = fixture(t);
  for (const url of ['https://ls.bot/app/', 'http://localhost:5173/app/', 'mailto:hello@example.com', '/docs/', '#run']) {
    assert.equal(resolveLink(url, page.source, pages, root), url);
  }
});

test('page metadata is derived from the title without rewriting code examples', (t) => {
  const { root, pages, page } = fixture(t);
  const code = 'cat <<\'EOF\'\n[example](missing.md)\n$HOME `literal` <tag>\nEOF';
  const result = transformPage(`# A practical guide\n\nUse **durable** agents.\n\n## Run\n\n\`\`\`sh\n${code}\n\`\`\`\n`, page, pages, root);
  assert.match(result, /title: "A practical guide"/);
  assert.match(result, /description: "Use durable agents\."/);
  const body = result.replace(/^---\n[\s\S]*?\n---\n/, '');
  const tree = unified().use(remarkParse).parse(body);
  assert.equal(tree.children.find((node) => node.type === 'code').value, code);
  assert.equal(tree.children.filter((node) => node.type === 'heading' && node.depth === 1).length, 0);
  assert.throws(() => transformPage('No page title.', page, pages, root), Error);
});

test('Markdown reference links and local images are adapted without network asset dependencies', (t) => {
  const { root, pages, page } = fixture(t);
  const assets = new Set();
  const result = transformPage('# Guide\n\n[Start][start]\n\n![Agent][agent]\n\n[start]: guide/start.md#run\n[agent]: ../images/agent.png\n', page, pages, root, assets);
  assert.match(result, /\[start\]: \/docs\/guide\/start\/#run/);
  assert.match(result, /\[agent\]: \/docs\/assets\/repository\/docs\/images\/agent.png/);
  assert.deepEqual([...assets], ['docs/images/agent.png']);
});
