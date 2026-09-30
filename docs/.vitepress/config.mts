import { defineConfig } from 'vitepress'

const repo = 'https://github.com/mah3uz/quarry'
const site = 'https://quarry.asmechanics.com'
const description =
  'A fast SQL client for the terminal: a smart REPL and a full-screen TUI for PostgreSQL, MySQL / MariaDB and SQLite.'

export default defineConfig({
  title: 'quarry',
  description,
  lang: 'en-US',
  cleanUrls: true,
  sitemap: { hostname: site },
  srcExclude: ['README.md'],
  lastUpdated: true,
  head: [
    ['link', { rel: 'icon', href: '/favicon.svg', type: 'image/svg+xml' }],
    ['meta', { name: 'theme-color', content: '#151823' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:site_name', content: 'quarry' }],
    ['meta', { property: 'og:image', content: `${site}/og.png` }],
    ['meta', { property: 'og:image:width', content: '1200' }],
    ['meta', { property: 'og:image:height', content: '630' }],
    ['meta', { property: 'og:image:alt', content: 'quarry: a SQL client that lives in your terminal' }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
    ['meta', { name: 'twitter:image', content: `${site}/og.png` }],
  ],
  // Each page's own title, description and address for search engines and link previews.
  transformHead({ pageData }) {
    const path = pageData.relativePath.replace(/(^|\/)index\.md$/, '$1').replace(/\.md$/, '')
    const url = `${site}/${path}`
    const title = pageData.frontmatter.layout === false ? 'quarry: a SQL client for your terminal' : `${pageData.title} | quarry`
    const text = pageData.frontmatter.description || pageData.description || description
    return [
      ['link', { rel: 'canonical', href: url }],
      ['meta', { property: 'og:title', content: title }],
      ['meta', { property: 'og:description', content: text }],
      ['meta', { property: 'og:url', content: url }],
      ['meta', { name: 'twitter:title', content: title }],
      ['meta', { name: 'twitter:description', content: text }],
    ]
  },
  markdown: {
    container: {
      tipLabel: 'Tip',
      infoLabel: 'Note',
      warningLabel: 'Careful',
      dangerLabel: 'Danger',
      detailsLabel: 'Details',
    },
  },
  themeConfig: {
    logo: '/logo.svg',
    nav: [
      { text: 'Guide', link: '/start/introduction', activeMatch: '^/(start|guides|advanced)/' },
      { text: 'Reference', link: '/reference/cli', activeMatch: '^/reference/' },
      { text: 'Help', link: '/help/troubleshooting', activeMatch: '^/help/' },
      { text: 'Releases', link: `${repo}/releases` },
    ],
    sidebar: [
      {
        text: 'Start here',
        items: [
          { text: 'What is quarry?', link: '/start/introduction' },
          { text: 'Installation', link: '/start/installation' },
          { text: 'Quick start', link: '/start/quick-start' },
        ],
      },
      {
        text: 'Guides',
        items: [
          { text: 'Connecting to a database', link: '/guides/connecting' },
          { text: 'Using the REPL', link: '/guides/repl' },
          { text: 'Using the TUI', link: '/guides/tui' },
          { text: 'Browsing and editing tables', link: '/guides/editing-data' },
          { text: 'Scripts, exports and pipes', link: '/guides/scripting' },
          { text: 'Asking a model for SQL', link: '/guides/ai' },
          { text: 'Staying safe', link: '/guides/safety' },
        ],
      },
      {
        text: 'Advanced',
        items: [
          { text: 'Saved connections', link: '/advanced/saved-connections' },
          { text: 'Passwords and secrets', link: '/advanced/passwords' },
          { text: 'TLS and SSH tunnels', link: '/advanced/tls-ssh' },
          { text: 'Favourite queries', link: '/advanced/favorites' },
          { text: 'Themes', link: '/advanced/themes' },
          { text: 'Prompt, icons and completion', link: '/advanced/customising' },
          { text: 'Key bindings and vim mode', link: '/advanced/keybindings' },
        ],
      },
      {
        text: 'Reference',
        items: [
          { text: 'Command-line options', link: '/reference/cli' },
          { text: 'Special commands', link: '/reference/special-commands' },
          { text: 'Keyboard shortcuts', link: '/reference/keys' },
          { text: 'Configuration file', link: '/reference/config' },
          { text: 'Output formats', link: '/reference/output-formats' },
          { text: 'Environment variables', link: '/reference/environment' },
          { text: 'Files and directories', link: '/reference/files' },
        ],
      },
      {
        text: 'Help',
        items: [
          { text: 'Troubleshooting', link: '/help/troubleshooting' },
          { text: 'Coming from pgcli, mycli or litecli', link: '/help/migrating' },
          { text: 'Contributing', link: '/help/contributing' },
          { text: 'Releasing', link: '/help/releasing' },
        ],
      },
    ],
    outline: { level: [2, 3], label: 'On this page' },
    search: { provider: 'local' },
    socialLinks: [{ icon: 'github', link: repo }],
    editLink: { pattern: `${repo}/edit/main/docs/:path`, text: 'Edit this page on GitHub' },
    lastUpdated: { text: 'Updated' },
    docFooter: { prev: 'Previous', next: 'Next' },
    footer: {
      message: 'Released under the MIT licence.',
      copyright: 'quarry: a SQL client for the terminal',
    },
    notFound: {
      title: 'Nothing quarried here',
      quote: 'This page doesn’t exist, or it moved. Try the search, or start from the beginning.',
      linkText: 'Go to the home page',
    },
  },
})
