# quarry documentation

The site for [quarry](../README.md), built with [Astro Starlight](https://starlight.astro.build).

```sh
npm install
npm run dev       # live preview at http://localhost:4321
npm run build     # static site in dist/
npm run preview   # serve dist/ locally
```

Node 22.12 or newer is required (see `.node-version`).

## Where things are

| Path | Contents |
|---|---|
| `src/content/docs/` | The pages, in Markdown (`.md`) or MDX (`.mdx` when a page uses components) |
| `astro.config.mjs` | Site title, sidebar order and theme settings |
| `src/styles/theme.css` | Colours (Tokyo Night) and fonts |
| `src/components/Terminal.astro` | Renders a real terminal capture as HTML |
| `src/captures/` | Terminal captures (`tmux capture-pane -e -p` output) used on the home page and TUI guide |
| `src/assets/fonts/` | JetBrains Mono, bundled whole so box-drawing characters render everywhere (OFL licence alongside) |
| `public/` | Files served as-is: the favicon and logo |

To add a page, create a Markdown file under `src/content/docs/` and add its slug to the sidebar in
`astro.config.mjs`. The logo files (`public/logo.svg`, `public/favicon.svg`, `src/assets/logo.svg`)
are written by `scripts/gen_art.py` in the repository root; edit the mascot there.

## Deploying

The build output is a static site in `dist/`, so any static host works.

**Cloudflare Pages:** create a project from the repository with

- Root directory: `docs`
- Build command: `npm run build`
- Build output directory: `dist`

**Netlify:** create a site from the repository and set the base directory to `docs`. The build
command and publish directory come from `netlify.toml`.

Both read the Node version from `.node-version`.

Once the site has a URL, set `site` in `astro.config.mjs` so Astro can generate a sitemap and
canonical links.
