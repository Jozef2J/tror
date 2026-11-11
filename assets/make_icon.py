from PIL import Image, ImageDraw, ImageFilter
S = 1024
def poly_mask(points_list):
    m = Image.new('L', (S, S), 0)
    d = ImageDraw.Draw(m)
    for pts in points_list:
        d.polygon(pts, fill=255)
    return m
import math
cx = 452
def stroke(a, b, w):
    dx, dy = b[0]-a[0], b[1]-a[1]
    n = math.hypot(dx, dy); px, py = -dy/n*w/2, dx/n*w/2
    return [(a[0]+px, a[1]+py), (b[0]+px, b[1]+py), (b[0]-px, b[1]-py), (a[0]-px, a[1]-py)]
W = 104
apex = (cx, 60)
stem = [(cx-W/2, 150), (cx+W/2, 150), (cx+W/2, 915), (cx, 968), (cx-W/2, 915)]
left = [apex, (cx, 60+W*1.42), (cx-215, 60+W*1.42+215), (cx-215-W*0.71, 60+215+W*0.71)]
right = [apex, (cx, 60+W*1.42), (cx+215, 60+W*1.42+215), (cx+215+W*0.71, 60+215+W*0.71)]
outer = [(cx+20, 412), (cx+345, 610), (cx+20, 808)]
inner = [(cx+52, 544), (cx+160, 610), (cx+52, 676)]
leg = stroke((cx+70, 742), (cx+305, 955), W*0.92)
mask = poly_mask([stem, left, right, outer, leg])
ImageDraw.Draw(mask).polygon(inner, fill=0)
m2 = Image.new('L', (S, S), 0); ImageDraw.Draw(m2).rectangle([0, 0, S, 1000], fill=255)
from PIL import ImageChops
mask = ImageChops.multiply(mask, m2)
grad = Image.new('RGBA', (S, S))
top, bottom = (137, 180, 250), (180, 142, 247)
gd = ImageDraw.Draw(grad)
for y in range(S):
    t = y / (S - 1)
    gd.line([(0, y), (S, y)], fill=tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)) + (255,))
glyph = Image.new('RGBA', (S, S), (0, 0, 0, 0))
glyph.paste(grad, (0, 0), mask)
hi = poly_mask([[(cx-W/2, 150), (cx-W/2+34, 150), (cx-W/2+34, 915), (cx-W/2, 915)]])
light = Image.new('RGBA', (S, S), (255, 255, 255, 46))
glyph.paste(Image.alpha_composite(glyph, Image.composite(light, Image.new('RGBA', (S, S), (0, 0, 0, 0)), hi)), (0, 0), mask)
edge = mask.filter(ImageFilter.MaxFilter(21))
outline = Image.new('RGBA', (S, S), (24, 24, 37, 0))
outline.putalpha(edge.point(lambda v: int(v * 0.55)))
icon = Image.alpha_composite(outline, glyph)
icon.resize((256, 256), Image.LANCZOS).save('assets/tror.png')
icon.resize((256, 256), Image.LANCZOS).save('assets/tror.ico', sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
