<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
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
// cut by hand but render the same on every build. The waves repeat every W and each band is drawn
// 2W wide, so sliding it left by W (the drift) loops without a seam.
//
// Each band is its own element, only as tall as its slice of rock, and drifts with a CSS transform:
// the GPU moves it without redrawing, which is what keeps the section smooth.
const W = 1440
const H = 620
const TOPS = [60, 140, 214, 290, 372, 450, 530]
const amp = (i: number) => 9 + (i % 3) * 4
function wave(y: number, seed: number, a: number): string {
  const pts: string[] = []
  const turn = (2 * Math.PI) / W
  for (let x = 0; x <= 2 * W; x += 48) {
    const dy = Math.sin(x * turn + seed) * a + Math.sin(x * turn * 3 + seed * 2.3) * a * 0.35
    pts.push(`${x},${(y + dy).toFixed(1)}`)
  }
  return pts.join(' L')
}
const layers = TOPS.map((y, i) => {
  const reach = (k: number) => amp(k) * 1.35 + 2
  const top = y - reach(i)
  // down to where the next band's wave can dip, which covers the rest
  const bottom = i + 1 < TOPS.length ? TOPS[i + 1] + reach(i + 1) : H
  return {
    cls: `s${i}`,
    d: `M0,${bottom} L0,${y} L${wave(y, i * 1.7 + 0.4, amp(i))} L${2 * W},${bottom} Z`,
    viewBox: `0 ${top.toFixed(1)} ${2 * W} ${(bottom - top).toFixed(1)}`,
    style: { top: `${(top / H) * 100}%`, height: `${((bottom - top) / H) * 100}%` },
  }
})
const vein = `M0,318 C220,286 380,356 620,310 S1020,254 1240,304 S1400,318 ${W},288`

// Crystals in the rock, placed from a fixed seed so every build draws the same face.
function seeded(seed: number) {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
const rand = seeded(7)
const flecks = Array.from({ length: 46 }, (_, i) => {
  const x = rand() * W
  const y = 96 + rand() * (H - 170)
  const s = 2.5 + rand() * 4.5
  return {
    key: i,
    d: `M${x.toFixed(1)},${(y - s).toFixed(1)} L${(x + s * 0.6).toFixed(1)},${y.toFixed(1)} L${x.toFixed(1)},${(y + s).toFixed(1)} L${(x - s * 0.6).toFixed(1)},${y.toFixed(1)} Z`,
    delay: `${(-rand() * 7).toFixed(2)}s`,
  }
})

// A headlamp on the rock face: it follows the pointer, or wanders slowly when the pointer is
// elsewhere, lighting the rock and making the crystals glint. The lamp is a disc moved by
// transform; the bright crystals inside it are shifted back the other way so they stay in place.
// Nothing is redrawn as it moves. Still with reduced motion, and stopped while off screen.
const face = ref<HTMLElement | null>(null)
const lampEl = ref<HTMLElement | null>(null)
const glintEl = ref<SVGSVGElement | null>(null)
const LAMP = 230
let stopLamp = () => {}
onMounted(() => {
  const el = face.value
  const lamp = lampEl.value
  const glints = glintEl.value
  if (!el || !lamp || !glints) return
  let rect = el.getBoundingClientRect()
  const pos = { x: rect.width * 0.1, y: rect.height * 0.5 }
  const place = () => {
    const x = pos.x - LAMP
    const y = pos.y - LAMP
    lamp.style.transform = `translate3d(${x.toFixed(1)}px, ${y.toFixed(1)}px, 0)`
    glints.style.transform = `translate3d(${(-x).toFixed(1)}px, ${(-y).toFixed(1)}px, 0)`
  }
  const size = () => {
    rect = el.getBoundingClientRect()
    glints.style.width = `${rect.width}px`
    glints.style.height = `${rect.height}px`
  }
  size()
  place()
  addEventListener('resize', size)
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
    stopLamp = () => removeEventListener('resize', size)
    return
  }
  let target: { x: number; y: number } | null = null
  let raf = 0
  const tick = (t: number) => {
    // idling, sweep the whole face: the terminal covers the middle, the rock shows at the sides
    const goal = target ?? { x: rect.width * (0.5 + 0.47 * Math.sin(t / 5200)), y: rect.height * (0.48 + 0.3 * Math.sin(t / 3700)) }
    pos.x += (goal.x - pos.x) * (target ? 0.14 : 0.02)
    pos.y += (goal.y - pos.y) * (target ? 0.14 : 0.02)
    place()
    raf = requestAnimationFrame(tick)
  }
  const move = (e: PointerEvent) => {
    rect = el.getBoundingClientRect()
    target = { x: e.clientX - rect.left, y: e.clientY - rect.top }
  }
  const leave = () => (target = null)
  // off screen, nothing runs: the lamp stops and the CSS animations pause
  const seen = new IntersectionObserver(([entry]) => {
    cancelAnimationFrame(raf)
    el.classList.toggle('asleep', !entry.isIntersecting)
    if (entry.isIntersecting) raf = requestAnimationFrame(tick)
  })
  el.addEventListener('pointermove', move)
  el.addEventListener('pointerleave', leave)
  seen.observe(el)
  stopLamp = () => {
    cancelAnimationFrame(raf)
    seen.disconnect()
    el.removeEventListener('pointermove', move)
    el.removeEventListener('pointerleave', leave)
    removeEventListener('resize', size)
  }
})
onBeforeUnmount(() => stopLamp())

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

      <section ref="face" class="face" aria-label="A quarry REPL session">
        <div class="rock" aria-hidden="true">
          <svg v-for="l in layers" :key="l.cls" :class="['band', l.cls]" :style="l.style" :viewBox="l.viewBox" preserveAspectRatio="none">
            <path :d="l.d" />
          </svg>
          <svg class="seam" :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none">
            <g class="flecks">
              <path v-for="f in flecks" :key="f.key" :d="f.d" :style="{ animationDelay: f.delay }" />
            </g>
            <path class="vein-glow" :d="vein" />
            <path class="vein" :d="vein" />
            <path class="pulse glow" :d="vein" pathLength="1" />
            <path class="pulse" :d="vein" pathLength="1" />
            <path class="pulse late" :d="vein" pathLength="1" />
          </svg>
          <div ref="lampEl" class="lamp">
            <svg ref="glintEl" class="glints" :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none">
              <path v-for="f in flecks" :key="f.key" :d="f.d" />
            </svg>
          </div>
        </div>
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
.rock {
  position: absolute;
  inset: 0;
  z-index: -1;
  overflow: hidden;
  mask-image: linear-gradient(to bottom, transparent, #000 16%, #000 80%, transparent);
}
.rock > svg {
  position: absolute;
  left: 0;
  display: block;
}
/* The rock creeps: each band drifts left at its own pace, the deeper ones slower. The bands are
   200% wide and move by half of that, one wave period. */
.band {
  width: 200%;
  will-change: transform;
  animation: drift var(--drift) linear infinite;
}
@keyframes drift {
  to { transform: translate3d(-50%, 0, 0); }
}
.s0 { --drift: 70s; fill: #eaeef5; }
.s1 { --drift: 84s; fill: #e2e7f0; }
.s2 { --drift: 98s; fill: #d8deea; }
.s3 { --drift: 112s; fill: #cdd4e3; }
.s4 { --drift: 126s; fill: #c2cadc; }
.s5 { --drift: 140s; fill: #b6bfd4; }
.s6 { --drift: 156s; fill: #aab4cb; }
.dark .s0 { fill: #191c29; }
.dark .s1 { fill: #1c2030; }
.dark .s2 { fill: #202436; }
.dark .s3 { fill: #23283b; }
.dark .s4 { fill: #272c41; }
.dark .s5 { fill: #2b3047; }
.dark .s6 { fill: #2f354d; }
.seam {
  top: 0;
  width: 100%;
  height: 100%;
}
.vein {
  fill: none;
  stroke: #16a8d8;
  stroke-width: 2;
  opacity: 0.6;
}
.dark .vein {
  opacity: 0.85;
}
/* Glows are wide faint strokes rather than blur filters, which are costly to draw. */
.vein-glow {
  fill: none;
  stroke: #4cc6ee;
  stroke-width: 9;
  opacity: 0;
}
.dark .vein-glow {
  opacity: 0.12;
}
/* Crystals twinkle faintly on their own and glint where the lamp shines. */
.flecks path {
  fill: #16a8d8;
  opacity: 0.12;
  animation: twinkle 7s ease-in-out infinite;
}
.dark .flecks path {
  fill: #9be3ff;
  opacity: 0.1;
}
@keyframes twinkle {
  50% { opacity: 0.32; }
}
/* Light running along the seam, into the terminal and out the other side. */
.pulse {
  fill: none;
  stroke: #0fa3d6;
  stroke-width: 3;
  stroke-linecap: round;
  stroke-dasharray: 0.05 0.95;
  stroke-dashoffset: 1;
  animation: seam 7.5s linear infinite;
}
.dark .pulse {
  stroke: #c9f3ff;
}
.pulse.glow {
  stroke: #4cc6ee;
  stroke-width: 10;
  opacity: 0.25;
}
.pulse.late {
  stroke-dasharray: 0.025 0.975;
  opacity: 0.6;
  animation-duration: 11s;
  animation-delay: -4s;
}
@keyframes seam {
  to { stroke-dashoffset: 0; }
}
/* The headlamp: a soft disc of light, moved by transform. */
.lamp {
  position: absolute;
  left: 0;
  top: 0;
  width: 460px;
  height: 460px;
  border-radius: 50%;
  overflow: hidden;
  will-change: transform;
  background: radial-gradient(closest-side, rgba(255, 255, 255, 0.55), rgba(255, 255, 255, 0.2) 55%, transparent);
  -webkit-mask-image: radial-gradient(closest-side, #000 35%, transparent);
  mask-image: radial-gradient(closest-side, #000 35%, transparent);
}
.dark .lamp {
  background: radial-gradient(closest-side, rgba(170, 200, 255, 0.16), rgba(170, 200, 255, 0.06) 55%, transparent);
}
.glints {
  position: absolute;
  left: 0;
  top: 0;
  will-change: transform;
}
.glints path {
  fill: #0f93c2;
}
.dark .glints path {
  fill: #eefcff;
  stroke: rgba(76, 198, 238, 0.55);
  stroke-width: 3;
  paint-order: stroke;
}
/* Off screen, the animations pause (the class is set by the lamp's observer). */
.face.asleep .rock * {
  animation-play-state: paused;
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
  .flecks path,
  .band {
    animation: none;
  }
  .pulse {
    display: none;
  }
}
</style>
