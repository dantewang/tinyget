// Builds the tinyget application icon: assets/tinyget.ico, ui/assets/icon.svg,
// ui/assets/icon-32.png and design/icon-256.png.
//
// The mark is "Ascent": a paper arrow over a signal-yellow base bar on the ink
// tile. Sizes 16, 20 and 24 are hand-placed pixel by pixel, because scaling the
// vector down that far softens every edge and thins the base bar to nothing.
// From 32 up the vector is rendered with 4x4 supersampling, with the axis-aligned
// edges snapped to whole pixels so only the arrowhead's diagonals get antialiased.
//
// No dependencies: PNG and ICO are written by hand. Run with `node tools/make-icon.mjs`.

import zlib from "node:zlib";
import fs from "node:fs";
import path from "node:path";

const INK    = [0x19, 0x19, 0x19];
const PAPER  = [0xF2, 0xF2, 0xEC];
const SIGNAL = [0xDC, 0xE4, 0x00];

// ---------------------------------------------------------------- hand-placed sizes

/** Inclusive [row, xFrom, xTo] spans. */
function rows(from, to, x0, x1) {
  return Array.from({ length: to - from + 1 }, (_, i) => [from + i, x0, x1]);
}

/** A 45-degree arrowhead: each row two pixels wider than the one above. */
function head(topRow, centreLeft, count) {
  return Array.from({ length: count }, (_, i) => [topRow + i, centreLeft - i, centreLeft + 1 + i]);
}

const HAND = {
  16: {
    arrow: [...head(2, 7, 6), ...rows(8, 11, 6, 9)],
    bar: rows(13, 14, 2, 13),
  },
  20: {
    arrow: [...head(3, 9, 7), ...rows(10, 14, 7, 12)],
    bar: rows(16, 17, 3, 16),
  },
  24: {
    arrow: [...head(3, 11, 8), ...rows(11, 16, 9, 14)],
    bar: rows(18, 20, 4, 19),
  },
};

function handTuned(size) {
  const { arrow, bar } = HAND[size];
  const px = Buffer.alloc(size * size * 4);
  const put = (x, y, [r, g, b]) => {
    const i = (y * size + x) * 4;
    px[i] = r; px[i + 1] = g; px[i + 2] = b; px[i + 3] = 255;
  };
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) put(x, y, INK);
  for (const [y, x0, x1] of arrow) for (let x = x0; x <= x1; x++) put(x, y, PAPER);
  for (const [y, x0, x1] of bar) for (let x = x0; x <= x1; x++) put(x, y, SIGNAL);
  return px;
}

// ---------------------------------------------------------------- vector sizes

/** The mark on a 64-unit grid. */
const G = {
  apexY: 8, shoulderY: 28, shaftBottom: 44,
  headLeft: 14, headRight: 50, shaftLeft: 24, shaftRight: 40,
  barLeft: 16, barRight: 48, barTop: 50, barBottom: 56,
};

function geometry(size) {
  const s = (v) => Math.round((v * size) / 64);
  return {
    poly: [
      [size / 2, s(G.apexY)],
      [s(G.headRight), s(G.shoulderY)],
      [s(G.shaftRight), s(G.shoulderY)],
      [s(G.shaftRight), s(G.shaftBottom)],
      [s(G.shaftLeft), s(G.shaftBottom)],
      [s(G.shaftLeft), s(G.shoulderY)],
      [s(G.headLeft), s(G.shoulderY)],
    ],
    bar: [s(G.barLeft), s(G.barTop), s(G.barRight), s(G.barBottom)],
  };
}

function inPolygon(px, py, poly) {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i], [xj, yj] = poly[j];
    if ((yi > py) !== (yj > py) && px < ((xj - xi) * (py - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

const mix = (a, b, t) => a.map((v, i) => Math.round(v + (b[i] - v) * t));

function vector(size) {
  const { poly, bar } = geometry(size);
  const [bx0, by0, bx1, by1] = bar;
  const px = Buffer.alloc(size * size * 4);
  const SS = 4, total = SS * SS;

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let arrowHits = 0, barHits = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const px_ = x + (sx + 0.5) / SS, py_ = y + (sy + 0.5) / SS;
          if (inPolygon(px_, py_, poly)) arrowHits++;
          if (px_ >= bx0 && px_ < bx1 && py_ >= by0 && py_ < by1) barHits++;
        }
      }
      let c = INK;
      if (arrowHits) c = mix(c, PAPER, arrowHits / total);
      if (barHits) c = mix(c, SIGNAL, barHits / total);
      const i = (y * size + x) * 4;
      px[i] = c[0]; px[i + 1] = c[1]; px[i + 2] = c[2]; px[i + 3] = 255;
    }
  }
  return px;
}

const render = (size) => (HAND[size] ? handTuned(size) : vector(size));

// ---------------------------------------------------------------- PNG

const CRC_TABLE = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();

function crc32(buf) {
  let c = -1;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function png(size, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8;   // bit depth
  ihdr[9] = 6;   // truecolour with alpha
  const stride = size * 4 + 1;
  const raw = Buffer.alloc(stride * size);
  for (let y = 0; y < size; y++) {
    raw[y * stride] = 0; // no filter
    rgba.copy(raw, y * stride + 1, y * size * 4, (y + 1) * size * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", zlib.deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ---------------------------------------------------------------- ICO

/** A BITMAPINFOHEADER entry: bottom-up BGRA plus an all-zero AND mask. */
function dib(size, rgba) {
  const header = Buffer.alloc(40);
  header.writeUInt32LE(40, 0);
  header.writeInt32LE(size, 4);
  header.writeInt32LE(size * 2, 8); // colour data + mask
  header.writeUInt16LE(1, 12);      // planes
  header.writeUInt16LE(32, 14);     // bits per pixel
  header.writeUInt32LE(0, 16);      // BI_RGB
  header.writeUInt32LE(size * size * 4, 20);

  const px = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    const src = (size - 1 - y) * size * 4;
    for (let x = 0; x < size; x++) {
      const d = (y * size + x) * 4, s = src + x * 4;
      px[d] = rgba[s + 2]; px[d + 1] = rgba[s + 1]; px[d + 2] = rgba[s]; px[d + 3] = rgba[s + 3];
    }
  }
  const maskStride = Math.ceil(size / 8 / 4) * 4;
  return Buffer.concat([header, px, Buffer.alloc(maskStride * size, 0)]);
}

// Vista and later read PNG entries at any size, but a DIB is what every shell
// path has always understood, so PNG is used only where it saves real bytes.
const SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256];
const AS_PNG = new Set([128, 256]);

function ico() {
  const images = SIZES.map((size) => {
    const rgba = render(size);
    return { size, data: AS_PNG.has(size) ? png(size, rgba) : dib(size, rgba) };
  });

  const dir = Buffer.alloc(6);
  dir.writeUInt16LE(0, 0);
  dir.writeUInt16LE(1, 2); // type: icon
  dir.writeUInt16LE(images.length, 4);

  let offset = 6 + 16 * images.length;
  const entries = images.map(({ size, data }) => {
    const e = Buffer.alloc(16);
    e[0] = size === 256 ? 0 : size; // 0 means 256
    e[1] = size === 256 ? 0 : size;
    e.writeUInt16LE(1, 4);   // planes
    e.writeUInt16LE(32, 6);  // bits per pixel
    e.writeUInt32LE(data.length, 8);
    e.writeUInt32LE(offset, 12);
    offset += data.length;
    return e;
  });

  return Buffer.concat([dir, ...entries, ...images.map((i) => i.data)]);
}

// ---------------------------------------------------------------- SVG

/// Derived from the same geometry the rasteriser uses, so the two cannot drift.
function svgMark() {
  const hex = (c) => "#" + c.map((v) => v.toString(16).padStart(2, "0")).join("").toUpperCase();
  const arrow =
    `M32 ${G.apexY} L${G.headRight} ${G.shoulderY} H${G.shaftRight} V${G.shaftBottom} ` +
    `H${G.shaftLeft} V${G.shoulderY} H${G.headLeft} Z`;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">
  <rect width="64" height="64" fill="${hex(INK)}"/>
  <path d="${arrow}" fill="${hex(PAPER)}"/>
  <rect x="${G.barLeft}" y="${G.barTop}" width="${G.barRight - G.barLeft}" height="${G.barBottom - G.barTop}" fill="${hex(SIGNAL)}"/>
</svg>
`;
}

// ---------------------------------------------------------------- write

export { render, png, SIZES };

// Only write when run directly, so the preview sheet can import the renderer.
if (path.resolve(process.argv[1] ?? "") === path.resolve(import.meta.filename)) {
  const root = path.resolve(import.meta.dirname, "..");
  const write = (rel, buf) => {
    const file = path.join(root, rel);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, buf);
    console.log(`${rel.padEnd(28)} ${buf.length.toLocaleString()} bytes`);
  };

  write("ui/assets/icon.svg", svgMark());
  write("assets/tinyget.ico", ico());
  // Slint sets its own default window icon unless given one, and that overrides
  // the executable resource, so the window needs its own copy. 32 px halves
  // cleanly to the 16 px the title bar draws.
  write("ui/assets/icon-32.png", png(32, render(32)));
  write("design/icon-256.png", png(256, render(256)));
  console.log(`sizes in the .ico: ${SIZES.join(", ")}`);
}
