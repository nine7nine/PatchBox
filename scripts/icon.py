#!/usr/bin/env python3
"""PatchBox's icon: patch cables plugged into a patch panel, in a rounded square (transparent
outside it). Writes an SVG:

    scripts/icon.py dist/share/patchbox.svg
    scripts/icon.py dist/share/patchbox-small.svg simple    (three plugs, for 16 and 24 px)
"""
import sys
COL = {
    'green':  ('#27b14b', '#7fe896', '#0f5e24'),
    'orange': ('#ff8a1c', '#ffc985', '#9c4704'),
    'blue':   ('#2b8cf2', '#9ccdff', '#0f4686'),
    'red':    ('#e5323d', '#ff9aa0', '#7d0f17'),
    'white':  ('#e4e7ea', '#ffffff', '#8d949b'),
    'black':  ('#2a2d33', '#6b727d', '#0a0b0d'),
    'lime':   ('#8fd13f', '#cdf59a', '#4b7a14'),
}
out = []
defs = []
def grad_h(name, c):
    base, light, dark = c
    defs.append(f'''<linearGradient id="{name}" x1="0" x2="1" y1="0" y2="0">
  <stop offset="0" stop-color="{dark}"/><stop offset="0.28" stop-color="{light}"/>
  <stop offset="0.55" stop-color="{base}"/><stop offset="1" stop-color="{dark}"/></linearGradient>''')
for k, c in COL.items():
    grad_h('g_'+k, c)

def plug(x, y, s, col, top_x):
    """A plug seated at (x, y) = the nut's centre, scale s, its cable coming from top_x at the top."""
    base, light, dark = COL[col]
    W = 44*s; H = 88*s
    body_y = y - 18*s - H          # body sits above the sleeve
    boot_top = body_y - 46*s
    cw = 25*s
    o = []
    # cable: dark edge, colour, highlight
    path = f'M {top_x:.1f} -30 C {top_x:.1f} {boot_top*0.45:.1f}, {x:.1f} {boot_top*0.55:.1f}, {x:.1f} {boot_top+4:.1f}'
    o.append(f'<path d="{path}" fill="none" stroke="#000" stroke-opacity="0.55" stroke-width="{cw+5*s:.1f}" stroke-linecap="round"/>')
    o.append(f'<path d="{path}" fill="none" stroke="{base}" stroke-width="{cw:.1f}" stroke-linecap="round"/>')
    o.append(f'<path d="{path}" fill="none" stroke="{dark}" stroke-opacity="0.55" stroke-width="{cw*0.35:.1f}" stroke-linecap="round" transform="translate({cw*0.30:.1f},0)"/>')
    o.append(f'<path d="{path}" fill="none" stroke="{light}" stroke-opacity="0.85" stroke-width="{cw*0.22:.1f}" stroke-linecap="round" transform="translate({-cw*0.22:.1f},0)"/>')
    # boot: tapered, ribbed
    bw_top, bw_bot = 26*s, W*0.86
    o.append(f'<path d="M {x-bw_top/2:.1f} {boot_top:.1f} L {x+bw_top/2:.1f} {boot_top:.1f} L {x+bw_bot/2:.1f} {body_y+2:.1f} L {x-bw_bot/2:.1f} {body_y+2:.1f} Z" fill="url(#g_{col})" stroke="#000" stroke-opacity="0.5" stroke-width="{1.5*s:.1f}"/>')
    for k in range(1, 6):
        t = k/6
        yy = boot_top + t*(body_y-boot_top)
        hw = (bw_top + t*(bw_bot-bw_top))/2
        o.append(f'<line x1="{x-hw:.1f}" y1="{yy:.1f}" x2="{x+hw:.1f}" y2="{yy:.1f}" stroke="{dark}" stroke-opacity="0.7" stroke-width="{1.6*s:.1f}"/>')
    # body: a cylinder, rounded top, a groove
    o.append(f'<rect x="{x-W/2:.1f}" y="{body_y:.1f}" width="{W:.1f}" height="{H:.1f}" rx="{8*s:.1f}" fill="url(#g_{col})" stroke="#000" stroke-opacity="0.55" stroke-width="{1.8*s:.1f}"/>')
    o.append(f'<rect x="{x-W/2+2*s:.1f}" y="{body_y+H*0.62:.1f}" width="{W-4*s:.1f}" height="{3.2*s:.1f}" fill="{dark}" fill-opacity="0.75"/>')
    o.append(f'<rect x="{x-W*0.30:.1f}" y="{body_y+6*s:.1f}" width="{W*0.12:.1f}" height="{H-14*s:.1f}" rx="{3*s:.1f}" fill="#fff" fill-opacity="0.22"/>')
    # sleeve: metal
    sw = 30*s
    o.append(f'<rect x="{x-sw/2:.1f}" y="{y-20*s:.1f}" width="{sw:.1f}" height="{20*s:.1f}" fill="url(#metal_h)" stroke="#000" stroke-opacity="0.5" stroke-width="{1.2*s:.1f}"/>')
    return o

def nut(x, y, s):
    return [
        f'<ellipse cx="{x:.1f}" cy="{y+3*s:.1f}" rx="{38*s:.1f}" ry="{14*s:.1f}" fill="#000" fill-opacity="0.5"/>',
        f'<ellipse cx="{x:.1f}" cy="{y:.1f}" rx="{36*s:.1f}" ry="{13*s:.1f}" fill="url(#metal_r)" stroke="#2c3036" stroke-width="{1.5*s:.1f}"/>',
        f'<ellipse cx="{x:.1f}" cy="{y-1*s:.1f}" rx="{22*s:.1f}" ry="{7.5*s:.1f}" fill="#3b4048"/>',
    ]

defs.append('''<linearGradient id="metal_h" x1="0" x2="1"><stop offset="0" stop-color="#5d636b"/><stop offset="0.35" stop-color="#f4f6f8"/><stop offset="0.6" stop-color="#b9bfc6"/><stop offset="1" stop-color="#4a4f56"/></linearGradient>
<radialGradient id="metal_r" cx="0.42" cy="0.35" r="0.75"><stop offset="0" stop-color="#ffffff"/><stop offset="0.45" stop-color="#c3c8ce"/><stop offset="1" stop-color="#5b6068"/></radialGradient>
<linearGradient id="bg" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="#2a2e35"/><stop offset="1" stop-color="#0d0e11"/></linearGradient>
<linearGradient id="panel" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="#34383f"/><stop offset="0.08" stop-color="#1d1f24"/><stop offset="1" stop-color="#101114"/></linearGradient>
<radialGradient id="vig" cx="0.5" cy="0.42" r="0.75"><stop offset="0.6" stop-color="#000" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity="0.55"/></radialGradient>
<linearGradient id="gloss" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.16"/><stop offset="0.45" stop-color="#fff" stop-opacity="0"/></linearGradient>
<clipPath id="box"><rect x="16" y="16" width="480" height="480" rx="108"/></clipPath>''')

body = []
body.append('<rect x="0" y="0" width="512" height="512" fill="url(#bg)"/>')
simple = len(sys.argv) > 2 and sys.argv[2] == 'simple'
if simple:
    back = []
    front = [(118, 452, 'green', 40), (256, 446, 'orange', 250), (394, 440, 'blue', 480)]
    FS, BS = 1.45, 0
else:
    back = [(84, 322, 'red', 20), (198, 318, 'white', 160), (314, 314, 'black', 340), (428, 310, 'lime', 540)]
    front = [(140, 446, 'green', 30), (256, 442, 'orange', 270), (372, 438, 'blue', 490)]
    FS, BS = 1.15, 0.84
# Cables further back, crossing, dimmed: the depth of a full patch bay.
if not simple:
    for (path, col, w) in [('M -30 150 C 140 60, 330 260, 540 120', 'red', 22), ('M -30 60 C 170 170, 360 20, 540 200', 'blue', 20), ('M -30 230 C 160 170, 380 200, 540 60', 'lime', 18)]:
        base, light, dark = COL[col]
        body.append(f'<path d="{path}" fill="none" stroke="#000" stroke-opacity="0.5" stroke-width="{w+5}" stroke-linecap="round"/>')
        body.append(f'<path d="{path}" fill="none" stroke="{base}" stroke-width="{w}" stroke-linecap="round"/>')
        body.append(f'<path d="{path}" fill="none" stroke="{light}" stroke-opacity="0.7" stroke-width="{w*0.22:.1f}" stroke-linecap="round" transform="translate(0,{-w*0.22:.1f})"/>')
    body.append('<rect x="0" y="0" width="512" height="512" fill="#0b0c0f" fill-opacity="0.52"/>')
# the panel: a dark plate in perspective, rows of sockets with little labels
body.append('<path d="M -10 300 L 522 280 L 522 530 L -10 530 Z" fill="url(#panel)"/>')
body.append('<path d="M -10 300 L 522 280" stroke="#5a606a" stroke-width="3" stroke-opacity="0.8"/>')
for x, y, c, t in back:
    body += nut(x, y, BS)
    body.append(f'<rect x="{x-16:.1f}" y="{y+18:.1f}" width="32" height="4" rx="2" fill="#c9cdd2" fill-opacity="0.55"/>')
for x, y, c, t in front:
    body += nut(x, y, FS)
    if not simple:
        body.append(f'<rect x="{x-22:.1f}" y="{y+24:.1f}" width="44" height="5" rx="2.5" fill="#c9cdd2" fill-opacity="0.55"/>')
for x, y, c, t in back:
    body += plug(x, y, BS, c, t)
for x, y, c, t in front:
    body += plug(x, y, FS, c, t)
body.append('<rect x="0" y="0" width="512" height="512" fill="url(#vig)"/>')
body.append('<rect x="16" y="16" width="480" height="240" fill="url(#gloss)"/>')

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">
<!-- PatchBox: patch cables plugged into a patch panel, in a rounded square. -->
<defs>
{chr(10).join(defs)}
</defs>
<g clip-path="url(#box)">
{chr(10).join(body)}
</g>
<rect x="17" y="17" width="478" height="478" rx="107" fill="none" stroke="#ffffff" stroke-opacity="0.10" stroke-width="2"/>
<rect x="16" y="16" width="480" height="480" rx="108" fill="none" stroke="#000" stroke-opacity="0.55" stroke-width="2"/>
</svg>
'''
open(sys.argv[1], 'w').write(svg)
