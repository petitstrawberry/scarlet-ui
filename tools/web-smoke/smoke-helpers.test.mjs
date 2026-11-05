import test from 'node:test';
import assert from 'node:assert/strict';
import {browserEnvironment, localRequest, artifactPath, changedPixels, coloredPixels, red, hasTextOnWhite, offscreenScrollTop, togglePixels} from './smoke-helpers.mjs';

test('browser environment strips workflow tokens and unknown keys', () => {
  assert.deepEqual(browserEnvironment({PATH:'/bin', HOME:'/tmp/home', DISPLAY:':99', GITHUB_TOKEN:'secret', ACTIONS_RUNTIME_TOKEN:'secret', AWS_SECRET_ACCESS_KEY:'secret', CUSTOM_SECRET:'secret'}), {PATH:'/bin', HOME:'/tmp/home', DISPLAY:':99'});
});
test('runtime URL allowlist pins protocol, host and port', () => {
  const origin = 'http://127.0.0.1:4321';
  assert.equal(localRequest(origin + '/counter/', origin), true);
  for(const u of ['https://example.com/', 'http://127.0.0.1:4322', 'http://localhost:4321', 'file:///tmp/test', 'data:text/plain,test']) assert.equal(localRequest(u, origin), false);
});
test('static artifact path cannot traverse out of root', () => {
  assert.equal(artifactPath('/tmp/site', '/counter/index.html'), '/tmp/site/counter/index.html');
  assert.throws(() => artifactPath('/tmp/site', '/%2e%2e/private'));
});
test('pixel assertions inspect real RGB changes within the selected area', () => {
  const a={width:2,height:1,data:new Uint8Array([255,255,255,255,255,255,255,255])};
  const b={width:2,height:1,data:new Uint8Array([220,20,30,255,255,255,255,255])};
  const area={x:0,y:0,width:2,height:1};
  assert.equal(changedPixels(a,b,area),1);
  assert.equal(coloredPixels(b,area,red).count,1);
});

test('toggle detection excludes thin progress/slider tracks and fails on missing or ambiguous toggles', () => {
  const image={width:200,height:160,data:new Uint8Array(200*160*4).fill(255)};
  const area={x:20,y:10,width:170,height:140};
  const fill=(x,y,width,height)=>{
    for(let row=y;row<y+height;row++) for(let col=x;col<x+width;col++)
      image.data.set([200,30,40,255],(row*image.width+col)*4);
  };
  fill(30,110,150,4);fill(30,135,120,6);
  assert.equal(togglePixels(image,area),null);
  fill(50,72,28,16);
  const found=togglePixels(image,area);
  assert.equal(found.count,448);
  assert.equal(found.x,63.5);assert.equal(found.y,79.5);
  assert.deepEqual(found.area,{x:20,y:72,width:170,height:16});
  fill(50,30,28,16);
  assert.equal(togglePixels(image,area),null);
});

test('a black unpresented frame never counts as rendered text', () => {
  const area={x:0,y:0,width:10,height:10};
  const image={width:10,height:10,data:new Uint8Array(400)};
  assert.equal(hasTextOnWhite(image,area,5),false);
  image.data.fill(255);
  assert.equal(hasTextOnWhite(image,area,5),false);
  image.data.fill(0,0,40);
  assert.equal(hasTextOnWhite(image,area,5),true);
});

test('offscreen scroll geometry does not mistake a partly visible compact hero for paused', () => {
  const bounds={top:300,bottom:600,viewportHeight:360,scrollHeight:1200};
  const target=offscreenScrollTop(bounds);
  assert.equal(target,601);
  assert.ok(bounds.bottom-target<0);
  assert.ok(target<=bounds.scrollHeight-bounds.viewportHeight);
});
test('offscreen scroll can place a lower specimen below the viewport when the bottom is clamped', () => {
  const bounds={top:700,bottom:900,viewportHeight:360,scrollHeight:1000};
  const target=offscreenScrollTop(bounds);
  assert.equal(target,339);
  assert.ok(bounds.top-target>bounds.viewportHeight);
});
test('unreachable offscreen geometry fails rather than weakening the pause assertion', () => {
  assert.throws(()=>offscreenScrollTop({top:100,bottom:400,viewportHeight:360,scrollHeight:500}),/Cannot scroll/);
});
