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

// Locate the enabled Toggle's compact accent track in the content column.
// Slider/ProgressView tracks are wider and thinner; navigation is excluded by
// the caller. Returning its own bounds keeps the off assertion just as strict.
export function togglePixels(image, rectangle) {
  let band;
  const candidates=[];
  for(let y=rectangle.y;y<=rectangle.y+rectangle.height;y++) {
    const row=y<rectangle.y+rectangle.height
      ? coloredPixels(image,{...rectangle,y,height:1},red) : {count:0};
    if(row.count) {
      band ??= {y,height:0};
      band.height++;
    } else if(band) {
      if(band.height>=10 && band.height<=22) {
        const area={...rectangle,...band};
        const pixels=coloredPixels(image,area,red);
        if(pixels.count>60 && pixels.count<800) candidates.push({...pixels,area});
      }
      band=undefined;
    }
  }
  return candidates.length===1 ? candidates[0] : null;
}

export const white = (r, g, b) => r > 235 && g > 235 && b > 235;
export function hasTextOnWhite(image, rectangle, minimumInk) {
  return coloredPixels(image, rectangle, dark).count > minimumInk &&
    coloredPixels(image, rectangle, white).count > rectangle.width * rectangle.height * 0.6;
}

// Find a reachable document scroll offset with no vertical intersection.
// A compact hero can remain visible at scrollY=0, so zero is not a pause test.
export function offscreenScrollTop({top, bottom, viewportHeight, scrollHeight}) {
  const maximum = Math.max(0, scrollHeight - viewportHeight);
  const after = Math.ceil(bottom) + 1;
  if (after <= maximum) return after;
  const before = Math.floor(top - viewportHeight) - 1;
  if (before >= 0 && before <= maximum) return before;
  throw new Error('Cannot scroll the entire specimen outside this viewport');
}
