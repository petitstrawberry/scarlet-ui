import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {chromium} from 'playwright';
import {PNG} from 'pngjs';
import {browserEnvironment, localRequest, artifactPath, changedPixels, coloredPixels, red, dark, hasTextOnWhite} from './smoke-helpers.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(process.argv[2] ?? path.join(here, '../../target/web-samples'));
const output = path.resolve(process.argv[3] ?? path.join(here, 'artifacts'));
await fs.mkdir(output, {recursive:true});
const report = {softwareGpu:true, browserSandboxRequested:true, gpuSandboxMayBeAbsent:true, checks:[], errors:[], events:[]};
const errors = report.errors;
let phase='setup';
function event(kind, detail) { report.events.push({at:new Date().toISOString(),phase,kind,detail}); }
function failure(kind, detail) { errors.push(`${kind}: ${detail}`); event(kind,detail); }
function setPhase(value) { phase=value;event('phase',value); }
async function bounded(promise, milliseconds, description) {
  let timer;
  try { return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error(`${description} timed out`)),milliseconds);})]); }
  finally { clearTimeout(timer); }
}
const MIME = {'.html':'text/html; charset=utf-8','.css':'text/css; charset=utf-8','.js':'text/javascript; charset=utf-8','.wasm':'application/wasm','.json':'application/json','.txt':'text/plain; charset=utf-8'};
let browser, context, page;
const server = http.createServer(async (request, response) => {
  try {
    const url = new URL(request.url, 'http://127.0.0.1');
    if(url.pathname === '/favicon.ico') { response.writeHead(204); response.end(); return; }
    let filename = artifactPath(root, url.pathname);
    if((await fs.stat(filename)).isDirectory()) filename = path.join(filename, 'index.html');
    // Do not allow a symlink to escape the generated artifact either.
    const real = await fs.realpath(filename);
    const relative = path.relative(root, real);
    if(relative.startsWith('..') || path.isAbsolute(relative)) throw Error('Path escapes artifact');
    response.writeHead(200, {'Content-Type':MIME[path.extname(filename)] ?? 'application/octet-stream','Cache-Control':'no-store'});
    response.end(await fs.readFile(filename));
  } catch { response.writeHead(404); response.end('Not found'); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const sleep = ms => new Promise(resolve => setTimeout(resolve,ms));

async function screenshot(name) {
  const bytes = await page.screenshot({path:path.join(output, name+'.png')});
  return PNG.sync.read(bytes);
}
async function untilPixels(name, predicate, timeout=12000) {
  const deadline=Date.now()+timeout;
  do {
    assert.equal(errors.length,0,errors.join('\n'));
    const image=await screenshot(name);
    if(predicate(image)) return image;
    await sleep(150);
  } while(Date.now()<deadline);
  throw Error(`GPU pixel assertion timed out: ${name}`);
}
async function ready(route,module) {
  setPhase(`start ${route}`);
  await page.goto(origin+route,{waitUntil:'domcontentloaded'});
  await page.waitForFunction(() => document.querySelector('#status')?.hidden || document.querySelector('#status')?.classList.contains('error'),null,{timeout:45000});
  assert.equal(await page.locator('#status').isHidden(),true,await page.locator('#message').textContent());
  // Read the public diagnostic export, not a test-only state injection.
  assert.equal(await page.evaluate(async module => (await import(`./pkg/${module}.js`)).runtime_error(),module),undefined);
}
const titleRegion={x:25,y:20,width:270,height:45};
const countRegion={x:80,y:70,width:150,height:45};
const sidebarRegion={x:0,y:28,width:190,height:225};
const toggleRegion=width=>({x:200,y:120,width:width-210,height:45});

try {
  browser=await chromium.launch({
    // Exercise the headed compositor under CI Xvfb, matching local GPU QA.
    headless:false,
    // Ubuntu's existing Chrome package policy permits sandboxed namespaces.
    // Do not use an unprofiled downloaded developer build or alter AppArmor.
    channel:'chrome',
    chromiumSandbox:true,
    env:browserEnvironment(),
    // Required for the read-only Browser.getBrowserCommandLine diagnostic.
    args:['--enable-automation','--use-angle=swiftshader','--enable-features=Vulkan','--disable-vulkan-surface','--enable-unsafe-webgpu','--disable-background-networking'],
  });
  const browserSession=await browser.newBrowserCDPSession();
  const command=await browserSession.send('Browser.getBrowserCommandLine');
  assert(!command.arguments.some(x=>['--no-sandbox','--disable-setuid-sandbox','--disable-gpu-sandbox'].includes(x)), 'Forbidden explicit sandbox-disabling flag');
  const gpuInfo=await browserSession.send('SystemInfo.getInfo');
  report.gpu={devices:gpuInfo.gpu.devices.map(({vendorString,deviceString})=>({vendorString,deviceString})),sandboxed:gpuInfo.gpu.auxAttributes?.sandboxed ?? null};
  // This internal browser diagnostic is not a network navigation. Fail closed
  // when the runner cannot provide both Linux sandbox layers; never modify its
  // sysctl/AppArmor policy or retry with a sandbox-disabling flag.
  const diagnostics=await browser.newContext();
  const sandboxPage=await diagnostics.newPage();
  try {
    await sandboxPage.goto('chrome://sandbox/', {waitUntil:'domcontentloaded'});
    const sandboxText=await sandboxPage.locator('body').innerText();
    report.rendererSandbox=sandboxText;
    await fs.writeFile(path.join(output,'sandbox.txt'), sandboxText+'\n');
    await sandboxPage.screenshot({path:path.join(output,'sandbox.png')});
    assert(/Seccomp-BPF sandbox\s+Yes/i.test(sandboxText), 'Renderer seccomp-BPF sandbox is not confirmed');
    assert(/PID namespaces\s+Yes/i.test(sandboxText), 'Renderer namespace sandbox is not confirmed');
  } finally { await diagnostics.close(); }
  context=await browser.newContext({viewport:{width:960,height:640},deviceScaleFactor:1,serviceWorkers:'block'});
  await context.route('**/*',route=>{
    if(localRequest(route.request().url(),origin)) return route.continue();
    errors.push(`Blocked non-local runtime request: ${new URL(route.request().url()).origin}`);
    return route.abort('blockedbyclient');
  });
  await context.routeWebSocket('**/*', socket => {
    errors.push('Blocked unexpected runtime WebSocket');
    socket.close();
  });
  // Observe native WebGPU loss before wgpu's own callback, without changing
  // application code or swallowing a rejection. Include destroyed probe devices.
  await context.addInitScript(() => {
    if(!globalThis.GPUAdapter) return;
    const requestDevice=GPUAdapter.prototype.requestDevice;
    let nextDevice=0;
    GPUAdapter.prototype.requestDevice=async function(...args) {
      const device=await requestDevice.apply(this,args);
      const id=++nextDevice;
      const log=value=>console.info('SCARLET_GPU_DIAGNOSTIC '+JSON.stringify({device:id,at:new Date().toISOString(),...value}));
      let explicitlyDestroyed=false;
      const destroy=device.destroy.bind(device);
      device.destroy=()=>{explicitlyDestroyed=true;destroy();};
      log({kind:'device-created',maxBufferSize:device.limits.maxBufferSize,maxTextureDimension2D:device.limits.maxTextureDimension2D});
      device.lost.then(info=>log({kind:'device-lost',reason:info.reason,message:info.message,expected:explicitlyDestroyed && info.reason==='destroyed'}));
      device.addEventListener('uncapturederror',event=>log({kind:'uncaptured-error',message:event.error.message}));
      return device;
    };
  });
  page=await context.newPage();
  page.on('pageerror',error=>failure('pageerror',error.stack??error.message));
  page.on('console',message=>{
    if(message.type()==='error') failure('console',message.text());
    if(message.type()==='info' && message.text().startsWith('SCARLET_GPU_DIAGNOSTIC ')) {
      const diagnostic=JSON.parse(message.text().slice('SCARLET_GPU_DIAGNOSTIC '.length));
      event('gpu-diagnostic',diagnostic);
      if(diagnostic.kind==='device-lost' && !diagnostic.expected && phase!=='teardown')
        failure('unexpected-device-lost',`${diagnostic.reason}: ${diagnostic.message}`);
    }
  });
  page.on('crash',()=>failure('crash','Browser page crashed'));
  setPhase('landing');
  await page.goto(origin+'/',{waitUntil:'domcontentloaded'});
  assert.equal(await page.locator('h1').textContent(),'Try ScarletUIin your browser.');
  assert.equal(await page.locator('a[href="./widget-factory/"]').count(),1);
  assert.equal(await page.locator('a[href="./counter/"]').count(),1);
  const capability=await page.evaluate(async()=>{
    if(!isSecureContext || !navigator.gpu) return {ok:false,reason:'Secure WebGPU is unavailable'};
    const adapter=await navigator.gpu.requestAdapter();
    if(!adapter) return {ok:false,reason:'requestAdapter returned null'};
    const device=await adapter.requestDevice();
    device.destroy();
    return {ok:true};
  });
  assert.equal(capability.ok,true,`Software WebGPU device required; not a skipped test: ${capability.reason}`);
  await page.locator('#check-gpu').click();
  await page.waitForFunction(()=>document.querySelector('#gpu-status').textContent.includes('adapter is available'));
  await screenshot('landing');report.checks.push('landing and real adapter/device');

  setPhase('GPU clear and mapped-buffer preflight');
  await bounded(page.evaluate(async()=>{
    const adapter=await navigator.gpu.requestAdapter();
    if(!adapter) throw Error('GPU presentation preflight has no adapter');
    const device=await adapter.requestDevice();
    device.addEventListener('uncapturederror',event=>console.error(`Preflight GPU error: ${event.error.message}`));
    const buffer=device.createBuffer({size:4096,usage:GPUBufferUsage.COPY_SRC,mappedAtCreation:true});
    new Uint8Array(buffer.getMappedRange()).fill(0);
    buffer.unmap();buffer.destroy();
    const canvas=document.createElement('canvas');
    canvas.id='gpu-presentation-probe';canvas.width=256;canvas.height=128;
    Object.assign(canvas.style,{position:'fixed',left:'0',top:'0',width:'256px',height:'128px',zIndex:'9999'});
    document.body.append(canvas);
    const context=canvas.getContext('webgpu');
    if(!context) throw Error('GPU presentation preflight has no canvas context');
    context.configure({device,format:navigator.gpu.getPreferredCanvasFormat(),alphaMode:'opaque'});
    const encoder=device.createCommandEncoder();
    const pass=encoder.beginRenderPass({colorAttachments:[{view:context.getCurrentTexture().createView(),clearValue:{r:0.125,g:0.75,b:0.25,a:1},loadOp:'clear',storeOp:'store'}]});
    pass.end();device.queue.submit([encoder.finish()]);
    await device.queue.onSubmittedWorkDone();
    globalThis.cleanupGpuProbe=()=>{context.unconfigure();device.destroy();canvas.remove();};
    await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
  }),15000,'GPU clear/buffer preflight');
  await untilPixels('gpu-clear-probe',img=>coloredPixels(img,{x:10,y:10,width:200,height:100},(r,g,b)=>g>r+50 && g>b+50).count>19000);
  await page.evaluate(()=>{cleanupGpuProbe();delete globalThis.cleanupGpuProbe;});
  report.checks.push('mapped 4096-byte buffer and visible WebGPU clear-color frame');

  await ready('/counter/','scarlet_ui_web_counter');
  const initial=await untilPixels('counter-0',img=>hasTextOnWhite(img,titleRegion,150) && hasTextOnWhite(img,countRegion,50));
  setPhase('counter click');
  await page.mouse.click(150,135);await page.mouse.move(500,500);
  const incremented=await untilPixels('counter-1',img=>hasTextOnWhite(img,countRegion,50) && changedPixels(initial,img,countRegion)>12);
  assert(hasTextOnWhite(incremented,titleRegion,150),'Counter title lost during partial repaint');
  setPhase('counter click');
  await page.mouse.click(150,135);await page.mouse.move(500,500);
  await untilPixels('counter-2',img=>hasTextOnWhite(img,countRegion,50) && changedPixels(incremented,img,countRegion)>12);
  report.checks.push('two Counter clicks change rendered count glyphs');

  await ready('/widget-factory/','scarlet_ui_widget_factory');
  await untilPixels('factory-overview',img=>hasTextOnWhite(img,sidebarRegion,200));
  await page.mouse.click(70,88);await page.mouse.move(900,600);
  let controls=await untilPixels('factory-controls',img=>coloredPixels(img,toggleRegion(960),red).count>60);
  const toggle=coloredPixels(controls,toggleRegion(960),red);
  await page.mouse.click(toggle.x,toggle.y);await page.mouse.move(900,600);
  const off=await untilPixels('factory-toggle-off',img=>coloredPixels(img,toggleRegion(960),red).count<10);
  await page.mouse.click(95,14);
  await untilPixels('factory-menu',img=>changedPixels(off,img,{x:65,y:34,width:170,height:32})>150);
  // Vertical path stays under View rather than traversing the adjacent Tools root.
  await page.mouse.move(95,48);await page.mouse.click(95,48);await page.mouse.move(900,600);
  await untilPixels('factory-menu-action',img=>coloredPixels(img,toggleRegion(960),red).count>60);
  report.checks.push('Factory navigation, rendered toggle and shared menu action');
  await page.setViewportSize({width:800,height:600});
  controls=await untilPixels('factory-resized',img=>img.width===800 && coloredPixels(img,toggleRegion(800),red).count>60);
  const resizedToggle=coloredPixels(controls,toggleRegion(800),red);
  await page.mouse.click(resizedToggle.x,resizedToggle.y);await page.mouse.move(750,550);
  const final=await untilPixels('factory-resized-toggle',img=>coloredPixels(img,toggleRegion(800),red).count<10);
  assert(hasTextOnWhite(final,sidebarRegion,200),'Sidebar disappeared after partial repaint');
  report.checks.push('resize preserves state and correct control hit mapping');
  assert.equal(await page.evaluate(async()=> (await import('./pkg/scarlet_ui_widget_factory.js')).runtime_error()),undefined);
  assert.equal(errors.length,0,errors.join('\n'));
  report.passed=true;
} catch(error) {
  report.passed=false;report.failure=error.stack??String(error);event('failure',report.failure);process.exitCode=1;
  if(page) {try {await screenshot('failure');} catch {}}
  console.error(report.failure);
} finally {
  setPhase('teardown');
  await context?.close().catch(()=>{});await browser?.close().catch(()=>{});
  await fs.writeFile(path.join(output,'report.json'),JSON.stringify(report,null,2)+'\n');
  await new Promise(resolve=>server.close(resolve));
}
