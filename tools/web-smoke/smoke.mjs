import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {chromium} from 'playwright';
import {PNG} from 'pngjs';
import {browserEnvironment, localRequest, artifactPath, changedPixels, coloredPixels, red, dark} from './smoke-helpers.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(process.argv[2] ?? path.join(here, '../../target/web-samples'));
const output = path.resolve(process.argv[3] ?? path.join(here, 'artifacts'));
await fs.mkdir(output, {recursive:true});
const report = {softwareGpu:true, browserSandboxRequested:true, gpuSandboxMayBeAbsent:true, checks:[], errors:[]};
const errors = report.errors;
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
    headless:true,
    channel:'chromium',
    chromiumSandbox:true,
    env:browserEnvironment(),
    args:['--use-angle=swiftshader','--enable-features=Vulkan','--disable-vulkan-surface','--enable-unsafe-webgpu','--disable-background-networking'],
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
  page=await context.newPage();
  page.on('pageerror',error=>errors.push(`pageerror: ${error.message}`));
  page.on('console',message=>{if(message.type()==='error') errors.push(`console: ${message.text()}`);});
  page.on('crash',()=>errors.push('Browser page crashed'));
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

  await ready('/counter/','scarlet_ui_web_counter');
  const initial=await untilPixels('counter-0',img=>coloredPixels(img,titleRegion,dark).count>150 && coloredPixels(img,countRegion,dark).count>50);
  await page.mouse.click(150,135);await page.mouse.move(500,500);
  const incremented=await untilPixels('counter-1',img=>changedPixels(initial,img,countRegion)>12);
  assert(coloredPixels(incremented,titleRegion,dark).count>150,'Counter title lost during partial repaint');
  await page.mouse.click(150,135);await page.mouse.move(500,500);
  await untilPixels('counter-2',img=>changedPixels(incremented,img,countRegion)>12);
  report.checks.push('two Counter clicks change rendered count glyphs');

  await ready('/widget-factory/','scarlet_ui_widget_factory');
  await untilPixels('factory-overview',img=>coloredPixels(img,sidebarRegion,dark).count>200);
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
  assert(coloredPixels(final,sidebarRegion,dark).count>200,'Sidebar disappeared after partial repaint');
  report.checks.push('resize preserves state and correct control hit mapping');
  assert.equal(await page.evaluate(async()=> (await import('./pkg/scarlet_ui_widget_factory.js')).runtime_error()),undefined);
  assert.equal(errors.length,0,errors.join('\n'));
  report.passed=true;
} catch(error) {
  report.passed=false;report.failure=error.stack??String(error);process.exitCode=1;
  if(page) {try {await screenshot('failure');} catch {}}
  console.error(report.failure);
} finally {
  await fs.writeFile(path.join(output,'report.json'),JSON.stringify(report,null,2)+'\n');
  await context?.close().catch(()=>{});await browser?.close().catch(()=>{});
  await new Promise(resolve=>server.close(resolve));
}
