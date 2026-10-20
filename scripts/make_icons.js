// Makes the website's icons from Chat Computer's icon (chatcomputer/ui/assets/icon.svg), drawn by Chromium so each
// size is drawn for itself rather than shrunk from a bigger one:
//   docs/favicon.ico            16, 32 and 48 pixels, for browsers that don't take the SVG (scripts/build_demo.py
//                               puts the SVG itself in docs/favicon.svg, which the others take)
//   docs/apple-touch-icon.png   180 pixels, on the screen's off-white, for a phone's or tablet's home screen
// Run it after changing the icon:  NODE_PATH=$(npm root -g) node scripts/make_icons.js
const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright");

const ROOT = path.resolve(__dirname, "..");
const SVG = fs.readFileSync(path.join(ROOT, "chatcomputer", "ui", "assets", "icon.svg"), "utf8");
const DOCS = path.join(ROOT, "docs");
const GROUND = "#F8F5F0";            // the screen's off-white (chatcomputer/ui/assets/theme.css)

// The icon at `size` pixels, its own `scale` of the square (the rest: `ground`, or see-through).
async function draw(page, size, scale, ground) {
  const inner = Math.round(size * scale);
  const html = `<!doctype html><html><body style="margin:0;background:${ground || "transparent"}">` +
    `<div style="width:${size}px;height:${size}px;display:flex;align-items:center;justify-content:center">` +
    SVG.replace(/(<svg[^>]*?) width="\d+" height="\d+"/, `$1 width="${inner}" height="${inner}"`) + `</div></body></html>`;
  await page.setViewportSize({ width: size, height: size });
  await page.setContent(html);
  return page.screenshot({ omitBackground: !ground, clip: { x: 0, y: 0, width: size, height: size } });
}

// An .ico of PNG images (as Windows Vista and every browser since read them).
function ico(pngs) {
  const head = Buffer.alloc(6 + 16 * pngs.length);
  head.writeUInt16LE(0, 0);
  head.writeUInt16LE(1, 2);                          // an icon
  head.writeUInt16LE(pngs.length, 4);
  let at = head.length;
  pngs.forEach(({ size, data }, i) => {
    const e = 6 + 16 * i;
    head.writeUInt8(size >= 256 ? 0 : size, e);
    head.writeUInt8(size >= 256 ? 0 : size, e + 1);
    head.writeUInt8(0, e + 2);                       // no palette
    head.writeUInt8(0, e + 3);
    head.writeUInt16LE(1, e + 4);                    // planes
    head.writeUInt16LE(32, e + 6);                   // bits per pixel
    head.writeUInt32LE(data.length, e + 8);
    head.writeUInt32LE(at, e + 12);
    at += data.length;
  });
  return Buffer.concat([head].concat(pngs.map((p) => p.data)));
}

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ deviceScaleFactor: 1 });
  const pngs = [];
  for (const size of [16, 32, 48]) pngs.push({ size, data: await draw(page, size, 1) });
  fs.writeFileSync(path.join(DOCS, "favicon.ico"), ico(pngs));
  // A home screen's icon fills its square (the phone rounds its corners): the monitor, smaller, on the off-white.
  fs.writeFileSync(path.join(DOCS, "apple-touch-icon.png"), await draw(page, 180, 0.8, GROUND));
  await browser.close();
  console.log("wrote docs/favicon.ico, docs/apple-touch-icon.png");
})().catch((e) => { console.error(e); process.exit(1); });
