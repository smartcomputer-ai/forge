# Documentation site

Astro Starlight renders the product manual at `/docs/`. The site is a separate
npm workspace within Lightspeed. It builds static files and does not need the
Lightspeed runtime, credentials, a database, or the neighboring `ls-site` repo.

From the repository root:

```bash
npm install
npm run dev:docs       # local preview; open the printed URL under /docs/
npm run check:docs     # adapter tests, Astro diagnostics, build, and link checks
npm run build:docs     # static output in docs/site/dist/
npm run preview:docs   # serve the production build, including search
```

Astro's development server watches the original manual and the included
references. Editing a source updates the preview. Astro 7 runs the development
server in the background; `npm exec --workspace @lightspeed/docs -- astro dev stop`
stops it. Search is indexed during the production build, so use `preview:docs`
when checking search behavior.

## Authoring and navigation

Edit ordinary Markdown in `docs/documentation/`. Keep its leading `# Title`,
relative Markdown links, code fences, and Mermaid diagrams readable in GitHub.
`scripts/content.mjs` derives Starlight title/description metadata, removes the
duplicate body title, and adapts links into ignored `.generated/` files. Those
files are build inputs, not an additional source to maintain.

Add completed pages to the sidebar in `astro.config.mjs`. Keep implementation
roadmaps and unfinished placeholders outside the published manual. The home
page's four reading-path cards are in `src/components/MarkdownContent.astro`.

Three references are included directly from their authoritative sources:

| Published route | Source |
| --- | --- |
| `/docs/reference/environment-variables/` | `docs/variables.md` |
| `/docs/reference/api/` | `crates/api/contract/api-reference.md` |
| `/docs/reference/workflow-contract/` | `crates/temporal-workflow/contract/workflow-contract.md` |

Regenerate contracts with their existing Rust exporters. Do not edit generated
contracts or their staged copies. Other repository links point to GitHub;
relative images are copied into the output so the site can serve them locally.
Broken repository paths fail the build. The output check validates site links,
heading anchors, local assets, canonical URLs, diagram transforms, and the
presence of the search index and sitemap.

## Appearance and assets

`src/styles/theme.css` adapts the landing page's Merriweather headings,
Merriweather Sans body text, League Gothic wordmark, and cyan/cobalt/lavender
palette. Dark mode uses graphite surfaces and cyan accents; light mode uses
white surfaces and cobalt links. Starlight supplies the light/dark/automatic
selector, responsive navigation, keyboard search, and code-copy controls.
Dark-mode body text uses the hero's light weight (300), with medium-weight
emphasis (500) and regular serif headings (400). Near-white body text, white
headings, and brighter secondary text preserve contrast on dark surfaces.

All three fonts are preloaded in the document head using the same asset URLs
as the stylesheet. `font-display: block` briefly waits for a font on a cold
load instead of flashing fallback typography before swapping to the brand
face. League Gothic is used for the wordmark and section labels; Merriweather
is used for page and section headings, and Merriweather Sans for body text.

The fonts were copied from `ls-site/public/fonts/` and live under
`src/assets/fonts/`. Their SIL Open Font Licenses from the Google Fonts
repository are shipped in `public/licenses/`. The transparent white mark was
copied from `ls-site/public/assets/ls-logo-2026-v1-ls.svg`; its black-stroke
inverse is used in light mode. The favicon is the same white mark served by
the landing page, copied into `public/assets/`. `public/social.png` is the existing
`docs/images/ls-screenshot-factory.png`. These are checked-in assets: no build
step reads `ls-site`, and no font service is contacted by readers.

The explicit `cookie` dependency matches Astro's prerenderer. It keeps static
builds resolving the correct API in this workspace, which also contains older
cookie versions used by Express and React Router.

## CI and hosting

`.github/workflows/docs.yml` checks the site independently of Rust builds and
product deployment. `check:docs` is intentionally separate from the root
product `check` command; the adapter unit tests also run with the workspace tests.

The output directory is the document root for `https://ls.bot/docs/`:
`dist/index.html` serves `/docs/`, not `/docs/docs/`. Configure Caddy to strip the
`/docs` prefix before looking up files, redirect `/docs` to `/docs/`, serve
directory indexes, and return `404.html` with a 404 status for unknown paths.
Assets and search URLs already include `/docs/`. Add the docs sitemap to the
origin's existing `robots.txt` when publishing.

The manual is labeled **Current development** and records its source Git SHA
in page metadata. Bundle publication, the deployment installer, and the public
Caddy route are the next deployment step. The build here does not deploy it.
