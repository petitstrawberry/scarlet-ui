import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {chromium} from 'playwright';
import {PNG} from 'pngjs';
import {browserEnvironment, localRequest, artifactPath, changedPixels, coloredPixels, red, dark, hasTextOnWhite, offscreenScrollTop} from './smoke-helpers.mjs';

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
const MIME = {'.html':'text/html; charset=utf-8','.css':'text/css; charset=utf-8','.js':'text/javascript; charset=utf-8','.wasm':'application/wasm','.json':'application/json','.txt':'text/plain; charset=utf-8','.svg':'image/svg+xml','.png':'image/png','.webp':'image/webp','.woff2':'font/woff2'};
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
    args:['--enable-automation','--use-angle=swiftshader','--use-vulkan=swiftshader','--use-webgpu-adapter=swiftshader','--enable-features=Vulkan','--disable-vulkan-surface','--enable-unsafe-webgpu','--disable-background-networking'],
  });
  report.browserVersion=browser.version();
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
      const info=this.info;
      log({kind:'device-created',adapter:info ? {vendor:info.vendor,architecture:info.architecture,device:info.device,description:info.description,isFallbackAdapter:info.isFallbackAdapter} : null,maxBufferSize:device.limits.maxBufferSize,maxTextureDimension2D:device.limits.maxTextureDimension2D});
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
  {
  setPhase('toolkit homepage and live specimen');
  await page.goto(origin+'/',{waitUntil:'domcontentloaded'});
  assert.equal((await page.locator('h1').innerText()).trim(),'ScarletUI');
  assert.equal((await page.locator('.platform-list').textContent()).trim(),'Linux · macOS · Windows · Scarlet · Web');
  const tryLink=page.getByRole('link',{name:'Try in your browser',exact:true});
  assert.equal(await tryLink.getAttribute('href'),'/try/');
  assert.equal(await page.locator('#check-gpu').count(),0);
  const specimen=page.locator('.hero [data-scarlet-specimen]');
  const canvas=specimen.locator('canvas');
  await canvas.scrollIntoViewIfNeeded();
  await page.waitForFunction(()=> {
    const host=document.querySelector('.hero [data-scarlet-specimen]');
    return host?.dataset.state==='live' && host.dataset.rendered==='true';
  },null,{timeout:45000});
  async function specimenFrame(name) {
    return PNG.sync.read(await canvas.screenshot({path:path.join(output,name+'.png')}));
  }
  async function specimenPixels(name,predicate) {
    const deadline=Date.now()+12000;
    do {
      assert.equal(errors.length,0,errors.join('\n'));
      const frame=await specimenFrame(name);
      if(predicate(frame)) return frame;
      await sleep(150);
    } while(Date.now()<deadline);
    throw Error('Specimen pixel assertion timed out: '+name);
  }
  const whole=frame=>({x:4,y:4,width:frame.width-8,height:frame.height-8});
  const initial=await specimenPixels('specimen-initial',frame=>hasTextOnWhite(frame,whole(frame),100));
  // The paper is a real SGFX surface. Its overlay forwards pointer input only;
  // use ordinary world-coordinate mouse/touch input, never forced canvas clicks.
  const width=initial.width;
  const paper={x:14,y:50,width:width-28,height:180};
  const blue=(r,g,b)=>b>120&&b-r>35&&b-g>15;
  const green=(r,g,b)=>g>95&&g>r*1.3&&g>b*1.1;
  const paperInk=frame=>coloredPixels(frame,paper,(r,g,b)=>Math.min(r,g,b)<180).count;
  async function specimenClick(x,y) {
    const bounds=await canvas.boundingBox();
    await page.mouse.click(bounds.x+x,bounds.y+y);
    await page.mouse.move(0,0);
  }
  async function drawStroke(y) {
    const bounds=await canvas.boundingBox();
    await page.mouse.move(bounds.x+45,bounds.y+y);
    await page.mouse.down();
    try { await page.mouse.move(bounds.x+width-70,bounds.y+y,{steps:12}); }
    finally { await page.mouse.up(); }
    await page.mouse.move(0,0);
  }
  assert.ok(paperInk(initial)>100,'Initial SGFX drawing must render');
  await drawStroke(72);
  await specimenPixels('sketch-mouse-stroke',frame=>changedPixels(initial,frame,paper)>100);
  await specimenClick(width-95,25);
  await specimenPixels('sketch-undo',frame=>changedPixels(initial,frame,paper)===0);
  await specimenClick(width-40,25);
  const cleared=await specimenPixels('sketch-clear',frame=>paperInk(frame)<10);
  // Clear is intentionally not undoable: it discards the drawing, not a stroke.
  await specimenClick(width-95,25);
  await specimenPixels('sketch-clear-not-undoable',frame=>changedPixels(cleared,frame,paper)===0);
  await specimenClick(88,255); // blue palette, outside the input overlay
  await specimenClick(161,255); // narrow stock Slider setting
  await drawStroke(100);
  const thin=await specimenPixels('sketch-blue-thin',frame=>coloredPixels(frame,paper,blue).count>60);
  const thinBlue=coloredPixels(thin,paper,blue).count;
  await specimenClick(width-15,255); // wide stock Slider setting
  await drawStroke(145);
  await specimenPixels('sketch-blue-thick',frame=>coloredPixels(frame,paper,blue).count>thinBlue*2);
  await specimenClick(width-95,25);
  await specimenPixels('sketch-undo-thick',frame=>changedPixels(thin,frame,paper)===0);
  await specimenClick(120,255); // green
  // Simulated touch on this same isolated localhost context, not real-device QA.
  const touch=await context.newCDPSession(page);
  let touchActive=false;
  try {
    await touch.send('Emulation.setTouchEmulationEnabled',{enabled:true,maxTouchPoints:1});
    const bounds=await canvas.boundingBox();
    const point=x=>({x:bounds.x+x,y:bounds.y+185,id:1,radiusX:2,radiusY:2,force:1});
    await touch.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[point(50)]});
    touchActive=true;
    for(let index=1;index<=8;index++)
      await touch.send('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[point(50+(width-125)*index/8)]});
    await touch.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
    touchActive=false;
  } finally {
    try {
      // CDP rejects cancel after a completed end: there is no active contact.
      if(touchActive)
        await touch.send('Input.dispatchTouchEvent',{type:'touchCancel',touchPoints:[]});
    } finally {
      try { await touch.send('Emulation.setTouchEmulationEnabled',{enabled:false}); }
      finally { await touch.detach(); }
    }
  }
  const adjusted=await specimenPixels('sketch-simulated-touch',frame=>
    coloredPixels(frame,paper,green).count>100&&coloredPixels(frame,paper,blue).count>=thinBlue);
  await canvas.focus();
  await page.keyboard.press('Tab');
  assert.equal(await canvas.evaluate(element=>document.activeElement===element),false,'Tab must leave the embedded canvas');
  report.checks.push('real SGFX mouse stroke, palette/width, Undo/Clear and CDP-simulated touch drawing');
  await screenshot('homepage');
  await page.setViewportSize({width:375,height:640});
  await canvas.scrollIntoViewIfNeeded();
  await specimenPixels('sketch-normalized-resize',frame=> {
    if(frame.width>=width) return false;
    const resizedPaper={x:14,y:50,width:frame.width-28,height:180};
    for(const color of [blue,green]) {
      const before=coloredPixels(adjusted,paper,color);
      const after=coloredPixels(frame,resizedPaper,color);
      if(after.count<50||Math.abs((before.x-12)/(width-24)-(after.x-12)/(frame.width-24))>0.03||
         Math.abs(before.y-after.y)>2) return false;
    }
    return true;
  });
  // Wheel input over the embedded canvas must remain owned by the document.
  await page.setViewportSize({width:500,height:360});
  await canvas.scrollIntoViewIfNeeded();
  const box=await canvas.boundingBox();
  await page.mouse.move(box.x+box.width/2,Math.min(340,Math.max(20,box.y+box.height/2)));
  const beforeScroll=await page.evaluate(()=>scrollY);
  await page.mouse.wheel(0,beforeScroll>0 ? -600 : 600);
  await page.waitForFunction(before=>Math.abs(scrollY-before)>0,beforeScroll);
  const heroBounds=await canvas.evaluate(element=> {
    const box=element.getBoundingClientRect();
    return {top:box.top+scrollY,bottom:box.bottom+scrollY,viewportHeight:innerHeight,scrollHeight:document.documentElement.scrollHeight};
  });
  await page.evaluate(target=>scrollTo(0,target),offscreenScrollTop(heroBounds));
  // Establish actual non-intersection before expecting the observer to pause RAF.
  await page.waitForFunction(()=> {
    const box=document.querySelector('.hero [data-scarlet-specimen]').getBoundingClientRect();
    return box.bottom<0 || box.top>innerHeight;
  });
  await page.waitForFunction(()=>document.querySelector('.hero [data-scarlet-specimen]')?.dataset.state==='paused');
  await page.setViewportSize({width:960,height:640});
  await canvas.scrollIntoViewIfNeeded();
  await page.waitForFunction(()=>document.querySelector('.hero [data-scarlet-specimen]')?.dataset.state==='live');
  await page.mouse.move(0,0);
  await specimenPixels('specimen-resumed',frame=>frame.width===adjusted.width&&
    changedPixels(adjusted,frame,whole(frame))===0);
  assert.equal(await page.evaluate(async()=>
    (await import('/widget-factory/pkg/scarlet_ui_widget_factory.js')).runtime_error()),undefined);
  report.checks.push('Sketch normalized resize, document wheel and pause/resume drawing retention');
  await page.getByRole('combobox',{name:'Select theme'}).selectOption('dark');
  await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');
  await canvas.scrollIntoViewIfNeeded();
  await specimenPixels('specimen-dark',frame=>hasTextOnWhite(frame,whole(frame),100));
  await screenshot('homepage-dark');
  await page.getByRole('combobox',{name:'Select theme'}).selectOption('light');
  await page.waitForFunction(()=>document.documentElement.dataset.theme==='light');
  report.checks.push('Starlight dark theme with readable native canvas surface');
  // The lower example is a different compiled Wasm module with its own state.
  const example=page.locator('[data-scarlet-specimen][data-start="start_example"]');
  const counterCanvas=example.locator('#counter-example');
  await counterCanvas.scrollIntoViewIfNeeded();
  await page.waitForFunction(()=> {
    const host=document.querySelector('[data-start="start_example"]');
    return host?.dataset.state==='live' && host.dataset.rendered==='true';
  },null,{timeout:45000});
  async function examplePixels(name,predicate) {
    const deadline=Date.now()+12000;
    do {
      assert.equal(errors.length,0,errors.join('\n'));
      const frame=PNG.sync.read(await counterCanvas.screenshot({path:path.join(output,name+'.png')}));
      if(predicate(frame)) return frame;
      await sleep(150);
    } while(Date.now()<deadline);
    throw Error('Embedded counter pixel assertion timed out: '+name);
  }
  const counterInitial=await examplePixels('embedded-counter-initial',frame=>hasTextOnWhite(frame,whole(frame),50));
  let increment;
  for(let y=20;y<counterInitial.height-12&&!increment;y++) {
    let start=-1;
    for(let x=8;x<counterInitial.width-8;x++) {
      const offset=(y*counterInitial.width+x)*4;
      const [r,g,b]=counterInitial.data.subarray(offset,offset+3);
      const match=Math.min(r,g,b)>=210&&Math.max(r,g,b)<=250&&Math.max(r,g,b)-Math.min(r,g,b)<12;
      if(match&&start<0) start=x;
      if((!match||x===counterInitial.width-9)&&start>=0) {
        if(x-start>16&&x-start<180) {increment={x:(start+x)/2,y:y+8};break;}
        start=-1;
      }
    }
  }
  assert.ok(increment,'Rendered embedded +1 button must exist');
  const countArea={x:8,y:8,width:counterInitial.width-16,height:increment.y-24};
  assert.ok(countArea.height>10,'Counter glyph region must be above its button');
  assert.ok(hasTextOnWhite(counterInitial,countArea,20),'The initial numeric glyph must render above +1');
  await counterCanvas.click({position:increment});
  await page.mouse.move(0,0);
  const counterClicked=await examplePixels('embedded-counter-clicked',frame=>
    changedPixels(counterInitial,frame,countArea)>12&&hasTextOnWhite(frame,countArea,20));
  assert.equal(await page.evaluate(async()=>
    (await import('/counter/pkg/scarlet_ui_web_counter.js')).runtime_error()),undefined);
  await canvas.scrollIntoViewIfNeeded();
  await page.waitForFunction(()=>document.querySelector('.hero [data-scarlet-specimen]')?.dataset.state==='live');
  await page.mouse.move(0,0);
  await specimenPixels('hero-after-counter',frame=>frame.width===adjusted.width&&changedPixels(adjusted,frame,whole(frame))===0);
  await counterCanvas.scrollIntoViewIfNeeded();
  await page.waitForFunction(()=>document.querySelector('[data-start="start_example"]')?.dataset.state==='live');
  await page.mouse.move(0,0);
  await examplePixels('embedded-counter-retained',frame=>changedPixels(counterClicked,frame,whole(frame))===0);
  report.checks.push('compiled lower counter increments independently and both embedded modules retain state');
  // Measure CSS boxes, not screenshot crop widths: scrollbars can change capture
  // dimensions without changing the document's real layout viewport.
  assert.equal((await page.locator('.counter-preview .preview-label').textContent()).trim(),'Preview');
  for(const viewportWidth of [500,390,375]) {
    await page.setViewportSize({width:viewportWidth,height:640});
    await page.locator('.counter-example').scrollIntoViewIfNeeded();
    const layout=await page.evaluate(()=> {
      const box=selector=> {
        const element=document.querySelector(selector);
        if(!element) throw Error('Missing narrow-layout element: '+selector);
        const rect=element.getBoundingClientRect();
        const style=getComputedStyle(element);
        const insetLeft=parseFloat(style.paddingLeft)+parseFloat(style.borderLeftWidth);
        const insetRight=parseFloat(style.paddingRight)+parseFloat(style.borderRightWidth);
        return {left:rect.left,right:rect.right,contentLeft:rect.left+insetLeft,contentRight:rect.right-insetRight};
      };
      return {
        viewport:document.documentElement.clientWidth,
        scrollWidth:document.documentElement.scrollWidth,
        section:box('.counter-example'),
        grid:box('.counter-example-layout'),
        code:box('.counter-code'),
        frame:box('.counter-code .expressive-code .frame'),
        pre:box('.counter-code pre'),
      };
    });
    const tolerance=1;
    assert.ok(layout.scrollWidth<=layout.viewport+tolerance,'No horizontal document overflow at '+viewportWidth);
    for(const [name,box] of Object.entries(layout).filter(([,value])=>typeof value==='object')) {
      assert.ok(box.left>=-tolerance&&box.right<=layout.viewport+tolerance,
        name+' must remain inside the actual client viewport at '+viewportWidth);
    }
    for(const [child,parent] of [['grid','section'],['code','grid'],['frame','code']]) {
      assert.ok(Math.abs(layout[child].left-layout[parent].contentLeft)<=tolerance,
        child+' must align with its parent content left edge at '+viewportWidth);
      assert.ok(Math.abs(layout[child].right-layout[parent].contentRight)<=tolerance,
        child+' must align with its parent content right edge at '+viewportWidth);
    }
    assert.ok(layout.pre.left>=layout.frame.left-tolerance&&layout.pre.right<=layout.frame.right+tolerance,
      'Code pre must remain inside its frame at '+viewportWidth);
    assert.ok(layout.section.left>0&&layout.viewport-layout.section.right>0,
      'Both narrow page gutters must remain visible at '+viewportWidth);
    assert.ok(Math.abs(layout.section.left-(layout.viewport-layout.section.right))<=tolerance,
      'Narrow page gutters must be equal on both sides at '+viewportWidth);
    event('narrow-code-layout',{requestedWidth:viewportWidth,...layout});
  }
  await page.setViewportSize({width:960,height:640});
  report.checks.push('narrow code frame respects actual client viewport and section content edges at 500/390/375px');

  await tryLink.click();
  await page.waitForURL(origin+'/try/');
  setPhase('browser samples');
  assert.equal(await page.locator('a[href="/widget-factory/"]').count(),1);
  assert.equal(await page.locator('a[href="/counter/"]').count(),1);
  }
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
  await screenshot('try-samples');report.checks.push('toolkit homepage, browser samples and real adapter/device');

  setPhase('GPU clear and mapped-buffer preflight');
  await bounded(page.evaluate(async()=>{
    const step=name=>console.info('SCARLET_GPU_DIAGNOSTIC '+JSON.stringify({kind:'preflight-step',step:name,at:new Date().toISOString()}));
    step('request adapter');
    const adapter=await navigator.gpu.requestAdapter();
    if(!adapter) throw Error('GPU presentation preflight has no adapter');
    step('request device');
    const device=await adapter.requestDevice();
    step('create mapped 4096 buffer');
    device.addEventListener('uncapturederror',event=>console.error(`Preflight GPU error: ${event.error.message}`));
    const buffer=device.createBuffer({size:4096,usage:GPUBufferUsage.COPY_SRC,mappedAtCreation:true});
    step('get mapped range');
    new Uint8Array(buffer.getMappedRange()).fill(0);
    buffer.unmap();buffer.destroy();
    step('mapped buffer initialized and destroyed');
    const canvas=document.createElement('canvas');
    canvas.id='gpu-presentation-probe';canvas.width=256;canvas.height=128;
    Object.assign(canvas.style,{position:'fixed',left:'0',top:'0',width:'256px',height:'128px',zIndex:'9999'});
    document.body.append(canvas);
    const context=canvas.getContext('webgpu');
    if(!context) throw Error('GPU presentation preflight has no canvas context');
    step('configure canvas');
    context.configure({device,format:navigator.gpu.getPreferredCanvasFormat(),alphaMode:'opaque'});
    step('create command encoder');
    const encoder=device.createCommandEncoder();
    step('get current texture and begin render pass');
    const pass=encoder.beginRenderPass({colorAttachments:[{view:context.getCurrentTexture().createView(),clearValue:{r:0.125,g:0.75,b:0.25,a:1},loadOp:'clear',storeOp:'store'}]});
    pass.end();
    step('submit clear pass');
    device.queue.submit([encoder.finish()]);
    step('wait submitted work');
    await device.queue.onSubmittedWorkDone();
    step('submitted work complete');
    globalThis.cleanupGpuProbe=()=>{context.unconfigure();device.destroy();canvas.remove();};
    await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
    step('two animation frames complete');
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
  setPhase('Factory real touch scrolling and gesture recovery');
  const factoryTouch=await context.newCDPSession(page);
  let factoryContact=false;
  async function factoryGesture(points,cancel=false) {
    const point=([x,y])=>({x,y,id:1,radiusX:3,radiusY:3,force:1});
    try {
      await factoryTouch.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[point(points[0])]});
      factoryContact=true;
      for(const position of points.slice(1)) {
        await factoryTouch.send('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[point(position)]});
        await sleep(20);
      }
      await factoryTouch.send('Input.dispatchTouchEvent',{type:cancel?'touchCancel':'touchEnd',touchPoints:[]});
      factoryContact=false;
    } finally {
      if(factoryContact) {
        await factoryTouch.send('Input.dispatchTouchEvent',{type:'touchCancel',touchPoints:[]});
        factoryContact=false;
      }
    }
  }
  const drag=(x,from,to)=>Array.from({length:13},(_,index)=>[x,from+(to-from)*index/12]);
  try {
    await factoryTouch.send('Emulation.setTouchEmulationEnabled',{enabled:true,maxTouchPoints:1});
    await factoryGesture([[70,168]]);
    const displayLabels={x:210,y:60,width:140,height:450};
    const display=await untilPixels('factory-touch-display',img=>
      hasTextOnWhite(img,sidebarRegion,200)&&changedPixels(final,img,displayLabels)>150);
    const documentBefore=await page.evaluate(()=>({x:scrollX,y:scrollY}));
    // The label column is inside the outer ScrollView but outside its nested
    // scroll demos. A finger drag must move rendered UI, not the HTML document.
    await factoryGesture(drag(260,480,160));
    const scrolled=await untilPixels('factory-touch-scroll',img=>
      changedPixels(display,img,displayLabels)>200&&hasTextOnWhite(img,sidebarRegion,200));
    assert.deepEqual(await page.evaluate(()=>({x:scrollX,y:scrollY})),documentBefore,
      'Factory touch scrolling must not scroll the host document');
    assert.equal(changedPixels(display,scrolled,{x:0,y:200,width:185,height:300}),0,
      'Touch scrolling must not move or erase the fixed sidebar');
    // Cancellation releases gesture ownership, allowing another independent drag.
    await factoryGesture(drag(260,240,300),true);
    const afterCancel=await screenshot('factory-touch-cancel');
    await factoryGesture(drag(260,220,440));
    await untilPixels('factory-touch-scroll-again',img=>changedPixels(afterCancel,img,displayLabels)>150);
    assert.deepEqual(await page.evaluate(()=>({x:scrollX,y:scrollY})),documentBefore);
    await factoryGesture([[70,88]]);
    await untilPixels('factory-touch-controls',img=>changedPixels(final,img,{x:200,y:40,width:590,height:200})<100);
    // Subsequent taps and horizontal Slider gestures must still reach controls.
    await factoryGesture([[resizedToggle.x,resizedToggle.y]]);
    await untilPixels('factory-touch-toggle-on',img=>coloredPixels(img,toggleRegion(800),red).count>60);
    await factoryGesture([[resizedToggle.x,resizedToggle.y]]);
    const touchOff=await untilPixels('factory-touch-toggle-off',img=>coloredPixels(img,toggleRegion(800),red).count<10);
    const sliderRegion={x:390,y:165,width:385,height:30};
    await factoryGesture(Array.from({length:13},(_,index)=>[500+240*index/12,180]));
    await untilPixels('factory-touch-slider',img=>changedPixels(touchOff,img,sliderRegion)>100);
    report.checks.push('CDP-simulated Factory touch scrolls internal content, survives cancellation/repeated gestures, and preserves tap/slider input');
  } finally {
    try {
      if(factoryContact)
        await factoryTouch.send('Input.dispatchTouchEvent',{type:'touchCancel',touchPoints:[]});
    } finally {
      try { await factoryTouch.send('Emulation.setTouchEmulationEnabled',{enabled:false}); }
      finally { await factoryTouch.detach(); }
    }
  }


  setPhase('Factory browser-owned context menu');
  await page.mouse.move(740,520);
  const beforeContext=await screenshot('factory-before-contextmenu');
  const contextMenuObserved=page.evaluate(()=>new Promise(resolve=>{
    window.addEventListener('contextmenu',event=>resolve({prevented:event.defaultPrevented}),{once:true});
  }));
  await page.mouse.click(740,520,{button:'right'});
  const contextMenu=await bounded(contextMenuObserved,5000,'Browser contextmenu event');
  assert.equal(contextMenu.prevented,false,'Factory must not suppress the browser context menu');
  const afterContext=await screenshot('factory-no-app-contextmenu');
  assert.equal(changedPixels(beforeContext,afterContext,{x:200,y:300,width:590,height:290}),0,
    'Right-click must not synthesize an application menu popup');
  await page.keyboard.press('Escape');
  report.checks.push('toolbar menus retained while right-click remains browser-owned without an application fallback');

  setPhase('Factory responsive page matrix');
  // Geometry comes from the real NavigationView layout: bottom52 below550px,
  // sidebar190 otherwise; demo rows stack when content width is below560px.
  for(const viewportWidth of [375,390,562,960]) {
    await page.setViewportSize({width:viewportWidth,height:640});
    const sidebar=viewportWidth>=550;
    const pageX=sidebar?190:0;
    const stacked=viewportWidth-pageX<560;
    const padding=stacked?16:24;
    const heading={x:pageX+padding,y:44,width:Math.min(260,viewportWidth-pageX-padding*2),height:42};
    const content={x:pageX+padding,y:44,width:viewportWidth-pageX-padding*2,height:500};
    async function navigateFactory(index) {
      await page.mouse.click(sidebar?70:(index+0.5)*viewportWidth/5,sidebar?48+40*index:620);
      await page.mouse.move(viewportWidth-2,30);
    }
    // Reset through the existing toolbar action, not through injected app state.
    await page.mouse.click(35,14);
    await page.mouse.move(35,48);await page.mouse.click(35,48);
    // Reset is disabled when already at defaults, so selecting it correctly
    // leaves the dropdown open. Dismiss before sidebar clicks share its bounds.
    await page.keyboard.press('Escape');
    await page.mouse.move(viewportWidth-2,30);
    for(const [index,name] of ['overview','controls','inputs','display','sgfx'].entries()) {
      const before=await screenshot('factory-'+viewportWidth+'-before-'+name);
      await navigateFactory(index);
      const frame=await untilPixels('factory-'+viewportWidth+'-'+name,img=>
        img.width===viewportWidth&&hasTextOnWhite(img,heading,70)&&changedPixels(before,img,heading)>20);
      assert.ok(hasTextOnWhite(frame,{...content,height:78},100),'Actual '+name+' heading/content must render at '+viewportWidth);
      assert.deepEqual(await page.evaluate(()=>({x:scrollX,y:scrollY})),{x:0,y:0},
        'Factory must remain a viewport-filling canvas');
      const size=await page.locator('canvas').evaluate(canvas=>({
        css:canvas.getBoundingClientRect().width,backing:canvas.width,client:document.documentElement.clientWidth,
        overflow:document.documentElement.scrollWidth,
      }));
      assert.equal(size.css,size.client);
      assert.equal(size.backing,viewportWidth);
      assert.ok(size.overflow<=size.client,'No horizontal browser overflow at '+viewportWidth);
      if(name==='controls') {
        const toggleArea={x:pageX+padding,y:stacked?164:128,width:viewportWidth-pageX-padding*2,height:40};
        const on=coloredPixels(frame,toggleArea,red);
        assert.ok(on.count>60,'Responsive Toggle must be visible at '+viewportWidth);
        await page.mouse.click(on.x,on.y);await page.mouse.move(viewportWidth-2,30);
        await untilPixels('factory-'+viewportWidth+'-toggle-off',img=>coloredPixels(img,toggleArea,red).count<10);
        await page.mouse.click(on.x,on.y);await page.mouse.move(viewportWidth-2,30);
        await untilPixels('factory-'+viewportWidth+'-toggle-on',img=>coloredPixels(img,toggleArea,red).count>60);
      }
      if(name==='inputs') {
        const selectX=pageX+padding+(stacked?0:168);
        const selectY=28+(stacked?146:110);
        await page.mouse.click(selectX+80,selectY+14);
        const popupArea={x:selectX,y:selectY,width:Math.min(230,viewportWidth-selectX),height:145};
        await untilPixels('factory-'+viewportWidth+'-select',img=>changedPixels(frame,img,popupArea)>500);
        await page.keyboard.press('Escape');
      }
      if(name==='sgfx') {
        // The retained 3D canvas must animate, not merely render a text heading.
        await untilPixels('factory-'+viewportWidth+'-sgfx-animation',img=>changedPixels(frame,img,content)>100);
      }
    }
  }
  report.checks.push('all five GPU-rendered Factory pages and controls reachable at 375/390/562/960px; Select and animated SGFX remain usable');
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
