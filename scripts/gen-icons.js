// 生成应用图标：蓝色圆角方块 + 白色眼睑。
// 没有图形库，所以直接算像素、自己写 PNG 编码。跑一次就够，产物提交到仓库。
//
// 图形定义（200x200 坐标系）：
//   底板  = 圆角方块，圆角半径 50
//   眼睑  = 两条二次贝塞尔夹出的梭形，两端收尖
//          上弧 (30,77) Q(100,169) (170,77)
//           下弧 (170,77) Q(100,113) (30,77)
// 小尺寸（<=32px）换用加粗版本，否则弧线会细到看不见。

const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const BRAND = [82, 192, 133]; // #52C085 浅绿
const INK = [255, 255, 255];

const MARK_MAIN = [
  [30, 77, 100, 169, 170, 77],
  [170, 77, 100, 113, 30, 77],
];
const MARK_SMALL = [
  [22, 75, 100, 175, 178, 75],
  [178, 75, 100, 109, 22, 75],
];

function crc32(buf) {
  if (!crc32.table) {
    crc32.table = new Int32Array(256);
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      crc32.table[n] = c;
    }
  }
  let c = -1;
  for (let i = 0; i < buf.length; i++) c = crc32.table[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const t = Buffer.from(type, 'ascii');
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([t, data])));
  return Buffer.concat([len, t, data, crc]);
}

function encodePng(w, h, rgba) {
  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  const stride = w * 4;
  const raw = Buffer.alloc((stride + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (stride + 1)] = 0;
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  return Buffer.concat([
    sig,
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

// 把两条贝塞尔展平成多边形，用来做「点在形状内」判定
function flatten(mark, steps) {
  const pts = [];
  for (const [x0, y0, cx, cy, x1, y1] of mark) {
    for (let i = 0; i <= steps; i++) {
      const t = i / steps;
      const u = 1 - t;
      pts.push([
        u * u * x0 + 2 * u * t * cx + t * t * x1,
        u * u * y0 + 2 * u * t * cy + t * t * y1,
      ]);
    }
  }
  return pts;
}

function inside(px, py, poly) {
  let hit = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i];
    const [xj, yj] = poly[j];
    if (yi > py !== yj > py && px < ((xj - xi) * (py - yi)) / (yj - yi) + xi) hit = !hit;
  }
  return hit;
}

function render(size) {
  const SS = 4;
  const W = size * SS;
  const buf = Buffer.alloc(W * W * 4);
  const unit = W / 200;
  const c = W / 2;
  const radius = 50 * unit;
  const poly = flatten(size <= 32 ? MARK_SMALL : MARK_MAIN, 24).map(([x, y]) => [
    x * unit,
    y * unit,
  ]);

  for (let y = 0; y < W; y++) {
    for (let x = 0; x < W; x++) {
      const px = x + 0.5;
      const py = y + 0.5;

      const dx = Math.max(Math.abs(px - c) - (W / 2 - radius), 0);
      const dy = Math.max(Math.abs(py - c) - (W / 2 - radius), 0);
      const rectDist = Math.hypot(dx, dy) - radius;

      const color = inside(px, py, poly) ? INK : BRAND;
      // 1px 的软边缘，避免小尺寸下出现锯齿
      const alpha = Math.min(1, Math.max(0, 1 - rectDist));

      const i = (y * W + x) * 4;
      buf[i] = color[0];
      buf[i + 1] = color[1];
      buf[i + 2] = color[2];
      buf[i + 3] = Math.round(alpha * 255);
    }
  }

  // 超采样降回目标尺寸
  const out = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let r = 0, g = 0, b = 0, a = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const i = ((y * SS + sy) * W + (x * SS + sx)) * 4;
          const pa = buf[i + 3] / 255;
          r += buf[i] * pa;
          g += buf[i + 1] * pa;
          b += buf[i + 2] * pa;
          a += buf[i + 3];
        }
      }
      const n = SS * SS;
      const alphaSum = a / n;
      const scale = alphaSum > 0 ? 255 / alphaSum : 0;
      const o = (y * size + x) * 4;
      out[o] = Math.min(255, Math.round((r / n) * scale));
      out[o + 1] = Math.min(255, Math.round((g / n) * scale));
      out[o + 2] = Math.min(255, Math.round((b / n) * scale));
      out[o + 3] = Math.round(alphaSum);
    }
  }
  return { buffer: out, png: encodePng(size, size, out) };
}

// 多尺寸 ICO：Windows 会按场景挑最合适的一档，单档会导致某些位置拉伸发虚
function ico(entries) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(entries.length, 4);

  let offset = 6 + entries.length * 16;
  const dirs = [];
  for (const { size, png } of entries) {
    const d = Buffer.alloc(16);
    d[0] = size >= 256 ? 0 : size;
    d[1] = size >= 256 ? 0 : size;
    d.writeUInt16LE(1, 4);
    d.writeUInt16LE(32, 6);
    d.writeUInt32LE(png.length, 8);
    d.writeUInt32LE(offset, 12);
    offset += png.length;
    dirs.push(d);
  }
  return Buffer.concat([header, ...dirs, ...entries.map((e) => e.png)]);
}

const dir = path.join(__dirname, '..', 'app', 'icons');
fs.mkdirSync(dir, { recursive: true });

const png = {};
for (const size of [16, 24, 32, 48, 64, 128, 256]) png[size] = render(size).png;

fs.writeFileSync(path.join(dir, 'icon.png'), png[256]);
fs.writeFileSync(path.join(dir, '128x128.png'), png[128]);
fs.writeFileSync(path.join(dir, '32x32@2x.png'), png[64]);
fs.writeFileSync(path.join(dir, '32x32.png'), png[32]);
fs.writeFileSync(
  path.join(dir, 'icon.ico'),
  ico([16, 24, 32, 48, 64, 128, 256].map((size) => ({ size, png: png[size] })))
);

console.log('图标已生成到', dir);
