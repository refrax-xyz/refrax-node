#!/usr/bin/env python3
"""Render .github/assets/demo.svg: an animated terminal session built from real `refrax` output.

Edit SESSION below after changing the CLI output, then run:  python3 scripts/demo-svg.py
"""
import re
from html import escape
from pathlib import Path

C = {  # the CLI's 256-colour palette
    "t": "#e4e4ec", "v": "#af87ff", "m": "#00ff87", "c": "#5fd7ff",
    "r": "#ff5f5f", "a": "#ffd75f", "d": "#7c7c88", "b": "#ffffff",
}

# Each line is a list of (colour, text). "$" lines are commands.
SESSION = [
    [("v", "$ "), ("b", "refrax devnet up")],
    [("t", "  "), ("v", "◆ "), ("b", "refrax devnet "), ("d", "· 16 nodes · local registry")],
    [("d", "  registry   "), ("c", "http://127.0.0.1:7700"), ("d", "  mirrors the Solana program accounts")],
    [("t", "  "), ("m", "ready"), ("t", " in 12 ms")],
    [],
    [("v", "$ "), ("b", "refrax put launch-video.png")],
    [("t", "  "), ("v", "◆ "), ("b", "refrax put "), ("d", "· launch-video.png · 142.6 KiB")],
    [("d", "  encode     "), ("t", "16 facets × 29.0 KiB · any 5 rebuild it"), ("d", "                5 ms")],
    [("d", "  register   "), ("t", "object "), ("v", "BbBw…goJW"), ("t", " · root 54b6…6dc5 · slot 6"), ("d", "             2 ms")],
    [("d", "  scatter    "), ("t", "16/16 nodes accepted their facet "), ("m", "✓"), ("d", "                   24 ms")],
    [("d", "  certify    "), ("t", "16/16 signatures · quorum 11 · "), ("m", "certified ✓"), ("d", "             2 ms")],
    [],
    [("v", "$ "), ("b", "refrax devnet wipe 7")],
    [("d", "  deleted    "), ("t", "1 facet from "), ("c", "http://127.0.0.1:7707")],
    [("d", "  ── devnet log ──")],
    [("d", "  19        "), ("a", "node-07"), ("t", " missed a proof · "), ("v", "BbBw…goJW"), ("t", " facet 06 · vote opened by node-16")],
    [("d", "  24        "), ("r", "node-07 slashed 500 REFRAX"), ("t", " · "), ("v", "BbBw…goJW"), ("t", " · 11 votes · stake 2,000 REFRAX")],
    [],
    [("v", "$ "), ("b", "refrax get BbBw21CYd9ukRes16jP3sv98PCdJmv4kPeBKe8gPgoJW -o copy.png")],
    [("d", "  fetch      "), ("t", "facets "), ("v", "00 01 02 03 04"), ("t", " · 0 skipped"), ("d", "                      3 ms")],
    [("d", "  rebuild    "), ("t", "142.6 KiB · data facets, no decoding · roots match "), ("m", "✓")],
]

W, LH, PAD, TOP, FS = 740, 21, 26, 52, 13.5
TIME_COL = 66  # timings are right-aligned to this column, like the CLI does
H = TOP + LH * len(SESSION) + PAD + 8

out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-label="refrax: start a devnet, store a file, wipe a node, get slashed, fetch the file">']
out.append("""<style>
.f{font-family:'Geist Mono','SFMono-Regular',Menlo,Consolas,'Liberation Mono',monospace;font-size:%spx;white-space:pre}
.l{opacity:0;animation:in .28s ease-out forwards}
.k{animation:blink 1.05s steps(1) infinite}
@keyframes in{from{opacity:0;transform:translateY(3px)}to{opacity:1;transform:none}}
@keyframes blink{50%%{opacity:0}}
@media (prefers-reduced-motion:reduce){.l{opacity:1;animation:none}.k{animation:none}}
</style>""" % FS)
out.append(f'<rect width="{W}" height="{H}" rx="14" fill="#0a0a0d"/>')
out.append(f'<rect x=".5" y=".5" width="{W-1}" height="{H-1}" rx="13.5" fill="none" stroke="#24242d"/>')
out.append(f'<path d="M0 36h{W}" stroke="#1a1a21"/>')
for i, col in enumerate(["#ff5f57", "#febc2e", "#28c840"]):
    out.append(f'<circle cx="{22 + i * 20}" cy="18" r="6" fill="{col}" opacity=".85"/>')
out.append(f'<text x="{W/2}" y="22.5" text-anchor="middle" class="f" fill="#7c7c88" font-size="12">~/refrax-node</text>')


def align(line):
    """Right-align a trailing timing segment such as '      24 ms'."""
    if line and re.fullmatch(r"\s{2,}\d+ ms", line[-1][1]):
        head = sum(len(t) for _, t in line[:-1])
        t = line[-1][1].strip()
        return line[:-1] + [(line[-1][0], " " * max(2, TIME_COL - head - len(t)) + t)]
    return line


delay = 0.2
for i, line in enumerate(map(align, SESSION)):
    y = TOP + LH * i + 4
    is_cmd = bool(line) and line[0][1].startswith("$")
    delay += 0.55 if is_cmd else (0.12 if line else 0.05)
    spans = "".join(f'<tspan fill="{C[c]}"{" font-weight=\"600\"" if c == "b" else ""}>{escape(t)}</tspan>' for c, t in line)
    out.append(f'<text x="{PAD}" y="{y}" class="f l" style="animation-delay:{delay:.2f}s">{spans}</text>')
cursor_y = TOP + LH * len(SESSION) + 4
out.append(f'<text x="{PAD}" y="{cursor_y}" class="f l" style="animation-delay:{delay+0.4:.2f}s"><tspan fill="{C["v"]}">$ </tspan><tspan class="k" fill="{C["t"]}">▍</tspan></text>')
out.append("</svg>")

dest = Path(__file__).resolve().parent.parent / ".github/assets/demo.svg"
dest.write_text("\n".join(out) + "\n")
print(f"wrote {dest} ({W}×{H})")
