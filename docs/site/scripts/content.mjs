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
export const repositoryUrl = 'https://github.com/smartcomputer-ai/lightspeed';

// These pages are published from their existing authoritative sources.
export const references = [
  { source: 'docs/variables.md', slug: 'reference/environment-variables' },
  { source: 'crates/api/contract/api-reference.md', slug: 'reference/api' },
  { source: 'crates/temporal-workflow/contract/workflow-contract.md', slug: 'reference/workflow-contract' },
];

const markdown = unified().use(remarkParse).use(remarkGfm).use(remarkStringify, {
  bullet: '-', fences: true, listItemIndent: 'one',
});
const urlPath = (path) => path.split(sep).map(encodeURIComponent).join('/');

export function collectPages() {
  const pages = globSync('**/*.md', { cwd: manualRoot }).sort().map((file) => ({
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
  if (page) return `${base}${page.slug ? `${page.slug}/` : ''}${suffix}`;
  const kind = statSync(target).isDirectory() ? 'tree' : 'blob';
  return `${repositoryUrl}/${kind}/main/${urlPath(repoPath)}${suffix}`;
}

export function transformPage(body, page, pages, root = repositoryRoot, assets = new Set()) {
  const tree = markdown.parse(body);
  const heading = tree.children[0];
  if (heading?.type !== 'heading' || heading.depth !== 1) {
    throw new Error(`${page.source}: expected a leading Markdown # title`);
  }
  const title = toString(heading);
  tree.children.shift();
  const paragraph = tree.children.find((node) => node.type === 'paragraph');
  const description = paragraph ? toString(paragraph).replace(/\s+/g, ' ').slice(0, 200) : title;
  const imageReferences = new Set();
  visit(tree, 'imageReference', (node) => imageReferences.add(node.identifier));
  visit(tree, (node) => {
    if (node.type === 'link' || node.type === 'definition' || node.type === 'image') {
      const isImage = node.type === 'image' ||
        (node.type === 'definition' && imageReferences.has(node.identifier));
      node.url = resolveLink(node.url, page.source, pages, root, isImage ? assets : undefined);
    }
  });
  const frontmatter = {
    title, description,
    editUrl: `${repositoryUrl}/edit/main/${urlPath(page.source)}`,
  };
  if (references.some((entry) => entry.source === page.source) && page.source.startsWith('crates/')) {
    // Generated references are edited through their Rust generators.
    frontmatter.editUrl = false;
  }
  return `---\n${Object.entries(frontmatter).map(([key, value]) => `${key}: ${JSON.stringify(value)}`).join('\n')}\n---\n\n${markdown.stringify(tree)}`;
}

function writeChanged(file, content) {
  const bytes = Buffer.from(content);
  if (existsSync(file) && readFileSync(file).equals(bytes)) return;
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, content);
}

export function stageDocumentation() {
  const pages = collectPages();
  const output = resolve(siteRoot, '.generated/content');
  const expected = new Set();
  const assets = new Set();
  for (const page of pages) {
    const filename = `${page.slug || 'index'}.md`;
    if (expected.has(filename)) throw new Error(`Duplicate documentation route: ${page.slug}`);
    expected.add(filename);
    writeChanged(resolve(output, filename), transformPage(
      readFileSync(resolve(repositoryRoot, page.source), 'utf8'), page, pages, repositoryRoot, assets,
    ));
  }
  // Renaming or removing a source must also remove its published route.
  for (const file of globSync('**/*.md', { cwd: output })) {
    if (!expected.has(file)) rmSync(resolve(output, file));
  }
  const publicOutput = resolve(siteRoot, '.generated/public');
  const publicFiles = new Set();
  for (const file of globSync('**/*', { cwd: resolve(siteRoot, 'public') })) {
    const source = resolve(siteRoot, 'public', file);
    if (!statSync(source).isFile()) continue;
    publicFiles.add(file);
    writeChanged(resolve(publicOutput, file), readFileSync(source));
  }
  for (const asset of assets) {
    const destination = `assets/repository/${asset}`;
    publicFiles.add(destination);
    writeChanged(resolve(publicOutput, destination), readFileSync(resolve(repositoryRoot, asset)));
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
