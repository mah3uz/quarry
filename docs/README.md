# quarry documentation

The site for [quarry](../README.md) at https://quarry.asmechanics.com, built with
[VitePress](https://vitepress.dev).

```sh
npm install
npm run dev       # live preview at http://localhost:5173
npm run build     # static site in .vitepress/dist
npm run preview   # serve the build
```

From the repository root, `just docs`, `just docs-build` and `just docs-deploy` do the same.
Node 22.12 or newer is required (see `.node-version`).

## Where things are

| Path | Contents |
|---|---|
| `index.md` | The landing page. It has no VitePress layout (`layout: false`) and renders `HomePage.vue`. |
| `start/`, `guides/`, `advanced/`, `reference/`, `help/` | The documentation pages, in Markdown |
| `.vitepress/config.mts` | Navigation, sidebar, search, SEO tags and the sitemap |
| `.vitepress/theme/style.css` | Colours, type and the styling of doc pages |
| `.vitepress/theme/components/HomePage.vue` | The landing page |
| `.vitepress/theme/components/Terminal.vue` | Renders a real terminal capture as HTML; use `<Terminal capture="tui" title="…" label="…" />` in any page |
| `.vitepress/theme/captures/` | The captures (`tmux capture-pane -e -p` output) |
| `.vitepress/theme/fonts/` | JetBrains Mono, bundled whole so box-drawing characters line up (OFL licence alongside) |
| `.vitepress/og.html` | The share image's source; `just docs-og` renders it to `public/og.png` |
| `public/` | Served as-is: logo, favicon, share image, `robots.txt` |
| `wrangler.jsonc` | Cloudflare deployment |

To add a page, create a Markdown file, start it with a `title` and `description` in the front matter
and a `# Heading`, and add it to the sidebar in `.vitepress/config.mts`. `npm run build` fails on
links to pages that don't exist.

The logo files in `public/` are written by `scripts/gen_art.py` in the repository root; edit the
mascot there.

## Deploying

The site is a Cloudflare Worker that serves the static build, on the custom domain
`quarry.asmechanics.com` (see `wrangler.jsonc`):

```sh
npx wrangler login     # once
just docs-deploy       # build and upload
```

`npx wrangler deploy --dry-run` checks the configuration without uploading anything.
