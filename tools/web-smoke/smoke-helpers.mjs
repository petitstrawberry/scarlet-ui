import path from 'node:path';

// Never forward workflow secrets/tokens or the complete runner environment.
export function browserEnvironment(source = process.env) {
  const result = {};
  for (const key of ['PATH', 'HOME', 'TMPDIR', 'TMP', 'TEMP', 'DISPLAY', 'XAUTHORITY', 'XDG_RUNTIME_DIR', 'LANG', 'LC_ALL']) {
    if (source[key]) result[key] = source[key];
  }
  return result;
}

export function localRequest(url, origin) {
  try { return new URL(url).origin === origin; } catch { return false; }
}

export function artifactPath(root, pathname) {
  const decoded = decodeURIComponent(pathname);
  const destination = path.resolve(root, `.${decoded}`);
  const relative = path.relative(root, destination);
  if (relative.startsWith('..') || path.isAbsolute(relative)) throw new Error('Path escapes static artifact');
  return destination;
}

export function changedPixels(a, b, rectangle) {
  if (a.width !== b.width || a.height !== b.height) throw new Error('Image dimensions differ');
  let changed = 0;
  const {x, y, width, height} = rectangle;
  for (let row = y; row < y + height; row++) for (let col = x; col < x + width; col++) {
    const i = (row * a.width + col) * 4;
    if (Math.max(...[0, 1, 2].map(c => Math.abs(a.data[i+c] - b.data[i+c]))) > 15) changed++;
  }
  return changed;
}

export function coloredPixels(image, rectangle, predicate) {
  let count = 0, sumX = 0, sumY = 0;
  const {x, y, width, height} = rectangle;
  for (let row = y; row < y + height; row++) for (let col = x; col < x + width; col++) {
    const i = (row * image.width + col) * 4;
    if (predicate(image.data[i], image.data[i+1], image.data[i+2])) { count++; sumX += col; sumY += row; }
  }
  return {count, x: count ? sumX / count : null, y: count ? sumY / count : null};
}
export const red = (r, g, b) => r > 170 && g < 95 && b < 105;
export const dark = (r, g, b) => r < 150 && g < 150 && b < 150;
