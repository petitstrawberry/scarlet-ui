import test from 'node:test';
import assert from 'node:assert/strict';
import {browserEnvironment, localRequest, artifactPath, changedPixels, coloredPixels, red, hasTextOnWhite} from './smoke-helpers.mjs';

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

test('a black unpresented frame never counts as rendered text', () => {
  const area={x:0,y:0,width:10,height:10};
  const image={width:10,height:10,data:new Uint8Array(400)};
  assert.equal(hasTextOnWhite(image,area,5),false);
  image.data.fill(255);
  assert.equal(hasTextOnWhite(image,area,5),false);
  image.data.fill(0,0,40);
  assert.equal(hasTextOnWhite(image,area,5),true);
});
