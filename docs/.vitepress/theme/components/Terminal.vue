<script setup lang="ts">
// Renders a real terminal capture (`tmux capture-pane -e -p`) as HTML. It runs during the static
// build, so pages ship plain HTML.
import { computed } from 'vue'

const props = defineProps<{ capture: string; title?: string; label: string }>()

const captures = import.meta.glob('../captures/*.ans', { query: '?raw', import: 'default', eager: true }) as Record<
  string,
  string
>

const BASIC = [
  '#15161e', '#f7768e', '#9ece6a', '#e0af68', '#7aa2f7', '#bb9af7', '#7dcfff', '#a9b1d6',
  '#414868', '#f7768e', '#9ece6a', '#e0af68', '#7aa2f7', '#bb9af7', '#7dcfff', '#c0caf5',
]

function xterm256(n: number): string {
  if (n < 16) return BASIC[n]
  if (n >= 232) {
    const v = 8 + (n - 232) * 10
    return `rgb(${v},${v},${v})`
  }
  const i = n - 16
  const c = (x: number) => (x === 0 ? 0 : 55 + x * 40)
  return `rgb(${c(Math.floor(i / 36))},${c(Math.floor(i / 6) % 6)},${c(i % 6)})`
}

const escapeHtml = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

// East Asian wide and emoji ranges take two terminal cells.
function cells(cp: number): number {
  return (cp >= 0x1100 && cp <= 0x115f) || (cp >= 0x2e80 && cp <= 0xa4cf) || (cp >= 0xac00 && cp <= 0xd7a3) ||
    (cp >= 0xf900 && cp <= 0xfaff) || (cp >= 0xfe30 && cp <= 0xfe4f) || (cp >= 0xff00 && cp <= 0xff60) ||
    (cp >= 0x1f300 && cp <= 0x1faff)
    ? 2
    : 1
}

// Characters outside ASCII and box drawing may come from a fallback font of another width; pinning
// each to its cell width keeps the terminal grid aligned whatever font draws it.
function cellText(text: string): string {
  let out = ''
  for (const ch of text) {
    const cp = ch.codePointAt(0)!
    if (cp < 0x80 || (cp >= 0x2500 && cp <= 0x259f)) out += escapeHtml(ch)
    else out += `<span class="cell w${cells(cp)}">${escapeHtml(ch)}</span>`
  }
  return out
}

function toHtml(input: string): string {
  let fg: string | null = null
  let bg: string | null = null
  let bold = false, dim = false, italic = false, underline = false, reverse = false
  let html = ''

  const emit = (text: string) => {
    if (!text) return
    const [f, b] = reverse ? [bg ?? 'var(--term-bg)', fg ?? 'var(--term-fg)'] : [fg, bg]
    const style = [
      f && `color:${f}`,
      b && `background:${b}`,
      bold && 'font-weight:700',
      dim && 'opacity:.7',
      italic && 'font-style:italic',
      underline && 'text-decoration:underline',
    ].filter(Boolean).join(';')
    html += style ? `<span style="${style}">${cellText(text)}</span>` : cellText(text)
  }

  const apply = (params: string) => {
    const codes = params === '' ? [0] : params.split(';').map(Number)
    for (let i = 0; i < codes.length; i++) {
      const c = codes[i]
      if (c === 0) { fg = bg = null; bold = dim = italic = underline = reverse = false }
      else if (c === 1) bold = true
      else if (c === 2) dim = true
      else if (c === 3) italic = true
      else if (c === 4) underline = true
      else if (c === 7) reverse = true
      else if (c === 22) bold = dim = false
      else if (c === 23) italic = false
      else if (c === 24) underline = false
      else if (c === 27) reverse = false
      else if (c === 39) fg = null
      else if (c === 49) bg = null
      else if (c >= 30 && c <= 37) fg = BASIC[c - 30]
      else if (c >= 90 && c <= 97) fg = BASIC[c - 90 + 8]
      else if (c >= 40 && c <= 47) bg = BASIC[c - 40]
      else if (c >= 100 && c <= 107) bg = BASIC[c - 100 + 8]
      else if (c === 38 || c === 48) {
        let color: string | null = null
        if (codes[i + 1] === 2) { color = `rgb(${codes[i + 2]},${codes[i + 3]},${codes[i + 4]})`; i += 4 }
        else if (codes[i + 1] === 5) { color = xterm256(codes[i + 2]); i += 2 }
        if (c === 38) fg = color
        else bg = color
      }
    }
  }

  const re = /\x1b\[([0-9;]*)([A-Za-z])/g
  let last = 0
  for (let m = re.exec(input); m; m = re.exec(input)) {
    emit(input.slice(last, m.index))
    if (m[2] === 'm') apply(m[1])
    last = re.lastIndex
  }
  emit(input.slice(last))
  return html.replace(/\s+$/, '')
}

const raw = computed(() => {
  const text = captures[`../captures/${props.capture}.ans`]
  if (text === undefined) throw new Error(`no terminal capture named "${props.capture}"`)
  return text
})
const body = computed(() => toHtml(raw.value))
// The widest line in terminal cells, so the font can scale to fit the whole capture.
const cols = computed(() =>
  Math.max(
    20,
    ...raw.value
      .replace(/\x1b\[[0-9;]*[A-Za-z]/g, '')
      .replace(/\s+$/gm, '')
      .split('\n')
      .map((line) => [...line].reduce((n, ch) => n + cells(ch.codePointAt(0)!), 0)),
  ),
)
</script>

<template>
  <figure class="terminal" :aria-label="label" role="img" :style="{ '--cols': cols }">
    <div class="bar" aria-hidden="true">
      <span class="dot" /><span class="dot" /><span class="dot" />
      <span v-if="title" class="title">{{ title }}</span>
    </div>
    <pre aria-hidden="true" v-html="body" />
  </figure>
</template>

<style scoped>
.terminal {
  --term-bg: #151823;
  --term-fg: #c0caf5;
  margin: 24px 0;
  container-type: inline-size;
  border-radius: 14px;
  overflow: hidden;
  background: var(--term-bg);
  border: 1px solid rgba(160, 172, 220, 0.14);
  box-shadow:
    0 1px 0 rgba(255, 255, 255, 0.04) inset,
    0 24px 60px -24px rgba(9, 11, 20, 0.55);
}
.bar {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 11px 14px;
  background: #11131c;
  border-bottom: 1px solid rgba(160, 172, 220, 0.1);
}
.dot {
  width: 11px;
  height: 11px;
  border-radius: 50%;
  background: #2c3144;
}
.title {
  margin-inline: auto;
  padding-inline-end: 54px;
  font-family: var(--vp-font-family-mono);
  font-size: 12px;
  color: #6b739c;
}
pre {
  margin: 0;
  padding: 16px 18px 18px;
  overflow-x: auto;
  color: var(--term-fg);
  background: var(--term-bg);
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  /* JetBrains Mono's cells are 0.6em wide: fit the widest line to the frame, up to 13px. */
  font-size: min(13px, calc((100cqi - 36px) / var(--cols) / 0.6));
  line-height: 1.22;
  white-space: pre;
  word-break: normal;
  font-variant-ligatures: none;
}
pre :deep(.cell) {
  display: inline-block;
  text-align: center;
  overflow: visible;
}
pre :deep(.w1) {
  width: 1ch;
}
pre :deep(.w2) {
  width: 2ch;
}
</style>
