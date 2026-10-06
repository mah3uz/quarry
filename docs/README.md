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
| `.vitepress/theme/fonts/` | JetBrains Mono, bundled whole so box-drawing characters line up, and `NerdSymbols-subset.woff2`, just the Nerd Font icons the captures use (both OFL, licence alongside) |
| `.vitepress/og.html` | The share image's source; `just docs-og` renders it to `public/og.png` |
| `public/` | Served as-is: logo, favicon, share image, `robots.txt` |
| `wrangler.jsonc` | Cloudflare deployment |

To add a page, create a Markdown file, start it with a `title` and `description` in the front matter
and a `# Heading`, and add it to the sidebar in `.vitepress/config.mts`. `npm run build` fails on
links to pages that don't exist.

## Updating the terminal captures

The captures are real quarry output. `just docs-captures` remakes all three: it builds a small shop
database, drives quarry in tmux with a clean config (so your own settings stay out), saves the
screens, and rebuilds the icon subset so any new icons render. It needs tmux, `pyftsubset`
(fonttools) and JetBrainsMono Nerd Font Mono. The steps are in `scripts/captures.sh`; the icon
subset on its own is:

```sh
cd docs/.vitepress/theme
U=$(python3 -c "import glob; print(','.join(sorted({'U+%X' % ord(c) for f in glob.glob('captures/*.ans') for c in open(f).read() if 0xE000 <= ord(c) <= 0xF8FF or ord(c) >= 0xF0000})))")
pyftsubset /usr/share/fonts/TTF/JetBrainsMonoNerdFontMono-Regular.ttf --unicodes="$U" \
  --flavor=woff2 --layout-features='' --no-hinting --output-file=fonts/NerdSymbols-subset.woff2
```

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
