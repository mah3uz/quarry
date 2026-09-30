#!/usr/bin/env python3
# Writes logo.svg and banner.svg (and the docs site's copies). Edit the mascot here, not in the output.
#
# Every animation leaves the element's own attributes at the resting pose, so renderers without SMIL
# support (librsvg, image converters, PNG exports) draw the still artwork.

from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EASE = 'calcMode="spline" keySplines="0.45 0 0.55 1;0.45 0 0.55 1"'
BLINK = 'dur="5s" repeatCount="indefinite" keyTimes="0;0.9;0.93;0.96;1"'
TAP = 'dur="4s" repeatCount="indefinite"'
TAP_KEYS = 'keyTimes="0;0.62;0.74;0.8;0.9;1"'
SPARK_KEYS = 'keyTimes="0;0.79;0.81;0.92;1"'

MASCOT_DEFS = """
    <radialGradient id="gemglow" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#7dcfff" stop-opacity="0.75"/>
      <stop offset="1" stop-color="#7dcfff" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="stone" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="#3b4261"/>
      <stop offset="0.22" stop-color="#6b739c"/>
      <stop offset="0.5" stop-color="#8189b6"/>
      <stop offset="0.82" stop-color="#565f89"/>
      <stop offset="1" stop-color="#2f3549"/>
    </linearGradient>
    <linearGradient id="top" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#d5dbf7"/>
      <stop offset="1" stop-color="#9aa5ce"/>
    </linearGradient>
    <linearGradient id="wood" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="#f0c585"/>
      <stop offset="1" stop-color="#b3874a"/>
    </linearGradient>
    <linearGradient id="steel" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#e4e9ff"/>
      <stop offset="1" stop-color="#8a94c0"/>
    </linearGradient>
    <linearGradient id="gemL" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#e0fbff"/>
      <stop offset="1" stop-color="#7dcfff"/>
    </linearGradient>
    <linearGradient id="gemR" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#7dcfff"/>
      <stop offset="1" stop-color="#2a8fbf"/>
    </linearGradient>"""


def indent(text, spaces):
    pad = " " * spaces
    return "\n".join(pad + line if line.strip() else line for line in text.strip("\n").split("\n"))


def eye(cx):
    return f"""
<ellipse cx="{cx}" cy="222" rx="13" ry="16" fill="#1a1b26">
  <animate attributeName="ry" values="16;16;1.5;16;16" {BLINK}/>
</ellipse>"""


def eye_highlights(cx, small_cx):
    return f"""
<circle cx="{cx}" cy="215" r="5" fill="#ffffff">
  <animate attributeName="opacity" values="1;1;0;1;1" {BLINK}/>
</circle>
<circle cx="{small_cx}" cy="229" r="2.2" fill="#ffffff" opacity="0.8">
  <animate attributeName="opacity" values="0.8;0.8;0;0.8;0.8" {BLINK}/>
</circle>"""


def spark(dx, dy, color):
    return f"""
<circle cx="336" cy="128" r="3.2" fill="{color}" opacity="0">
  <animate attributeName="opacity" values="0;0;1;0;0" {TAP} {SPARK_KEYS}/>
  <animateTransform attributeName="transform" type="translate" values="0 0;0 0;0 0;{dx} {dy};{dx} {dy}" {TAP} {SPARK_KEYS}/>
</circle>"""


def mascot_body():
    """The mascot in a 512x512 frame, without its ground shadow."""
    return f"""
<g transform="translate(-14 8)">
  <!-- pickaxe, handle tucked behind the body; taps the rim, pivoting on the hand (local 0,155) -->
  <g transform="translate(420 118) rotate(16)">
    <rect x="-8" y="0" width="16" height="300" rx="8" fill="url(#wood)" stroke="#1a1b26" stroke-width="5"/>
    <path d="M-74 30 Q-38 -18 0 -18 Q38 -18 74 30 Q36 2 0 4 Q-36 2 -74 30 Z" fill="url(#steel)" stroke="#1a1b26" stroke-width="5" stroke-linejoin="round"/>
    <rect x="-14" y="-18" width="28" height="30" rx="6" fill="#565f89" stroke="#1a1b26" stroke-width="5"/>
    <animateTransform attributeName="transform" type="rotate" additive="sum" values="0 0 155;0 0 155;7 0 155;-4 0 155;0 0 155;0 0 155" {TAP} {TAP_KEYS}/>
  </g>

  <!-- stone cylinder -->
  <path d="M136 150 V400 A120 34 0 0 0 376 400 V150 Z" fill="url(#stone)"/>
  <path d="M136 233 A120 34 0 0 0 376 233" fill="none" stroke="#1f2335" stroke-width="7"/>
  <path d="M140 240 A118 33 0 0 0 372 240" fill="none" stroke="#a9b1d6" stroke-opacity="0.35" stroke-width="3"/>
  <path d="M136 316 A120 34 0 0 0 376 316" fill="none" stroke="#1f2335" stroke-width="7"/>
  <path d="M140 323 A118 33 0 0 0 372 323" fill="none" stroke="#a9b1d6" stroke-opacity="0.3" stroke-width="3"/>
  <path d="M136 150 V400 A120 34 0 0 0 376 400 V150" fill="none" stroke="#1a1b26" stroke-width="6" stroke-linejoin="round"/>

  <g fill="#1f2335" opacity="0.45">
    <circle cx="162" cy="206" r="2.5"/><circle cx="344" cy="198" r="2"/><circle cx="170" cy="300" r="2"/>
    <circle cx="232" cy="330" r="2.5"/><circle cx="300" cy="352" r="2"/><circle cx="348" cy="300" r="2.5"/>
    <circle cx="210" cy="410" r="2.5"/><circle cx="280" cy="420" r="2"/><circle cx="330" cy="392" r="2"/>
  </g>
  <g fill="#c0caf5" opacity="0.3">
    <circle cx="186" cy="196" r="2"/><circle cx="262" cy="292" r="2"/><circle cx="196" cy="352" r="2"/>
    <circle cx="252" cy="398" r="2"/><circle cx="318" cy="326" r="1.8"/>
  </g>

  <!-- chisel marks -->
  <path d="M136 272 l14 8 l-14 10 Z" fill="#1f2335" opacity="0.7"/>
  <path d="M376 356 l-12 7 l12 9 Z" fill="#1f2335" opacity="0.7"/>
  <path d="M178 372 l13 11 l-6 13 l12 9" fill="none" stroke="#2a2f45" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>
  <path d="M318 290 l-10 9 l8 10" fill="none" stroke="#2a2f45" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>

  <!-- top face -->
  <ellipse cx="256" cy="150" rx="120" ry="34" fill="url(#top)" stroke="#1a1b26" stroke-width="6"/>
  <path d="M150 140 L192 124 L226 142 L186 158 Z" fill="#ffffff" opacity="0.18"/>
  <path d="M300 170 L338 160 L360 166 L330 178 Z" fill="#565f89" opacity="0.35"/>
  <path d="M138 158 l16 -4 l-10 14 Z" fill="#2a2f45" stroke="#1a1b26" stroke-width="3" stroke-linejoin="round"/>

  <!-- face -->
{indent(eye(216), 2)}
{indent(eye(296), 2)}
{indent(eye_highlights(221, 212), 2)}
{indent(eye_highlights(301, 292), 2)}
  <ellipse cx="186" cy="248" rx="15" ry="8" fill="#f7768e" opacity="0.45"/>
  <ellipse cx="326" cy="248" rx="15" ry="8" fill="#f7768e" opacity="0.45"/>
  <path d="M240 244 Q256 260 272 244" fill="none" stroke="#1a1b26" stroke-width="6" stroke-linecap="round"/>

  <!-- data crystal -->
  <circle cx="256" cy="112" r="78" fill="url(#gemglow)">
    <animate attributeName="r" values="70;86;70" dur="3.2s" repeatCount="indefinite" {EASE}/>
    <animate attributeName="opacity" values="0.7;1;0.7" dur="3.2s" repeatCount="indefinite" {EASE}/>
  </circle>
  <path d="M222 156 L210 118 L224 96 L236 150 Z" fill="url(#gemR)" stroke="#1a1b26" stroke-width="4" stroke-linejoin="round"/>
  <path d="M290 156 L304 124 L292 104 L278 150 Z" fill="url(#gemR)" stroke="#1a1b26" stroke-width="4" stroke-linejoin="round"/>
  <path d="M256 58 L230 98 L238 160 L256 160 Z" fill="url(#gemL)"/>
  <path d="M256 58 L282 98 L274 160 L256 160 Z" fill="url(#gemR)"/>
  <path d="M256 58 L282 98 L274 160 L238 160 L230 98 Z" fill="none" stroke="#1a1b26" stroke-width="5" stroke-linejoin="round"/>
  <path d="M247 80 L239 100 L244 140" fill="none" stroke="#ffffff" stroke-width="4" stroke-linecap="round" opacity="0.85"/>
  <g transform="translate(256 60)">
    <path d="M0 -14 Q0 0 14 0 Q0 0 0 14 Q0 0 -14 0 Q0 0 0 -14 Z" fill="#ffffff" transform="scale(0)">
      <animateTransform attributeName="transform" type="scale" values="0;0;1;0;0" dur="3.2s" repeatCount="indefinite" keyTimes="0;0.35;0.45;0.55;1"/>
    </path>
  </g>

  <!-- little stone arm and hand gripping the handle -->
  <path d="M362 270 q10 -3 16 -2" fill="none" stroke="#1a1b26" stroke-width="20" stroke-linecap="round"/>
  <path d="M362 270 q10 -3 16 -2" fill="none" stroke="#6b739c" stroke-width="11" stroke-linecap="round"/>
  <circle cx="381" cy="268" r="15" fill="#737aa2" stroke="#1a1b26" stroke-width="5"/>
  <path d="M373 263 q8 -4 15 1" fill="none" stroke="#a9b1d6" stroke-opacity="0.6" stroke-width="3" stroke-linecap="round"/>

  <!-- pebbles -->
  <path d="M112 440 l8 -14 l16 2 l6 12 Z" fill="#565f89" stroke="#1a1b26" stroke-width="4" stroke-linejoin="round"/>
  <path d="M146 442 l5 -9 l10 2 l2 7 Z" fill="#737aa2" stroke="#1a1b26" stroke-width="3" stroke-linejoin="round"/>
  <path d="M384 442 l6 -10 l11 2 l3 8 Z" fill="#6b739c" stroke="#1a1b26" stroke-width="3" stroke-linejoin="round"/>

  <!-- sparks on each strike -->
{indent(spark(-16, -12, "#e0af68"), 2)}
{indent(spark(-4, -20, "#ffd88a"), 2)}
{indent(spark(10, -14, "#7dcfff"), 2)}
</g>"""


def shadow(breathing):
    anims = (
        f"""
    <animate attributeName="rx" values="150;136;150" dur="3.2s" repeatCount="indefinite" {EASE}/>
    <animate attributeName="opacity" values="0.55;0.38;0.55" dur="3.2s" repeatCount="indefinite" {EASE}/>
  """
        if breathing
        else ""
    )
    close = f">{anims}</ellipse>" if breathing else "/>"
    return f"""
<g transform="translate(-14 8)">
  <ellipse cx="256" cy="440" rx="150" ry="20" fill="#0b0c12" opacity="0.55"{close}
</g>"""


def twinkle(shape, cx, cy, begin, dur):
    return f"""
<g>
  {shape}
  <animate attributeName="opacity" values="1;0.25;1" dur="{dur}s" begin="{begin}s" repeatCount="indefinite"/>
  <animateTransform attributeName="transform" type="rotate" from="0 {cx} {cy}" to="360 {cx} {cy}" dur="14s" repeatCount="indefinite"/>
</g>"""


def sparkles():
    return "\n".join([
        twinkle('<path d="M118 108 Q118 124 134 124 Q118 124 118 140 Q118 124 102 124 Q118 124 118 108 Z" fill="#e0af68"/>', 118, 124, 0, 2.6),
        twinkle('<path d="M84 240 Q84 250 94 250 Q84 250 84 260 Q84 250 74 250 Q84 250 84 240 Z" fill="#e0af68"/>', 84, 250, 0.9, 2.2),
        twinkle('<path d="M446 300 Q446 309 455 309 Q446 309 446 318 Q446 309 437 309 Q446 309 446 300 Z" fill="#7dcfff"/>', 446, 309, 1.6, 2.8),
        """
<circle cx="150" cy="176" r="4" fill="#bb9af7">
  <animate attributeName="r" values="4;6;4" dur="2.4s" begin="0.4s" repeatCount="indefinite"/>
</circle>""",
    ])


def mascot(bob):
    """Shadow, body and sparkles in the 512x512 frame. `bob` makes the body float (too busy for an icon)."""
    body = mascot_body()
    if bob:
        body = f"""
<g>
  <animateTransform attributeName="transform" type="translate" values="0 0;0 -8;0 0" dur="3.2s" repeatCount="indefinite" {EASE}/>
{indent(body, 2)}
</g>"""
    return "\n\n".join([shadow(bob).strip("\n"), body.strip("\n"), sparkles().strip("\n")])


def logo():
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512" role="img" aria-labelledby="title">
  <title id="title">quarry</title>
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#2a2f45"/>
      <stop offset="1" stop-color="#16161e"/>
    </linearGradient>
    <radialGradient id="halo" cx="0.5" cy="0.42" r="0.5">
      <stop offset="0" stop-color="#7aa2f7" stop-opacity="0.35"/>
      <stop offset="1" stop-color="#7aa2f7" stop-opacity="0"/>
    </radialGradient>{MASCOT_DEFS}
  </defs>

  <rect width="512" height="512" rx="112" fill="url(#bg)"/>
  <rect width="512" height="512" rx="112" fill="url(#halo)"/>

{indent(mascot(bob=False), 2)}
</svg>
"""


def pill(x, label, color, i):
    width = round(len(label) * 11.2 + 44)
    return width, f"""
<g transform="translate({x} 262)">
  <rect width="{width}" height="40" rx="20" fill="{color}" fill-opacity="0.14" stroke="{color}" stroke-opacity="0.55" stroke-width="1.5"/>
  <circle cx="21" cy="20" r="5" fill="{color}">
    <animate attributeName="opacity" values="1;0.35;1" dur="2.5s" begin="{i * 0.5}s" repeatCount="indefinite"/>
  </circle>
  <text x="34" y="26.5" fill="#c0caf5" font-size="18" font-weight="600">{label}</text>
</g>"""


def pills():
    x, out = 452, []
    for i, (label, color) in enumerate([
        ("PostgreSQL", "#7aa2f7"),
        ("MySQL / MariaDB", "#e0af68"),
        ("SQLite", "#7dcfff"),
        ("REPL + TUI", "#bb9af7"),
        ("Rust", "#f7768e"),
    ]):
        width, markup = pill(x, label, color, i)
        out.append(markup)
        x += width + 12
    return "\n".join(out)


def typing():
    # Each glyph is forced to CW px (textLength), so the reveal and the cursor advance by whole characters.
    cw, x0, cmd, rest = 12, 476, "\\llm", "top 10 customers by revenue this month"
    n = len(cmd) + 1 + len(rest)
    dur, start, step, clear = 9.0, 0.6, 0.065, 8.4
    # Chrome ignores discrete keyTimes unless the list ends at 1.
    times = [0.0] + [start + k * step for k in range(n)] + [clear, dur]
    counts = [0] + list(range(1, n + 1)) + [0, 0]
    key_times = ";".join(f"{t / dur:.4f}" for t in times)
    # Once fully typed, leave slack so renderers that ignore textLength don't clip the last glyph.
    widths = ";".join(str(c * cw + (48 if c == n else 0)) for c in counts)
    cursor = ";".join(str(x0 + c * cw + 2) for c in counts)
    timing = f'keyTimes="{key_times}" dur="{dur}s" calcMode="discrete" repeatCount="indefinite"'
    return f"""
<clipPath id="typed">
  <rect x="{x0}" y="326" width="{n * cw + 48}" height="36">
    <animate attributeName="width" values="{widths}" {timing}/>
  </rect>
</clipPath>
<g font-family="'JetBrains Mono', 'SF Mono', Menlo, Consolas, 'DejaVu Sans Mono', monospace" font-size="20">
  <text x="452" y="352" fill="#9ece6a">❯</text>
  <text y="352" clip-path="url(#typed)"><tspan x="{x0}" fill="#bb9af7" textLength="{len(cmd) * cw}" lengthAdjust="spacing">{cmd}</tspan><tspan x="{x0 + (len(cmd) + 1) * cw}" fill="#c0caf5" textLength="{len(rest) * cw}" lengthAdjust="spacing">{rest}</tspan></text>
  <rect x="{x0 + n * cw + 2 + cw}" y="334" width="11" height="23" rx="2" fill="#7aa2f7">
    <animate attributeName="x" values="{cursor}" {timing}/>
    <animate attributeName="opacity" values="0.9;0" dur="1.1s" calcMode="discrete" repeatCount="indefinite"/>
  </rect>
</g>"""


def flowing_stop(offset, shift):
    colors = ["#7dcfff", "#7aa2f7", "#bb9af7"]
    colors = colors[shift:] + colors[:shift]
    values = ";".join(colors + colors[:1])
    return f"""
<stop offset="{offset}" stop-color="{colors[0]}">
  <animate attributeName="stop-color" values="{values}" dur="8s" repeatCount="indefinite"/>
</stop>"""


def banner():
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1280 400" width="1280" height="400" role="img" aria-labelledby="title desc">
  <title id="title">quarry</title>
  <desc id="desc">A fast, beautiful SQL client and TUI for PostgreSQL, MySQL/MariaDB and SQLite, written in Rust.</desc>
  <defs>{MASCOT_DEFS}
    <linearGradient id="bannerbg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#1f2335"/>
      <stop offset="1" stop-color="#13141c"/>
    </linearGradient>
    <radialGradient id="glowblue" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#7aa2f7" stop-opacity="0.28"/>
      <stop offset="1" stop-color="#7aa2f7" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="glowpurple" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#bb9af7" stop-opacity="0.16"/>
      <stop offset="1" stop-color="#bb9af7" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="wordmark" x1="0" y1="0" x2="1" y2="0">
{indent(flowing_stop("0", 0), 6)}
{indent(flowing_stop("0.5", 1), 6)}
{indent(flowing_stop("1", 2), 6)}
    </linearGradient>
    <pattern id="dots" width="28" height="28" patternUnits="userSpaceOnUse">
      <circle cx="2" cy="2" r="1.4" fill="#a9b1d6" fill-opacity="0.07"/>
    </pattern>
    <clipPath id="card"><rect width="1280" height="400" rx="28"/></clipPath>
  </defs>

  <g clip-path="url(#card)">
    <rect width="1280" height="400" fill="url(#bannerbg)"/>
    <rect width="1280" height="400" fill="url(#dots)"/>
    <ellipse cx="220" cy="210" rx="260" ry="230" fill="url(#glowblue)">
      <animate attributeName="opacity" values="0.75;1;0.75" dur="6s" repeatCount="indefinite" {EASE}/>
    </ellipse>
    <ellipse cx="1120" cy="60" rx="320" ry="220" fill="url(#glowpurple)">
      <animate attributeName="cx" values="1120;980;1120" dur="14s" repeatCount="indefinite" {EASE}/>
    </ellipse>
  </g>
  <rect x="0.75" y="0.75" width="1278.5" height="398.5" rx="27.5" fill="none" stroke="#3b4261" stroke-width="1.5"/>

  <g transform="translate(58 38) scale(0.64)">
{indent(mascot(bob=True), 4)}
  </g>

  <g font-family="Inter, 'Segoe UI', -apple-system, BlinkMacSystemFont, 'Helvetica Neue', Arial, sans-serif">
    <text x="446" y="168" fill="url(#wordmark)" font-size="124" font-weight="800" letter-spacing="-3">quarry</text>
    <text x="450" y="224" fill="#a9b1d6" font-size="30" font-weight="500">A fast, beautiful SQL client for the terminal</text>
{indent(pills(), 4)}
  </g>

{indent(typing(), 2)}
</svg>
"""


OUTPUTS = {
    "logo.svg": logo,
    "banner.svg": banner,
    # The docs site can't read files outside docs/, so it gets its own copies.
    "docs/public/logo.svg": logo,
    "docs/public/favicon.svg": logo,
    "docs/src/assets/logo.svg": logo,
}

if __name__ == "__main__":
    for name, render in OUTPUTS.items():
        (ROOT / name).write_text(render())
        print(f"wrote {name}")
