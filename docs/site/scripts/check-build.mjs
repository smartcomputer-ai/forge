import assert from 'node:assert/strict';
import { existsSync, globSync, readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { load } from 'cheerio';
import { base, collectPages, siteRoot } from './content.mjs';

const output = resolve(siteRoot, 'dist');
const origin = 'https://ls.bot';
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
}
assert.ok(existsSync(resolve(output, '404.html')), 'Missing static 404 page');
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
console.log(`Verified ${pages.length} documentation pages, ${actualDiagrams} diagrams, links, anchors, local assets, search, and sitemap.`);
