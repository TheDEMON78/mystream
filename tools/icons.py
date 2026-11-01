"""Dessine les icônes du plugin Stream Deck (touches 72/144 px, catégorie, plugin)."""
import math
import os
import sys
from PIL import Image, ImageDraw

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'com.bloopstudio.mystream.sdPlugin', 'imgs')
BG = (24, 25, 32, 255)
FG = (220, 224, 236, 255)
DIM = (120, 126, 145, 255)
RED = (229, 72, 77, 255)
WHITE = (255, 255, 255, 255)
S = 576  # dessin en grand, puis réduction (bords lissés)


def canvas(bg):
    img = Image.new('RGBA', (S, S), bg)
    return img, ImageDraw.Draw(img)


def w(n):
    return int(n * S / 144)


def scene(d, c):
    d.rounded_rectangle([w(40), w(36), w(112), w(84)], w(6), outline=c, width=w(7))
    d.rounded_rectangle([w(30), w(56), w(102), w(104)], w(6), fill=c)


def mic(d, c, slash=False, bg=BG):
    d.rounded_rectangle([w(58), w(28), w(86), w(82)], w(14), fill=c)
    d.arc([w(44), w(46), w(100), w(98)], 0, 180, fill=c, width=w(7))
    d.line([w(72), w(98), w(72), w(114)], fill=c, width=w(7))
    d.line([w(56), w(114), w(88), w(114)], fill=c, width=w(7))
    if slash:
        d.line([w(36), w(30), w(108), w(110)], fill=bg, width=w(16))
        d.line([w(36), w(30), w(108), w(110)], fill=c, width=w(8))


def eye(d, c, off=False, bg=BG):
    # Paupières : deux arcs de grands cercles qui se rejoignent aux coins de l'œil.
    r = 58
    d.arc([w(72 - r), w(112 - r), w(72 + r), w(112 + r)], 223, 317, fill=c, width=w(7))
    d.arc([w(72 - r), w(32 - r), w(72 + r), w(32 + r)], 43, 137, fill=c, width=w(7))
    d.ellipse([w(58), w(58), w(86), w(86)], fill=c)
    if off:
        d.line([w(34), w(34), w(110), w(110)], fill=bg, width=w(16))
        d.line([w(34), w(34), w(110), w(110)], fill=c, width=w(8))


def broadcast(d, c):
    d.ellipse([w(62), w(62), w(82), w(82)], fill=c)
    for r, a in ((26, 7), (44, 7)):
        box = [w(72 - r), w(72 - r), w(72 + r), w(72 + r)]
        d.arc(box, 140, 220, fill=c, width=w(a))
        d.arc(box, -40, 40, fill=c, width=w(a))


def record(d, c, filled=False):
    if filled:
        d.ellipse([w(44), w(44), w(100), w(100)], fill=c)
    else:
        d.ellipse([w(40), w(40), w(104), w(104)], outline=c, width=w(8))
        d.ellipse([w(56), w(56), w(88), w(88)], fill=c)


def transition(d, c):
    d.line([w(30), w(72), w(100), w(72)], fill=c, width=w(9))
    d.polygon([(w(92), w(48)), (w(120), w(72)), (w(92), w(96))], fill=c)
    d.rounded_rectangle([w(24), w(40), w(44), w(104)], w(4), outline=c, width=w(6))


def save(img, name, size):
    path = os.path.join(OUT, name)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    img.resize((size, size), Image.LANCZOS).save(f'{path}.png')
    img.resize((size * 2, size * 2), Image.LANCZOS).save(f'{path}@2x.png')


def key(name, draw, on=False):
    img, d = canvas(RED if on else BG)
    draw(d, WHITE if on else FG, RED if on else BG)
    save(img, f'actions/{name}', 72)


key('scene', lambda d, c, b: scene(d, c))
key('scene-on', lambda d, c, b: scene(d, c), on=True)
key('mute', lambda d, c, b: mic(d, c, bg=b))
key('mute-on', lambda d, c, b: mic(d, c, slash=True, bg=b), on=True)
key('source', lambda d, c, b: eye(d, c, bg=b))
key('source-off', lambda d, c, b: eye(d, DIM, off=True, bg=b))
key('stream', lambda d, c, b: broadcast(d, c))
key('stream-on', lambda d, c, b: broadcast(d, c), on=True)
key('record', lambda d, c, b: record(d, c))
key('record-on', lambda d, c, b: record(d, c, filled=True), on=True)
key('transition', lambda d, c, b: transition(d, c))

# Icône de la catégorie : blanche sur fond transparent (28 px).
img, d = canvas((0, 0, 0, 0))
broadcast(d, WHITE)
save(img, 'category', 28)

# Icône du plugin : pastille rouge de MyStream sur fond sombre arrondi.
img = Image.new('RGBA', (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
d.rounded_rectangle([0, 0, S - 1, S - 1], w(28), fill=BG)
d.ellipse([w(36), w(36), w(108), w(108)], fill=RED)
d.polygon([(w(64), w(54)), (w(64), w(90)), (w(92), w(72))], fill=WHITE)
save(img, 'plugin', 144)
print('Icônes générées dans', os.path.normpath(OUT))
