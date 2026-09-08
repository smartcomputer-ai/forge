import { execFileSync } from 'node:child_process';
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import mermaid from 'astro-mermaid';
import { stageDocumentation, watchDocumentation, repositoryRoot } from './scripts/content.mjs';

stageDocumentation();
const gitSha = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repositoryRoot, encoding: 'utf8' }).trim();

export default defineConfig({
  site: 'https://ls.bot',
  base: '/docs',
  trailingSlash: 'always',
  output: 'static',
  publicDir: './.generated/public',
  integrations: [
    mermaid({ autoTheme: true, enableLog: false }),
    starlight({
      title: 'Lightspeed',
      description: 'Build, deploy, and operate durable agents with Lightspeed.',
      logo: {
        dark: './src/assets/ls-mark.svg',
        light: './src/assets/ls-mark-light.svg',
        alt: '',
      },
      favicon: '/assets/ls-logo-2026-v1-ls.svg',
      customCss: ['./src/styles/theme.css'],
      social: [{ icon: 'github', label: 'Lightspeed on GitHub', href: 'https://github.com/smartcomputer-ai/lightspeed' }],
      components: {
        Head: './src/components/Head.astro',
        SocialIcons: './src/components/SocialIcons.astro',
        PageTitle: './src/components/PageTitle.astro',
        MarkdownContent: './src/components/MarkdownContent.astro',
        Footer: './src/components/Footer.astro',
      },
      head: [
        { tag: 'meta', attrs: { name: 'lightspeed-docs-sha', content: gitSha } },
        { tag: 'meta', attrs: { property: 'og:image', content: 'https://ls.bot/docs/social.png' } },
        { tag: 'meta', attrs: { name: 'twitter:card', content: 'summary_large_image' } },
      ],
      sidebar: [
        { label: 'Welcome', slug: '' },
        { label: 'Getting started', items: [
          { slug: 'getting-started/concepts' },
          { slug: 'getting-started/quickstart' },
          { slug: 'getting-started/first-agent' },
        ] },
        { label: 'Using Lightspeed', items: [
          { slug: 'using-lightspeed/sessions-and-runs' },
          { slug: 'using-lightspeed/models-and-credentials' },
          { slug: 'using-lightspeed/profiles-and-instructions' },
          { slug: 'using-lightspeed/workspaces-and-skills' },
          { slug: 'using-lightspeed/tools-and-mcp' },
          { slug: 'using-lightspeed/bots-and-triggers' },
          { slug: 'using-lightspeed/subagents-and-federation' },
          { slug: 'using-lightspeed/chat-channels' },
        ] },
        { label: 'Environments', collapsed: true, items: [
          { slug: 'environments/overview' },
          { slug: 'environments/bring-your-own-compute' },
          { slug: 'environments/incus-vms' },
          { slug: 'environments/using-environments' },
          { slug: 'environments/processes-and-jobs' },
          { slug: 'environments/credentials' },
          { slug: 'environments/power-and-cleanup' },
          { slug: 'environments/networking-and-ingress' },
        ] },
        { label: 'Deployment', items: [
          { slug: 'deployment/overview' },
          { slug: 'deployment/self-hosting' },
        ] },
        { label: 'Reference', collapsed: true, items: [
          { label: 'JSON-RPC API', slug: 'reference/api' },
          { label: 'Environment variables', slug: 'reference/environment-variables' },
          { label: 'Workflow contract', slug: 'reference/workflow-contract' },
        ] },
      ],
    }),
  ],
  vite: { plugins: [watchDocumentation()] },
});
