import assert from 'node:assert/strict';
import { existsSync, globSync, readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { load } from 'cheerio';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import remarkGfm from 'remark-gfm';
import { toString } from 'mdast-util-to-string';
import { visit } from 'unist-util-visit';
import { base, collectPages, markdownPath, siteRoot, siteUrl } from './content.mjs';

const output = resolve(siteRoot, 'dist');
const origin = siteUrl;
const markdown = unified().use(remarkParse).use(remarkGfm);
const markdownDocuments = new Map();
const codeBlocks = (tree) => {
  const blocks = [];
  visit(tree, 'code', ({ lang, value }) => blocks.push({ lang, value }));
  return blocks;
};
const documents = new Map();
for (const file of globSync('**/*.html', { cwd: output })) {
  const filename = resolve(output, file);
  const $ = load(readFileSync(filename, 'utf8'));
  documents.set(filename, { $, ids: new Set($('[id]').map((_, el) => $(el).attr('id')).get()) });
}
const pages = collectPages();
assert.equal(documents.size, pages.length + 1, 'Build must contain every published page and one 404, without extra routes');
for (const page of pages) {
  const filename = resolve(output, page.slug, 'index.html');
  const document = documents.get(filename);
  assert.ok(document, `Missing published page: ${page.source}`);
  assert.equal(document.$('h1').length, 1, `${page.source}: expected one page title`);
  for (const theme of ['auto', 'light', 'dark']) {
    assert.ok(document.$(`starlight-theme-select option[value="${theme}"]`).length,
      `${page.source}: missing ${theme} theme option`);
  }
  assert.equal(document.$('link[rel="canonical"]').attr('href'),
    `${origin}${base}${page.slug ? `${page.slug}/` : ''}`, `${page.source}: incorrect canonical URL`);
  assert.equal(document.$('link[rel="alternate"][type="text/markdown"]').attr('href'),
    markdownPath(page.slug), `${page.source}: missing Markdown discovery link`);
  const markdownFile = resolve(output, `${page.slug || 'index'}.md`);
  assert.ok(existsSync(markdownFile), `${page.source}: missing Markdown export`);
  const tree = markdown.parse(readFileSync(markdownFile, 'utf8'));
  assert.equal(tree.children[0]?.type, 'heading', `${page.source}: Markdown must start with its title`);
  assert.equal(tree.children[0]?.depth, 1, `${page.source}: Markdown must start with H1`);
  assert.equal(toString(tree.children[0]), document.$('h1').text(), `${page.source}: Markdown title differs from HTML`);
  assert.deepEqual(codeBlocks(tree), codeBlocks(markdown.parse(readFileSync(resolve(siteRoot, '../..', page.source), 'utf8'))),
    `${page.source}: Markdown export changed code or diagram source`);
  markdownDocuments.set(markdownFile, { tree, document });
}
assert.deepEqual([...globSync('**/*.md', { cwd: output })].sort(), pages.map((page) => `${page.slug || 'index'}.md`).sort(),
  'Only published pages may have Markdown exports');
assert.ok(existsSync(resolve(output, '404.html')), 'Missing static 404 page');
assert.equal(documents.get(resolve(output, '404.html')).$('link[rel="alternate"][type="text/markdown"]').length, 0,
  'The 404 must not advertise a nonexistent Markdown page');
assert.ok(existsSync(resolve(output, 'pagefind/pagefind.js')), 'Missing Pagefind search index');
assert.ok(existsSync(resolve(output, 'sitemap-index.xml')), 'Missing sitemap');
for (const file of globSync('**/*', { cwd: resolve(siteRoot, 'public') })) {
  if (statSync(resolve(siteRoot, 'public', file)).isFile()) {
    assert.ok(existsSync(resolve(output, file)), `Missing public asset or license: ${file}`);
  }
}
const search = JSON.parse(readFileSync(resolve(output, 'pagefind/pagefind-entry.json'), 'utf8'));
assert.equal(search.languages.en.page_count, pages.length, 'Search must index every published page and exclude the 404');

const failures = [];
const indexFile = resolve(output, 'llms.txt');
assert.ok(existsSync(indexFile), 'Missing llms.txt discovery index');
const indexTree = markdown.parse(readFileSync(indexFile, 'utf8'));
const indexLinks = [];
visit(indexTree, 'link', (node) => indexLinks.push(node.url));
assert.deepEqual(indexLinks.sort(), pages.map((page) => `${origin}${markdownPath(page.slug)}`).sort(),
  'llms.txt must link to every published Markdown page exactly once');
for (const [filename, { tree, document }] of markdownDocuments) {
  const route = `${origin}${base}${filename.slice(output.length + 1)}`;
  visit(tree, (node) => {
    if (!['link', 'definition', 'image'].includes(node.type)) return;
    const url = new URL(node.url, route);
    if (url.origin !== origin || !url.pathname.startsWith(base)) return;
    const target = resolve(output, decodeURIComponent(url.pathname.slice(base.length)));
    if (!existsSync(target)) failures.push(`${route}: missing Markdown target ${node.url}`);
    const targetDocument = node.url.startsWith('#') ? document : markdownDocuments.get(target)?.document;
    if (url.hash && targetDocument && !targetDocument.ids.has(decodeURIComponent(url.hash.slice(1)))) {
      failures.push(`${route}: missing Markdown heading ${node.url}`);
    }
    if (node.type === 'image' && !node.url.startsWith(`${origin}/`)) {
      failures.push(`${route}: image must have an absolute public URL: ${node.url}`);
    }
  });
}
for (const [filename, { $ }] of documents) {
  const route = `${origin}${base}${filename.slice(output.length + 1).replace(/index\.html$/, '')}`;
  $('[href], [src]').each((_, element) => {
    const raw = $(element).attr('href') ?? $(element).attr('src');
    if (!raw || /^(?:data:|mailto:|tel:|javascript:)/.test(raw)) return;
    const url = new URL(raw, route);
    if (url.origin !== origin) return;
    if (!url.pathname.startsWith(base)) {
      if (element.tagName !== 'a') failures.push(`${route}: asset outside /docs/: ${raw}`);
      return; // Links to the marketing site and hosted product are intentional.
    }
    let target = resolve(output, decodeURIComponent(url.pathname.slice(base.length)));
    if (existsSync(target) && statSync(target).isDirectory()) target = resolve(target, 'index.html');
    if (!existsSync(target)) {
      failures.push(`${route}: missing target ${raw}`);
    } else if (url.hash && documents.has(target) && !documents.get(target).ids.has(decodeURIComponent(url.hash.slice(1)))) {
      failures.push(`${route}: missing heading ${raw}`);
    }
  });
}
assert.deepEqual(failures, [], 'Published links, anchors, and local assets must resolve');

const expectedDiagrams = pages.reduce((count, page) => count +
  (readFileSync(resolve(siteRoot, '../..', page.source), 'utf8').match(/^```mermaid\s*$/gm)?.length ?? 0), 0);
const actualDiagrams = [...documents.values()].reduce((count, { $ }) => count + $('.mermaid').length, 0);
assert.equal(actualDiagrams, expectedDiagrams, 'Every Mermaid block must be prepared for diagram rendering');
console.log(`Verified ${pages.length} HTML and Markdown pages, llms.txt, ${actualDiagrams} diagrams, links, anchors, local assets, search, and sitemap.`);
