// 把 screen-shot 产出的 32 位 BMP 转成 PNG。
// screen-shot 只会写 BMP（不想在 Rust 里手写 deflate），文档里要贴图时用这个转一下。
//
// 用法：node scripts/bmp-to-png.js <输入.bmp> <输出.png>

const fs = require("fs");
const zlib = require("zlib");

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
  const t = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([t, data])));
  return Buffer.concat([len, t, data, crc]);
}

const [input, output] = process.argv.slice(2);
if (!input || !output) {
  console.error("用法：node scripts/bmp-to-png.js <输入.bmp> <输出.png>");
  process.exit(1);
}

const bmp = fs.readFileSync(input);
const dataOffset = bmp.readUInt32LE(10);
const width = bmp.readInt32LE(18);
const height = bmp.readInt32LE(22);
const bpp = bmp.readUInt16LE(28);
const bytesPerPixel = bpp / 8;
const rowSize = Math.floor((bpp * width + 31) / 32) * 4;

if (bpp !== 32) {
  console.error(`只支持 32 位 BMP，当前是 ${bpp} 位`);
  process.exit(1);
}

// BMP 是自下而上存的，PNG 要自上而下
const pixels = Buffer.alloc(width * height * 4);
for (let y = 0; y < height; y++) {
  const srcRow = height - 1 - y;
  for (let x = 0; x < width; x++) {
    const s = dataOffset + srcRow * rowSize + x * bytesPerPixel;
    const d = (y * width + x) * 4;
    pixels[d] = bmp[s + 2];
    pixels[d + 1] = bmp[s + 1];
    pixels[d + 2] = bmp[s];
    pixels[d + 3] = 255; // 截图没有透明区域，Alpha 一律不透明
  }
}

const stride = width * 4;
const raw = Buffer.alloc((stride + 1) * height);
for (let y = 0; y < height; y++) {
  raw[y * (stride + 1)] = 0; // 每行的过滤器类型：None
  pixels.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(width, 0);
ihdr.writeUInt32BE(height, 4);
ihdr[8] = 8;
ihdr[9] = 6;

const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk("IHDR", ihdr),
  chunk("IDAT", zlib.deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

fs.writeFileSync(output, png);
console.log(`已生成 ${output}  ${width}x${height}`);
