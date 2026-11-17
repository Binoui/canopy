#!/usr/bin/env python3
"""Report the audit's text pairs, compositing hover over chrome in sRGB."""
import re
from pathlib import Path

css = Path('src/chrome/assets/chrome.css').read_text()

def rgb(value):
    if value.startswith('#'):
        return tuple(int(value[i:i+2], 16) / 255 for i in (1, 3, 5))
    parts = [float(n) for n in re.findall(r'[\d.]+', value)]
    return tuple(n / 255 for n in parts[:3]) + (parts[3],)

def luminance(color):
    return sum(w * (c / 12.92 if c <= .04045 else ((c + .055) / 1.055) ** 2.4)
               for w, c in zip((.2126, .7152, .0722), color))

def ratio(a, b):
    x, y = sorted((luminance(a), luminance(b)))
    return (y + .05) / (x + .05)

for theme, selector in [('light', ':root'), ('dark', ':root.dark')]:
    block = re.search(re.escape(selector) + r'\s*\{([^}]+)', css)[1]
    tokens = dict(re.findall(r'--([\w-]+):\s*([^;]+)', block))
    colors = {k: rgb(v) for k, v in tokens.items() if v.startswith(('#', 'rgba'))}
    hover = colors['hover']
    backgrounds = {k: colors[k] for k in ('bg', 'chrome', 'tile')}
    backgrounds['hover'] = tuple(hover[i] * hover[3] + colors['chrome'][i] * (1-hover[3]) for i in range(3))
    hint = re.search(r'kbd \{[^}]*color: var\(--([\w-]+)\)', css)[1]
    for label, name in [('secondary', 'muted'), ('hint text', hint)]:
        print(f'{theme} {label}: ' + ', '.join(f'{k} {ratio(colors[name], v):.2f}:1' for k, v in backgrounds.items()))
    print(f'{theme} white/--danger: {ratio((1,1,1), colors["danger"]):.2f}:1')
    print(f'{theme} --line/--chrome: {ratio(colors["line"], colors["chrome"]):.2f}:1')
