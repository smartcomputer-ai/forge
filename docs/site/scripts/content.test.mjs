import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import remarkGfm from 'remark-gfm';
import { visit } from 'unist-util-visit';
import { llmsIndex, markdownPage, markdownPath, preparePage, resolveLink, stageDocumentation, transformPage } from './content.mjs';

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

test('agent Markdown preserves source structure and points to Markdown pages and public images', (t) => {
  const { root, pages, page } = fixture(t);
  const body = '# A `practical` guide\n\nUse **durable** agents.\n\n' +
    '[Start][start]\n\n[start]: guide/start.md?view=all#run\n\n' +
    '[Home](/docs/) [Guide](https://ls.bot/docs/guide/start/) [Section](#table)\n\n' +
    '[App](/app/) [Source](../../crates/protocol/src/lib.rs)\n\n' +
    '![Agent](../images/agent.png)\n\n' +
    '## Table\n\n| Key | Value |\n| --- | --- |\n| status | ready |\n\n' +
    '```sh\nprintf "[untouched](missing.md)"\n```\n\n```mermaid\nflowchart LR\n  A --> B\n```\n';
  const prepared = preparePage(body, page, pages, root);
  const result = markdownPage(prepared, pages);
  const parse = (text) => unified().use(remarkParse).use(remarkGfm).parse(text);
  const source = parse(body);
  const tree = parse(result);
  assert.deepEqual(tree.children[0].children.map(({ type, value }) => ({ type, value })),
    source.children[0].children.map(({ type, value }) => ({ type, value })));
  assert.equal(tree.children.filter((node) => node.type === 'heading' && node.depth === 1).length, 1);
  assert.ok(tree.children.some((node) => node.type === 'table'));
  assert.deepEqual(tree.children.filter((node) => node.type === 'code').map(({ lang, value }) => ({ lang, value })),
    source.children.filter((node) => node.type === 'code').map(({ lang, value }) => ({ lang, value })));
  assert.match(result, /\[start\]: https:\/\/ls\.bot\/docs\/guide\/start\.md\?view=all#run/);
  assert.match(result, /\[Home\]\(https:\/\/ls\.bot\/docs\/index\.md\)/);
  assert.match(result, /\[Guide\]\(https:\/\/ls\.bot\/docs\/guide\/start\.md\)/);
  assert.match(result, /\[Section\]\(#table\)/);
  assert.match(result, /\[App\]\(https:\/\/ls\.bot\/app\/\)/);
  assert.match(result, /https:\/\/github.com\/smartcomputer-ai\/lightspeed\/blob\/main\/crates\/protocol\/src\/lib.rs/);
  assert.match(result, /!\[Agent\]\(https:\/\/ls\.bot\/docs\/assets\/repository\/docs\/images\/agent.png\)/);
  assert.doesNotMatch(result, /^(?:---|editUrl:|description:)/m);
  // Rendering the Markdown representation must not mutate the HTML input.
  assert.equal(prepared.tree.children.find((node) => node.type === 'definition').url, '/docs/guide/start/?view=all#run');
  assert.equal(markdownPath(''), '/docs/index.md');
  assert.equal(markdownPath('guide/with spaces'), '/docs/guide/with%20spaces.md');
});

test('llms index lists exactly the published Markdown pages with escaped titles and descriptions', () => {
  const result = llmsIndex([
    { slug: 'guide/start', title: 'Use [agents]', description: 'Configure **settings**.' },
    { slug: '', title: 'Welcome', description: 'Start here.' },
  ]);
  const tree = unified().use(remarkParse).parse(result);
  const links = [];
  visit(tree, 'link', (node) => links.push({ url: node.url, title: node.children[0].value }));
  assert.deepEqual(links, [
    { url: 'https://ls.bot/docs/index.md', title: 'Welcome' },
    { url: 'https://ls.bot/docs/guide/start.md', title: 'Use [agents]' },
  ]);
  assert.equal(tree.children[0].type, 'heading');
  assert.equal(tree.children[1].type, 'blockquote');
  assert.ok(tree.children.some((node) => node.type === 'heading' && node.depth === 2));
});

test('staging keeps Markdown, its discovery index, and copied assets current after edits and removals', (t) => {
  const { root, pages } = fixture(t);
  for (const page of pages) writeFileSync(join(root, page.source), `# ${page.slug || 'Welcome'}\n\nA guide.\n`);
  const site = join(root, 'docs/site');
  mkdirSync(join(site, 'public'), { recursive: true });
  writeFileSync(join(site, 'public/favicon.svg'), '<svg/>');
  const stage = (selected = pages) => stageDocumentation({ root, pages: selected });
  const output = join(site, '.generated/public');
  stage();
  assert.match(readFileSync(join(output, 'index.md'), 'utf8'), /^# Welcome\n/);
  assert.ok(existsSync(join(output, 'guide/start.md')));
  assert.ok(existsSync(join(output, 'favicon.svg')));
  writeFileSync(join(root, pages[0].source), '# Updated welcome\n\nA new description.\n\n![Agent](../images/agent.png)\n');
  stage();
  assert.match(readFileSync(join(output, 'llms.txt'), 'utf8'), /Updated welcome.*A new description/);
  assert.ok(existsSync(join(output, 'assets/repository/docs/images/agent.png')));
  writeFileSync(join(root, pages[0].source), '# Updated welcome\n\nNo image.\n');
  rmSync(join(root, pages[1].source));
  stage([pages[0], pages[2]]);
  assert.equal(existsSync(join(output, 'guide/start.md')), false);
  assert.equal(existsSync(join(site, '.generated/content/guide/start.md')), false);
  assert.equal(existsSync(join(output, 'assets/repository/docs/images/agent.png')), false);
  assert.doesNotMatch(readFileSync(join(output, 'llms.txt'), 'utf8'), /guide\/start/);
  writeFileSync(join(site, 'public/index.md'), '# Conflicting page');
  assert.throws(() => stage([pages[0], pages[2]]), /Duplicate public documentation file: index\.md/);
});
