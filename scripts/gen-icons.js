// 生成应用图标：深色圆角方块 + 琥珀色圆环。
// 没有图形库，所以直接算像素、自己写 PNG 编码。跑一次就够，产物提交到仓库。

const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

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

function render(size) {
  const SS = 4;
  const W = size * SS;
  const buf = Buffer.alloc(W * W * 4);
  const c = W / 2;
  const radius = W * 0.24;

  // 杏仁形眼型 = 两个大圆的交集
  const lensR = W * 0.48;
  const lensK = W * 0.315;
  const hiR = W * 0.042;

  // 暖色底 + 深色眼，任务栏里比黑底更容易认出来
  const bgTop = [255, 205, 133];
  const bgBot = [226, 122, 46];
  const eye = [40, 23, 8];
  const highlight = [255, 250, 240];

  for (let y = 0; y < W; y++) {
    for (let x = 0; x < W; x++) {
      const px = x + 0.5;
      const py = y + 0.5;

      const dx = Math.max(Math.abs(px - c) - (W / 2 - radius), 0);
      const dy = Math.max(Math.abs(py - c) - (W / 2 - radius), 0);
      const rectDist = Math.hypot(dx, dy) - radius;

      const inLens =
        Math.hypot(px - c, py - (c - lensK)) <= lensR &&
        Math.hypot(px - c, py - (c + lensK)) <= lensR;

      const dHi = Math.hypot(px - (c - W * 0.055), py - (c - W * 0.05));

      // 1px 的软边缘，避免小尺寸下出现锯齿
      const alpha = Math.min(1, Math.max(0, 1 - rectDist));
      const t = py / W;
      let color = [
        Math.round(bgTop[0] + (bgBot[0] - bgTop[0]) * t),
        Math.round(bgTop[1] + (bgBot[1] - bgTop[1]) * t),
        Math.round(bgTop[2] + (bgBot[2] - bgTop[2]) * t),
      ];
      if (inLens) color = eye;
      if (inLens && dHi <= hiR) color = highlight;

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

function ico(png, size) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(1, 4);
  const entry = Buffer.alloc(16);
  entry[0] = size >= 256 ? 0 : size;
  entry[1] = size >= 256 ? 0 : size;
  entry.writeUInt16LE(1, 4);
  entry.writeUInt16LE(32, 6);
  entry.writeUInt32LE(png.length, 8);
  entry.writeUInt32LE(22, 12);
  return Buffer.concat([header, entry, png]);
}

const dir = path.join(__dirname, '..', 'app', 'icons');
fs.mkdirSync(dir, { recursive: true });

const big = render(256);
fs.writeFileSync(path.join(dir, 'icon.png'), big.png);
fs.writeFileSync(path.join(dir, 'icon.ico'), ico(big.png, 256));
fs.writeFileSync(path.join(dir, '128x128.png'), render(128).png);
fs.writeFileSync(path.join(dir, '32x32.png'), render(32).png);
fs.writeFileSync(path.join(dir, '32x32@2x.png'), render(64).png);

console.log('图标已生成到', dir);
