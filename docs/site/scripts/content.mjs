import { existsSync, globSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import remarkGfm from 'remark-gfm';
import remarkStringify from 'remark-stringify';
import { toString } from 'mdast-util-to-string';
import { visit } from 'unist-util-visit';

export const siteRoot = fileURLToPath(new URL('../', import.meta.url));
export const repositoryRoot = resolve(siteRoot, '../..');
export const manualRoot = resolve(repositoryRoot, 'docs/documentation');
export const base = '/docs/';
export const siteUrl = 'https://ls.bot';
export const repositoryUrl = 'https://github.com/smartcomputer-ai/lightspeed';

// These pages are published from their existing authoritative sources.
export const references = [
  { source: 'crates/api/contract/api-reference.md', slug: 'reference/api' },
  { source: 'crates/temporal-workflow/contract/workflow-contract.md', slug: 'reference/workflow-contract' },
];

const markdown = unified().use(remarkParse).use(remarkGfm).use(remarkStringify, {
  bullet: '-', fences: true, listItemIndent: 'one',
});
const urlPath = (path) => path.split(sep).map(encodeURIComponent).join('/');
export const pagePath = (slug) => `${base}${slug ? `${urlPath(slug)}/` : ''}`;
export const markdownPath = (slug) => `${base}${urlPath(slug || 'index')}.md`;

export function collectPages(root = repositoryRoot) {
  const pages = globSync('**/*.md', { cwd: resolve(root, 'docs/documentation') }).sort().map((file) => ({
    source: `docs/documentation/${file}`,
    slug: file.replace(/\.md$/, '').replace(/(?:^|\/)index$/, ''),
  }));
  return [...pages, ...references];
}

export function resolveLink(url, source, pages, root = repositoryRoot, assets) {
  if (/^(?:[a-z][a-z\d+.-]*:|\/|#)/i.test(url)) return url;
  const [, pathname, suffix] = url.match(/^([^?#]*)(.*)$/);
  if (!pathname) return url;
  const target = resolve(root, dirname(source), decodeURIComponent(pathname));
  const repoPath = relative(root, target);
  if (repoPath === '..' || repoPath.startsWith(`..${sep}`)) {
    throw new Error(`${source}: link escapes the repository: ${url}`);
  }
  if (!existsSync(target)) throw new Error(`${source}: missing link target: ${url}`);
  if (assets) {
    assets.add(repoPath);
    return `${base}assets/repository/${urlPath(repoPath)}${suffix}`;
  }
  const page = pages.find((entry) => resolve(root, entry.source) === target);
  if (page) return `${pagePath(page.slug)}${suffix}`;
  const kind = statSync(target).isDirectory() ? 'tree' : 'blob';
  return `${repositoryUrl}/${kind}/main/${urlPath(repoPath)}${suffix}`;
}

export function preparePage(body, page, pages, root = repositoryRoot, assets = new Set()) {
  const tree = markdown.parse(body);
  const heading = tree.children[0];
  if (heading?.type !== 'heading' || heading.depth !== 1) {
    throw new Error(`${page.source}: expected a leading Markdown # title`);
  }
  const title = toString(heading);
  const paragraph = tree.children.find((node) => node.type === 'paragraph');
  const summary = paragraph ? toString(paragraph).replace(/\s+/g, ' ').trim() : title;
  const description = summary.length > 200 ? `${summary.slice(0, 197).replace(/\s+\S*$/, '')}…` : summary;
  const imageReferences = new Set();
  visit(tree, 'imageReference', (node) => imageReferences.add(node.identifier));
  visit(tree, (node) => {
    if (node.type === 'link' || node.type === 'definition' || node.type === 'image') {
      const isImage = node.type === 'image' ||
        (node.type === 'definition' && imageReferences.has(node.identifier));
      node.url = resolveLink(node.url, page.source, pages, root, isImage ? assets : undefined);
    }
  });
  return { title, description, tree };
}

function starlightPage({ title, description, tree }, page) {
  const frontmatter = {
    title, description,
    editUrl: `${repositoryUrl}/edit/main/${urlPath(page.source)}`,
  };
  if (references.some((entry) => entry.source === page.source) && page.source.startsWith('crates/')) {
    // Generated references are edited through their Rust generators.
    frontmatter.editUrl = false;
  }
  return `---\n${Object.entries(frontmatter).map(([key, value]) => `${key}: ${JSON.stringify(value)}`).join('\n')}\n---\n\n${markdown.stringify({ ...tree, children: tree.children.slice(1) })}`;
}

export function transformPage(body, page, pages, root = repositoryRoot, assets = new Set()) {
  return starlightPage(preparePage(body, page, pages, root, assets), page);
}

export function markdownPage(prepared, pages) {
  const tree = structuredClone(prepared.tree);
  const routes = new Map(pages.map((page) => [pagePath(page.slug), markdownPath(page.slug)]));
  visit(tree, (node) => {
    if (!['link', 'definition', 'image'].includes(node.type)) return;
    // Repository-relative destinations have already been validated and mapped.
    // Absolute site URLs also need to work when this text is read on its own.
    if (!node.url.startsWith('/') && !node.url.startsWith(`${siteUrl}/`)) return;
    const url = new URL(node.url, siteUrl);
    if (url.origin !== siteUrl) return;
    const pathname = url.pathname.endsWith('/') ? url.pathname : `${url.pathname}/`;
    const destination = routes.get(pathname);
    if (destination) url.pathname = destination;
    node.url = url.href;
  });
  return markdown.stringify(tree);
}

export function llmsIndex(pages) {
  const text = (value) => ({ type: 'text', value });
  return markdown.stringify({ type: 'root', children: [
    { type: 'heading', depth: 1, children: [text('Lightspeed documentation')] },
    { type: 'blockquote', children: [{ type: 'paragraph', children: [
      text('Build, deploy, and operate durable agents with Lightspeed.'),
    ] }] },
    { type: 'paragraph', children: [text('Current development documentation. Each link opens the Markdown version of a published page.')] },
    { type: 'heading', depth: 2, children: [text('Documentation')] },
    { type: 'list', ordered: false, spread: false, children: [...pages]
      .sort((a, b) => a.slug.localeCompare(b.slug, 'en'))
      .map((page) => ({ type: 'listItem', spread: false, children: [
        { type: 'paragraph', children: [
          { type: 'link', url: `${siteUrl}${markdownPath(page.slug)}`, children: [text(page.title)] },
          text(`: ${page.description}`),
        ] },
      ] })),
    },
  ] });
}

function writeChanged(file, content) {
  const bytes = Buffer.from(content);
  if (existsSync(file) && readFileSync(file).equals(bytes)) return;
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, content);
}

export function stageDocumentation({ root = repositoryRoot, site = resolve(root, 'docs/site'), pages = collectPages(root) } = {}) {
  const output = resolve(site, '.generated/content');
  const publicOutput = resolve(site, '.generated/public');
  const expected = new Set();
  const assets = new Set();
  const publicFiles = new Set();
  const index = [];
  const writePublic = (file, content) => {
    if (publicFiles.has(file)) throw new Error(`Duplicate public documentation file: ${file}`);
    publicFiles.add(file);
    writeChanged(resolve(publicOutput, file), content);
  };
  for (const page of pages) {
    const filename = `${page.slug || 'index'}.md`;
    if (expected.has(filename)) throw new Error(`Duplicate documentation route: ${page.slug}`);
    expected.add(filename);
    const prepared = preparePage(readFileSync(resolve(root, page.source), 'utf8'), page, pages, root, assets);
    writeChanged(resolve(output, filename), starlightPage(prepared, page));
    writePublic(filename, markdownPage(prepared, pages));
    index.push({ ...page, title: prepared.title, description: prepared.description });
  }
  writePublic('llms.txt', llmsIndex(index));
  // Renaming or removing a source must also remove its published route.
  for (const file of globSync('**/*.md', { cwd: output })) {
    if (!expected.has(file)) rmSync(resolve(output, file));
  }
  for (const file of globSync('**/*', { cwd: resolve(site, 'public') })) {
    const source = resolve(site, 'public', file);
    if (!statSync(source).isFile()) continue;
    writePublic(file, readFileSync(source));
  }
  for (const asset of assets) {
    const destination = `assets/repository/${asset}`;
    writePublic(destination, readFileSync(resolve(root, asset)));
  }
  for (const file of globSync('**/*', { cwd: publicOutput })) {
    const target = resolve(publicOutput, file);
    if (statSync(target).isFile() && !publicFiles.has(file)) rmSync(target);
  }
  return pages;
}

export function watchDocumentation() {
  return {
    name: 'lightspeed-documentation-sources',
    configureServer(server) {
      const inputs = [manualRoot, resolve(repositoryRoot, 'docs/images'), resolve(siteRoot, 'public'),
        ...references.map(({ source }) => resolve(repositoryRoot, source))];
      server.watcher.add(inputs);
      const onChange = (file) => {
        if (!inputs.some((input) => file === input || file.startsWith(`${input}${sep}`))) return;
        try { stageDocumentation(); }
        catch (error) {
          server.config.logger.error(String(error));
          server.ws.send({ type: 'error', err: { message: String(error), stack: '' } });
        }
      };
      for (const event of ['add', 'change', 'unlink']) server.watcher.on(event, onChange);
      server.httpServer?.once('close', () => {
        for (const event of ['add', 'change', 'unlink']) server.watcher.off(event, onChange);
      });
    },
  };
}
