// DOM/pixel regression only. This does not emulate iOS's native scroll pocket
// and is separate from the sandboxed, real-WebGPU smoke in smoke.mjs.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {test} from 'node:test';
import {chromium} from 'playwright';

test('standalone status background leaves canvas pixels, geometry and input intact', async () => {
  const source = await fs.readFile(new URL('../../examples/widget-factory/index.html', import.meta.url), 'utf8');
  // Run the shipped host HTML/CSS and launch-mode script, without loading Wasm.
  const html = source.replace(/<script type="module">[\s\S]*?<\/script>/, '');
  const browser = await chromium.launch({headless:true});
  try {
    for (const mode of ['browser', 'legacy', 'standalone', 'fullscreen']) {
      for (const colorScheme of ['light', 'dark']) {
        const context = await browser.newContext({viewport:{width:390,height:844},deviceScaleFactor:2,colorScheme});
        await context.route('**/*', route => route.abort());
        const page = await context.newPage();
        await page.evaluate(mode => {
          Object.defineProperty(navigator, 'standalone', {configurable:true,value:mode==='legacy'});
          const nativeMatchMedia = window.matchMedia.bind(window);
          const queries = new Map();
          window.matchMedia = query => {
            if (!query.startsWith('(display-mode:')) return nativeMatchMedia(query);
            const result = new EventTarget();
            result.matches = query===`(display-mode: ${mode})`;
            queries.set(query,result);
            return result;
          };
          window.setTestDisplayMode = next => {
            Object.defineProperty(navigator, 'standalone', {configurable:true,value:false});
            for (const [query,result] of queries) {
              result.matches = query===`(display-mode: ${next})`;
              result.dispatchEvent(new Event('change'));
            }
          };
        }, mode);
        await page.setContent(html);
        await page.evaluate(() => {
          document.querySelector('#status').hidden = true;
          const canvas = document.querySelector('#ui');
          canvas.width = 390; canvas.height = 844;
          const ctx = canvas.getContext('2d');
          ctx.fillStyle = '#9370db'; ctx.fillRect(0,0,390,844);
          ctx.fillStyle = '#111'; ctx.font = '12px sans-serif';
          ctx.fillText('Factory  View  Tools', 4, 10);
          window.canvasClicks = 0;
          canvas.addEventListener('click', () => window.canvasClicks++);
        });
        for (const viewport of [{width:390,height:844},{width:844,height:390},{width:390,height:844}]) {
          await page.setViewportSize(viewport);
          const actual = await page.evaluate(() => {
            const strip = document.querySelector('#ios-status-background');
            const css = getComputedStyle(strip);
            const canvas = document.querySelector('#ui').getBoundingClientRect();
            const box = strip.getBoundingClientRect();
            return {display:css.display,position:css.position,color:css.backgroundColor,
              clip:css.backgroundClip,opacity:css.opacity,pointer:css.pointerEvents,
              strip:[box.x,box.y,box.width,box.height],canvas:[canvas.x,canvas.y,canvas.width,canvas.height],
              target:document.elementFromPoint(innerWidth/2,4)?.id,
              empty:strip.childNodes.length===0,aria:strip.getAttribute('aria-hidden')};
          });
          assert.deepEqual(actual.canvas,[0,0,viewport.width,viewport.height]);
          assert.equal(actual.target,'ui');
          assert.equal(actual.empty,true);
          assert.equal(actual.aria,'true');
          if (mode==='browser') assert.equal(actual.display,'none');
          else {
            assert.equal(actual.position,'fixed');
            assert.equal(actual.color,'rgb(255, 255, 255)');
            assert.equal(actual.clip,'text');
            assert.equal(actual.opacity,'1');
            assert.equal(actual.pointer,'none');
            assert.deepEqual(actual.strip,[0,0,viewport.width,11]);
          }
          const withStrip = await page.screenshot();
          // A future app theme may update the declared surface. The empty strip
          // must still paint nothing, including while its CSS color changes.
          for (const [color,computed] of [['#23272c','rgb(35, 39, 44)'],['transparent','rgba(0, 0, 0, 0)']]) {
            await page.evaluate(color => document.documentElement.style.setProperty('--factory-surface',color),color);
            if (mode !== 'browser') assert.equal(await page.locator('#ios-status-background').evaluate(el => getComputedStyle(el).backgroundColor),computed);
            assert.deepEqual(await page.screenshot(), withStrip);
          }
          await page.evaluate(() => document.documentElement.style.removeProperty('--factory-surface'));
          await page.evaluate(() => document.querySelector('#ios-status-background').style.display='none');
          assert.deepEqual(await page.screenshot(), withStrip, 'status strip must not paint over menu text or content');
          await page.evaluate(() => document.querySelector('#ios-status-background').style.removeProperty('display'));
          await page.mouse.click(viewport.width/2,4);
        }
        assert.equal(await page.evaluate(() => window.canvasClicks),3);
        await page.evaluate(() => {
          document.documentElement.removeAttribute('data-standalone');
          dispatchEvent(new Event('pageshow'));
        });
        assert.equal(await page.evaluate(() => document.documentElement.hasAttribute('data-standalone')),mode!=='browser');
        for (const next of ['browser','standalone','fullscreen','browser']) {
          await page.evaluate(next => window.setTestDisplayMode(next),next);
          assert.equal(await page.evaluate(() => document.documentElement.hasAttribute('data-standalone')),next!=='browser');
        }
        await context.close();
      }
    }
  } finally { await browser.close(); }
});
