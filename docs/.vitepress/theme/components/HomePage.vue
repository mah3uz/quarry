<script setup lang="ts">
import { ref } from 'vue'
import { useData, withBase } from 'vitepress'
import Terminal from './Terminal.vue'

const { isDark } = useData()
const repo = 'https://github.com/mah3uz/quarry'

const installs = [
  { id: 'aur', label: 'Arch (AUR)', command: 'paru -S quarry-sql-bin' },
  { id: 'cargo', label: 'Cargo', command: 'cargo install --git https://github.com/mah3uz/quarry' },
  { id: 'binary', label: 'Prebuilt', command: '' },
]
const chosen = ref('aur')
const copied = ref(false)
async function copyInstall(command: string) {
  await navigator.clipboard.writeText(command)
  copied.value = true
  setTimeout(() => (copied.value = false), 1600)
}

// Layered rock behind the REPL: each boundary is a gentle, deterministic wave so the strata look
// cut by hand but render the same on every build.
const W = 1440
const H = 620
function wave(y: number, seed: number, amp: number): string {
  const pts: string[] = []
  for (let x = 0; x <= W; x += 48) {
    const dy = Math.sin(x / 210 + seed) * amp + Math.sin(x / 77 + seed * 2.3) * amp * 0.35
    pts.push(`${x},${(y + dy).toFixed(1)}`)
  }
  return pts.join(' L')
}
const layers = [60, 140, 214, 290, 372, 450, 530].map((y, i) => ({
  d: `M0,${H} L0,${y} L${wave(y, i * 1.7 + 0.4, 9 + (i % 3) * 4)} L${W},${H} Z`,
  cls: `s${i}`,
}))
const vein = `M0,318 C220,286 380,356 620,310 S1020,254 1240,304 S1400,318 ${W},288`

const features = [
  {
    title: 'Completion that reads your schema',
    text: 'Tables after FROM, the columns of the tables in your statement, whole JOIN … ON clauses from foreign keys, and fuzzy matching, so ui finds user_id.',
  },
  {
    title: 'Careful with real data',
    text: 'DROP, TRUNCATE and a DELETE without WHERE ask first. Read-only mode is enforced by quarry and by the server. Passwords stay out of history.',
  },
  {
    title: 'Every way in',
    text: 'URLs, psql and mysql flags, ~/.pgpass and ~/.my.cnf, TLS up to verify-full, SSH tunnels, and saved connections, tinted red for production if you like.',
  },
  {
    title: 'Ask for SQL in plain words',
    text: '\\llm top 10 customers by revenue writes SQL for your schema and waits for you to review it. Use an API key, Ollama, or the Claude Code or Codex you already have.',
  },
  {
    title: 'Output that fits where it goes',
    text: 'Tables that turn vertical when they are too wide, a pager for long results, and 17 formats from psql and Markdown to CSV, JSON and SQL INSERT.',
  },
  {
    title: 'Made yours',
    text: '27 colour themes shared by the REPL and the TUI, your own palettes, any base16 or base24 scheme, and tab completion for bash, zsh and fish.',
  },
]
</script>

<template>
  <div class="landing">
    <header class="top">
      <a class="brand" :href="withBase('/')">
        <img :src="withBase('/logo.svg')" width="34" height="34" alt="" />
        <span>quarry</span>
      </a>
      <nav aria-label="Main">
        <a :href="withBase('/start/introduction')">Docs</a>
        <a :href="withBase('/reference/cli')">Reference</a>
        <a :href="`${repo}/releases`">Releases</a>
        <a :href="repo" class="icon" aria-label="quarry on GitHub">
          <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path fill="currentColor" d="M12 .5a11.5 11.5 0 0 0-3.64 22.41c.58.1.79-.25.79-.56v-2c-3.2.7-3.87-1.37-3.87-1.37-.53-1.33-1.28-1.69-1.28-1.69-1.05-.72.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.77 2.7 1.26 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.24-1.28-5.24-5.68 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.77 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.41-2.69 5.39-5.25 5.67.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 12 .5Z"/></svg>
        </a>
        <button class="icon" type="button" :aria-label="isDark ? 'Switch to light theme' : 'Switch to dark theme'" @click="isDark = !isDark">
          <svg v-if="isDark" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><circle cx="12" cy="12" r="4.5" fill="none" stroke="currentColor" stroke-width="2"/><path stroke="currentColor" stroke-width="2" stroke-linecap="round" d="M12 2v2.5M12 19.5V22M2 12h2.5M19.5 12H22M4.9 4.9l1.8 1.8M17.3 17.3l1.8 1.8M4.9 19.1l1.8-1.8M17.3 6.7l1.8-1.8"/></svg>
          <svg v-else viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5Z"/></svg>
        </button>
      </nav>
    </header>

    <main>
      <section class="hero">
        <div class="copy">
          <h1>A SQL client that lives in your terminal.</h1>
          <p class="lede">
            quarry talks to PostgreSQL, MySQL / MariaDB and SQLite. Type SQL in a REPL that knows your
            schema, or explore in a full-screen TUI. One small binary, nothing else to install.
          </p>
          <div class="actions">
            <a class="button primary" :href="withBase('/start/quick-start')">Get started</a>
            <a class="button" :href="withBase('/start/introduction')">Read the docs</a>
          </div>
          <div class="installer">
            <div class="choices" role="tablist" aria-label="How to install">
              <button v-for="i in installs" :key="i.id" type="button" role="tab" :aria-selected="chosen === i.id"
                @click="chosen = i.id; copied = false">{{ i.label }}</button>
            </div>
            <template v-for="i in installs" :key="i.id">
              <div v-if="chosen === i.id" class="install" role="tabpanel">
                <template v-if="i.command">
                  <code>{{ i.command }}</code>
                  <button type="button" @click="copyInstall(i.command)">{{ copied ? 'Copied' : 'Copy' }}</button>
                </template>
                <p v-else class="download">
                  Linux x86_64: <a :href="`${repo}/releases/latest`">download the latest release</a>
                </p>
              </div>
            </template>
            <p class="alt">
              Other ways, and macOS, in the <a :href="withBase('/start/installation')">installation guide</a>.
            </p>
          </div>
        </div>
        <img class="mascot" :src="withBase('/logo.svg')" width="340" height="340"
          alt="The quarry mascot: a stone database cylinder with a glowing crystal and a pickaxe" />
      </section>

      <section class="face" aria-label="A quarry REPL session">
        <svg class="strata" :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none" aria-hidden="true">
          <path v-for="l in layers" :key="l.cls" :d="l.d" :class="l.cls" />
          <path class="vein" :d="vein" />
        </svg>
        <div class="set">
          <Terminal capture="repl" title="quarry shop.db"
            label="A quarry REPL session: a join query with a table of results, then column completion for an alias" />
        </div>
      </section>

      <section class="mode wide">
        <div class="text">
          <h2>Go full-screen when you want to look around</h2>
          <p>
            <code>quarry --tui</code>, or <code>\tui</code> from the REPL, opens the same connection in a
            workspace: an explorer that writes SELECT and INSERT scripts for you, tabbed editors, a grid
            for large results, and a table view where edits are staged and applied in one transaction.
          </p>
          <a :href="withBase('/guides/tui')">Tour the TUI</a>
        </div>
        <Terminal capture="tui" title="quarry --tui shop.db"
          label="The quarry TUI: an explorer listing tables, a query editor and a results grid" />
      </section>

      <section class="mode flip">
        <div class="text">
          <h2>And script it</h2>
          <p>
            The same binary runs SQL from <code>-e</code>, a file or a pipe, and prints CSV, JSON, TSV or
            Markdown. Exit codes tell your scripts when something failed.
          </p>
          <a :href="withBase('/guides/scripting')">Scripts, exports and pipes</a>
        </div>
        <Terminal capture="script" title="bash" label="quarry printing query results as CSV and TSV in a shell" />
      </section>

      <section class="features">
        <h2>What else is in the box</h2>
        <dl>
          <div v-for="f in features" :key="f.title">
            <dt>{{ f.title }}</dt>
            <dd>{{ f.text }}</dd>
          </div>
        </dl>
      </section>

      <section class="start">
        <h2>Start with a SQLite file</h2>
        <p>No server needed. quarry creates the file, and the quick start takes it from there.</p>
        <pre><code>quarry tour.db</code></pre>
        <a class="button primary" :href="withBase('/start/quick-start')">Take the quick start</a>
      </section>
    </main>

    <footer class="bottom">
      <div class="cols">
        <div class="about">
          <a class="brand" :href="withBase('/')">
            <img :src="withBase('/logo.svg')" width="28" height="28" alt="" />
            <span>quarry</span>
          </a>
          <p>A SQL client for the terminal. Released under the MIT licence.</p>
        </div>
        <nav aria-label="Documentation">
          <h3>Learn</h3>
          <a :href="withBase('/start/installation')">Installation</a>
          <a :href="withBase('/start/quick-start')">Quick start</a>
          <a :href="withBase('/guides/connecting')">Connecting</a>
          <a :href="withBase('/guides/tui')">The TUI</a>
        </nav>
        <nav aria-label="Reference">
          <h3>Look up</h3>
          <a :href="withBase('/reference/cli')">Command-line options</a>
          <a :href="withBase('/reference/special-commands')">Special commands</a>
          <a :href="withBase('/reference/keys')">Keyboard shortcuts</a>
          <a :href="withBase('/reference/config')">Configuration</a>
        </nav>
        <nav aria-label="Project">
          <h3>Project</h3>
          <a :href="repo">Source code</a>
          <a :href="`${repo}/releases`">Releases</a>
          <a :href="`${repo}/blob/main/CHANGELOG.md`">Changelog</a>
          <a :href="`${repo}/issues`">Report a problem</a>
        </nav>
      </div>
    </footer>
  </div>
</template>

<style scoped>
.landing {
  --gutter: clamp(20px, 5vw, 64px);
  --measure: 1180px;
  min-height: 100vh;
  color: var(--vp-c-text-1);
  background: var(--vp-c-bg);
}
section,
.top,
.cols {
  max-width: var(--measure);
  margin: 0 auto;
  padding-left: var(--gutter);
  padding-right: var(--gutter);
}
a {
  color: inherit;
}

/* Header */
.top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 24px;
  padding-top: 20px;
  padding-bottom: 20px;
}
.brand {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  text-decoration: none;
  font-size: 21px;
  font-weight: 850;
  font-variation-settings: 'CASL' 0.8;
  letter-spacing: -0.02em;
}
.top nav {
  display: flex;
  align-items: center;
  gap: 6px;
}
.top nav a,
.top nav button {
  padding: 8px 12px;
  border-radius: 8px;
  font-size: 15px;
  font-weight: 600;
  text-decoration: none;
  color: var(--vp-c-text-2);
  background: none;
  border: 0;
  cursor: pointer;
  transition: color 0.15s, background-color 0.15s;
}
.top nav a:hover,
.top nav button:hover {
  color: var(--vp-c-text-1);
  background: var(--vp-c-bg-soft);
}
.top nav .icon {
  display: inline-grid;
  place-items: center;
  padding: 8px;
}

/* Hero: the headline is the voice of the page, set in Recursive's casual cut. */
.hero {
  display: grid;
  grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
  align-items: center;
  gap: 48px;
  padding-top: clamp(32px, 6vw, 88px);
  padding-bottom: 48px;
}
h1 {
  margin: 0;
  max-width: 13ch;
  font-size: clamp(42px, 6.4vw, 80px);
  line-height: 1;
  font-weight: 850;
  font-variation-settings: 'CASL' 1, 'MONO' 0;
  letter-spacing: -0.035em;
}
.lede {
  margin: 26px 0 0;
  max-width: 33em;
  font-size: clamp(17px, 1.6vw, 20px);
  line-height: 1.6;
  color: var(--vp-c-text-2);
}
.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin-top: 32px;
}
.button {
  display: inline-flex;
  align-items: center;
  padding: 12px 22px;
  border-radius: 10px;
  font-size: 16px;
  font-weight: 650;
  text-decoration: none;
  color: var(--vp-c-text-1);
  border: 1px solid var(--vp-c-border);
  transition: border-color 0.15s, background-color 0.15s;
}
.button:hover {
  border-color: var(--vp-c-brand-1);
}
.button.primary {
  color: var(--vp-button-brand-text);
  background: var(--vp-button-brand-bg);
  border-color: transparent;
}
.button.primary:hover {
  background: var(--vp-button-brand-hover-bg);
}
.installer {
  margin-top: 28px;
  max-width: 100%;
}
.choices {
  display: flex;
  gap: 4px;
  margin-bottom: 8px;
}
.choices button {
  padding: 5px 12px;
  border-radius: 7px;
  font-size: 13px;
  font-weight: 650;
  color: var(--vp-c-text-3);
  background: none;
  border: 1px solid transparent;
  cursor: pointer;
}
.choices button:hover {
  color: var(--vp-c-text-1);
}
.choices button[aria-selected='true'] {
  color: var(--vp-c-text-1);
  background: var(--vp-c-bg-soft);
  border-color: var(--vp-c-divider);
}
.download {
  margin: 0;
  padding: 6px 10px 6px 0;
  font-size: 14px;
  color: var(--vp-c-text-2);
}
.download a {
  color: var(--vp-c-brand-1);
  text-underline-offset: 3px;
}
.install {
  display: flex;
  align-items: center;
  gap: 8px;
  width: fit-content;
  max-width: 100%;
  min-height: 46px;
  padding: 6px 6px 6px 16px;
  border-radius: 10px;
  background: var(--vp-code-block-bg);
  border: 1px solid var(--vp-c-divider);
}
.install code {
  overflow-x: auto;
  white-space: nowrap;
  font-family: var(--vp-font-family-mono);
  font-size: 14px;
  color: var(--vp-c-text-1);
}
.install code::before {
  content: '$ ';
  color: var(--vp-c-text-3);
}
.install button {
  flex: none;
  padding: 6px 12px;
  border-radius: 7px;
  font-size: 13px;
  font-weight: 650;
  color: var(--vp-c-text-2);
  background: var(--vp-c-bg-soft);
  border: 1px solid var(--vp-c-divider);
  cursor: pointer;
}
.install button:hover {
  color: var(--vp-c-text-1);
}
.alt {
  margin: 12px 0 0;
  font-size: 14px;
  color: var(--vp-c-text-3);
}
.alt a {
  color: var(--vp-c-brand-1);
  text-underline-offset: 3px;
}
.mascot {
  justify-self: center;
  width: min(100%, 340px);
  height: auto;
}

/* The quarry face: full-bleed strata with the REPL set into the rock. */
.face {
  position: relative;
  max-width: none;
  padding-top: 88px;
  padding-bottom: 104px;
  isolation: isolate;
}
.strata {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  z-index: -1;
  mask-image: linear-gradient(to bottom, transparent, #000 16%, #000 80%, transparent);
}
.s0 { fill: #eaeef5; }
.s1 { fill: #e2e7f0; }
.s2 { fill: #d8deea; }
.s3 { fill: #cdd4e3; }
.s4 { fill: #c2cadc; }
.s5 { fill: #b6bfd4; }
.s6 { fill: #aab4cb; }
.vein {
  fill: none;
  stroke: #16a8d8;
  stroke-width: 2;
  opacity: 0.6;
}
.dark .s0 { fill: #191c29; }
.dark .s1 { fill: #1c2030; }
.dark .s2 { fill: #202436; }
.dark .s3 { fill: #23283b; }
.dark .s4 { fill: #272c41; }
.dark .s5 { fill: #2b3047; }
.dark .s6 { fill: #2f354d; }
.dark .vein {
  opacity: 0.85;
  filter: drop-shadow(0 0 6px rgba(76, 198, 238, 0.6));
}
.set {
  --term-max: 17px;
  max-width: 1000px;
  margin: 0 auto;
}
.set :deep(.terminal) {
  margin: 0;
  box-shadow:
    0 0 0 1px rgba(76, 198, 238, 0.18),
    0 40px 90px -30px rgba(9, 11, 20, 0.7);
}

/* Text beside a capture, alternating sides. */
.mode {
  display: grid;
  grid-template-columns: minmax(0, 0.8fr) minmax(0, 1.45fr);
  gap: 56px;
  align-items: center;
  margin-top: 104px;
}
.mode {
  --term-max: 15px;
}
.mode.flip {
  grid-template-columns: minmax(0, 1.65fr) minmax(0, 0.75fr);
}
/* The TUI needs the whole width to be readable: heading and text above, the capture below. */
.mode.wide {
  --term-max: 16px;
  grid-template-columns: minmax(0, 1fr);
  gap: 32px;
}
.mode.wide .text {
  display: grid;
  grid-template-columns: minmax(0, 0.9fr) minmax(0, 1.1fr);
  column-gap: 56px;
  align-items: start;
}
.mode.wide .text h2 {
  grid-row: span 2;
}
.mode.flip .text {
  order: 2;
}
h2 {
  margin: 0 0 16px;
  font-size: clamp(27px, 3vw, 38px);
  line-height: 1.12;
  font-weight: 800;
  font-variation-settings: 'CASL' 0.7;
  letter-spacing: -0.022em;
}
.text p,
.start p {
  margin: 0 0 18px;
  font-size: 17px;
  line-height: 1.65;
  color: var(--vp-c-text-2);
}
.text a {
  font-weight: 650;
  color: var(--vp-c-brand-1);
  text-decoration: underline;
  text-underline-offset: 4px;
  text-decoration-thickness: 1px;
}
.text code {
  font-family: var(--vp-font-family-mono);
  font-size: 0.86em;
  padding: 2px 6px;
  border-radius: 5px;
  background: var(--vp-code-bg);
  color: var(--vp-code-color);
}

/* Features: a plain two-column list, not a wall of cards. */
.features {
  margin-top: 128px;
}
dl {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 40px 64px;
  margin: 36px 0 0;
}
dt {
  width: fit-content;
  padding-top: 16px;
  padding-right: 14px;
  font-size: 18px;
  font-weight: 750;
  font-variation-settings: 'CASL' 0.5;
  border-top: 2px solid var(--vp-c-brand-1);
}
dd {
  max-width: 34em;
  margin: 10px 0 0;
  font-size: 16px;
  line-height: 1.65;
  color: var(--vp-c-text-2);
}

.start {
  margin-top: 128px;
  margin-bottom: 120px;
  text-align: center;
}
.start p {
  max-width: 34em;
  margin-left: auto;
  margin-right: auto;
}
.start pre {
  display: inline-block;
  margin: 6px 0 28px;
  padding: 14px 28px;
  border-radius: 10px;
  background: var(--q-shale);
  border: 1px solid rgba(160, 172, 220, 0.18);
}
.start pre code {
  font-family: var(--vp-font-family-mono);
  font-size: 17px;
  color: #e3e8fa;
}
.start pre code::before {
  content: '$ ';
  color: #6b739c;
}
.start .button {
  display: flex;
  width: fit-content;
  margin: 0 auto;
}

/* Footer */
.bottom {
  padding: 56px 0 64px;
  background: var(--vp-c-bg-alt);
  border-top: 1px solid var(--vp-c-divider);
}
.cols {
  display: grid;
  grid-template-columns: 1.6fr repeat(3, 1fr);
  gap: 40px;
}
.about p {
  max-width: 24em;
  margin: 14px 0 0;
  font-size: 14px;
  line-height: 1.6;
  color: var(--vp-c-text-3);
}
.bottom nav {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.bottom h3 {
  margin: 0 0 4px;
  font-size: 14px;
  font-weight: 750;
  color: var(--vp-c-text-1);
}
.bottom nav a {
  font-size: 14px;
  text-decoration: none;
  color: var(--vp-c-text-2);
}
.bottom nav a:hover {
  color: var(--vp-c-brand-1);
}

@media (max-width: 900px) {
  .hero,
  .mode,
  .mode.flip {
    grid-template-columns: minmax(0, 1fr);
    gap: 28px;
  }
  .mode.flip .text {
    order: 0;
  }
  .mode.wide .text {
    grid-template-columns: minmax(0, 1fr);
  }
  .mascot {
    grid-row: 1;
    justify-self: start;
    width: 160px;
  }
  dl {
    grid-template-columns: minmax(0, 1fr);
  }
  .face {
    padding-top: 48px;
    padding-bottom: 64px;
  }
  .cols {
    grid-template-columns: 1fr 1fr;
  }
  .about {
    grid-column: 1 / -1;
  }
}
@media (max-width: 560px) {
  .top nav a:not(.icon) {
    display: none;
  }
  .top nav a:first-child {
    display: inline-block;
  }
}
@media (prefers-reduced-motion: reduce) {
  .button,
  .top nav a,
  .top nav button {
    transition: none;
  }
}
</style>
