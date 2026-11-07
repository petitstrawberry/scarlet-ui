import test from 'node:test';
import assert from 'node:assert/strict';
import {browserEnvironment, localRequest, artifactPath, changedPixels, coloredPixels, red, hasTextOnWhite, offscreenScrollTop, togglePixels, inputsGeometry} from './smoke-helpers.mjs';

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

test('Inputs geometry keeps the CI TextView intact when text crosses the old probe', async () => {
  const {PNG}=await import('pngjs');
  const {readFile}=await import('node:fs/promises');
  const frame=PNG.sync.read(await readFile(new URL('./fixtures/factory-375-inputs.png',import.meta.url)));
  assert.deepEqual(inputsGeometry(frame,16,92),{
    field:{x:16,y:132,width:320,height:38},
    select:{x:16,y:209,width:320,height:38},
    view:{x:16,y:347,width:320,height:202},
  });
});

function inputFixture(x=16,top=92) {
  const frame={width:x+350,height:top+520,data:new Uint8Array((x+350)*(top+520)*4).fill(255)};
  const fill=(left,y,width,height,color=246)=>{
    for(let row=y;row<y+height;row++) for(let col=left;col<left+width;col++)
      frame.data.set([color,color,color,255],(row*frame.width+col)*4);
  };
  fill(x,top+20,320,32);fill(x,top+90,260,32);fill(x,top+160,320,200);
  return {frame,fill};
}

test('Inputs geometry tolerates dense text, icons and any single obstructed column', () => {
  for(const x of [16,382]) for(const top of [92,139]) {
    const {frame,fill}=inputFixture(x,top);
    const expected=inputsGeometry(frame,x,top);
    // Dense text rows obscure 60% of the shared span, including x+200.
    for(let y=top+185;y<top+290;y+=18) {
      for(let col=x+8;col<x+252;col+=5) fill(col,y,3,8,40);
    }
    fill(x+235,top+99,8,10,80); // Select icon.
    assert.deepEqual(inputsGeometry(frame,x,top),expected);
    for(let col=x+8;col<x+252;col++) {
      const copy={...frame,data:frame.data.slice()};
      for(let y=top+160;y<top+360;y++) copy.data.set([20,20,20,255],(y*frame.width+col)*4);
      assert.deepEqual(inputsGeometry(copy,x,top),expected);
    }
  }
});

test('Inputs geometry rejects missing, extra and unrendered surfaces', () => {
  const {frame,fill}=inputFixture();
  fill(16,92+390,260,28);
  assert.throws(()=>inputsGeometry(frame,16,92),/Inputs must show/);
  fill(16,92+390,260,28,255);
  fill(16,92+90,260,32,255);
  assert.throws(()=>inputsGeometry(frame,16,92),/Inputs must show/);
  frame.data.fill(255);
  assert.throws(()=>inputsGeometry(frame,16,92),/Inputs must show/);
  frame.data.fill(0);
  assert.throws(()=>inputsGeometry(frame,16,92),/Inputs must show/);
});
