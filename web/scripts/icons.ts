/**
 * Generates the PWA icons (PNG + SVG) from the hush mark, with no image
 * dependency: shapes are rasterized with 4×4 supersampling and written by a
 * minimal PNG encoder. Run with `bun scripts/icons.ts`; output is committed.
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { deflateSync } from "node:zlib";

const OUT = resolve(import.meta.dir, "../public/icons");
const BG: [number, number, number] = [91, 69, 196];
const FG: [number, number, number] = [250, 250, 255];

// Geometry on the 32-unit grid of the in-app logo (src/ui/components/Icons.tsx).
const BUBBLE = { cx: 16, cy: 14, hx: 8, hy: 6, r: 3, width: 2 };
const TAIL: [number, number, number, number][] = [
  [12, 20, 11, 23.5],
  [11, 23.5, 15, 20],
];
const MOUTH = { x0: 12.5, x1: 19.5, y: 14, width: 2.2 };

function crc32(buf: Uint8Array): number {
  let c = ~0;
  for (const b of buf) {
    c ^= b;
    for (let k = 0; k < 8; k++) c = c & 1 ? (c >>> 1) ^ 0xedb88320 : c >>> 1;
  }
  return ~c >>> 0;
}

function chunk(type: string, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + data.length);
  const dv = new DataView(out.buffer);
  dv.setUint32(0, data.length);
  out.set(new TextEncoder().encode(type), 4);
  out.set(data, 8);
  dv.setUint32(8 + data.length, crc32(out.subarray(4, 8 + data.length)));
  return out;
}

function png(size: number, rgba: Uint8Array): Uint8Array {
  const raw = new Uint8Array(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0;
    raw.set(rgba.subarray(y * size * 4, (y + 1) * size * 4), y * (size * 4 + 1) + 1);
  }
  const ihdr = new Uint8Array(13);
  const dv = new DataView(ihdr.buffer);
  dv.setUint32(0, size);
  dv.setUint32(4, size);
  ihdr.set([8, 6, 0, 0, 0], 8);
  const parts = [
    new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr),
    chunk("IDAT", new Uint8Array(deflateSync(raw, { level: 9 }))),
    chunk("IEND", new Uint8Array()),
  ];
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

function segment(x: number, y: number, [ax, ay, bx, by]: [number, number, number, number]): number {
  const dx = bx - ax;
  const dy = by - ay;
  const t = Math.max(0, Math.min(1, ((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)));
  return Math.hypot(x - (ax + t * dx), y - (ay + t * dy));
}

/** Coverage of the mark at a point of the 32-grid: [background, foreground alpha]. */
function sample(x: number, y: number, rounded: boolean): [number, number] {
  let bg = 1;
  if (rounded) {
    const r = 9;
    const qx = Math.max(Math.abs(x - 16) - (16 - r), 0);
    const qy = Math.max(Math.abs(y - 16) - (16 - r), 0);
    if (Math.hypot(qx, qy) > r) bg = 0;
  }
  // Rounded-rectangle outline of the bubble (signed distance), open where the tail starts.
  const qx = Math.abs(x - BUBBLE.cx) - (BUBBLE.hx - BUBBLE.r);
  const qy = Math.abs(y - BUBBLE.cy) - (BUBBLE.hy - BUBBLE.r);
  const d = Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - BUBBLE.r;
  const inGap = y > BUBBLE.cy && x > 12.2 && x < 14.8;
  let fg = Math.abs(d) <= BUBBLE.width / 2 && !inGap ? 1 : 0;
  for (const s of TAIL) if (segment(x, y, s) <= BUBBLE.width / 2) fg = 1;
  if (segment(x, y, [MOUTH.x0, MOUTH.y, MOUTH.x1, MOUTH.y]) <= MOUTH.width / 2) fg = 1;
  return [bg, fg];
}

function render(size: number, opts: { rounded: boolean; scale: number }): Uint8Array {
  const rgba = new Uint8Array(size * size * 4);
  const ss = 4;
  for (let py = 0; py < size; py++) {
    for (let px = 0; px < size; px++) {
      let bgA = 0;
      let fgA = 0;
      for (let sy = 0; sy < ss; sy++) {
        for (let sx = 0; sx < ss; sx++) {
          const u = ((px + (sx + 0.5) / ss) / size) * 32;
          const v = ((py + (sy + 0.5) / ss) / size) * 32;
          // The mark is scaled around the center (maskable icons keep it in the safe zone).
          const [, f] = sample(16 + (u - 16) / opts.scale, 16 + (v - 16) / opts.scale, false);
          const [b] = sample(u, v, opts.rounded);
          bgA += b;
          fgA += f * b;
        }
      }
      bgA /= ss * ss;
      fgA /= ss * ss;
      const i = (py * size + px) * 4;
      const t = bgA > 0 ? fgA / bgA : 0;
      for (let c = 0; c < 3; c++)
        rgba[i + c] = Math.round((BG[c] as number) * (1 - t) + (FG[c] as number) * t);
      rgba[i + 3] = Math.round(bgA * 255);
    }
  }
  return png(size, rgba);
}

const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" role="img"><title>hush</title><rect width="32" height="32" rx="9" fill="rgb(${BG.join(",")})"/><path d="M8 11a3 3 0 0 1 3-3h10a3 3 0 0 1 3 3v6a3 3 0 0 1-3 3h-6l-4 3.5V20a3 3 0 0 1-3-3z" fill="none" stroke="rgb(${FG.join(",")})" stroke-width="2" stroke-linejoin="round"/><path d="M12.5 14h7" stroke="rgb(${FG.join(",")})" stroke-width="2.2" stroke-linecap="round"/></svg>\n`;

mkdirSync(OUT, { recursive: true });
writeFileSync(join(OUT, "icon.svg"), svg);
writeFileSync(join(OUT, "icon-192.png"), render(192, { rounded: true, scale: 1 }));
writeFileSync(join(OUT, "icon-512.png"), render(512, { rounded: true, scale: 1 }));
writeFileSync(join(OUT, "maskable-512.png"), render(512, { rounded: false, scale: 0.72 }));
writeFileSync(join(OUT, "apple-touch-icon.png"), render(180, { rounded: false, scale: 0.86 }));
console.log(`icons written to ${OUT}`);
