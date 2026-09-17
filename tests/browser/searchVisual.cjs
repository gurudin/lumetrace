const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require(process.env.LUMETRACE_PLAYWRIGHT_PATH || 'playwright');
const fixtureDir = process.env.LUMETRACE_TEST_VISION_DIR;
assert(fixtureDir, 'Only an explicitly supplied synthetic fixture directory is allowed');
const names = ['document.png','scan.pdf','mixed.pdf','embedded.pdf','rotated.pdf'];
const payload = names.map(name => JSON.parse(fs.readFileSync(path.join(fixtureDir,`${name}.search.json`))));
const base = process.env.LUMETRACE_TEST_ORIGIN || 'http://127.0.0.1:1432';
(async () => {
  const browser = await chromium.launch({channel:'chrome',headless:true});
  try {
    for (const theme of ['light','dark']) for (const language of ['en','zh-CN']) for (const width of [920,1280]) {
      const page = await browser.newPage({viewport:{width,height:width===920?640:800},colorScheme:theme});
      page.setDefaultTimeout(8000);
      const errors=[]; let imageReads=0, failPdf=false;
      page.on('pageerror', error=>errors.push(error.message));
      await page.route('**/*', async route => {
        const url=new URL(route.request().url());
        if(url.origin!==base) return route.abort();
        if(url.pathname==='/__fixtures/payload') return route.fulfill({json:payload});
        if(url.pathname.startsWith('/__fixtures/')) {
          const name=decodeURIComponent(url.pathname.slice('/__fixtures/'.length));
          if(!names.includes(name)) return route.fulfill({status:404,body:'Synthetic missing preview'});
          if(name==='document.png') imageReads++;
          if(failPdf && name.endsWith('.pdf')) return route.fulfill({status:503,body:'Synthetic failure'});
          return route.fulfill({body:fs.readFileSync(path.join(fixtureDir,name)),contentType:name.endsWith('.pdf')?'application/pdf':'image/png'});
        }
        return route.continue();
      });
      await page.addInitScript(({theme,language})=>{
        localStorage.setItem('lumetrace.theme.preference',theme);
        localStorage.setItem('lumetrace.language.preference',language);
      },{theme,language});
      await page.goto(`${base}/tests/fixtures/search-visual.html`);
      const input=page.locator('.file-space-global-search-panel input');
      await input.fill('ORCHID');
      const canvas=page.locator('.search-visual-surface canvas');
      const current=page.locator('.search-visual-highlights rect.is-current');
      async function inspect(name) {
        await page.locator('.file-space-global-search-results > button').filter({has:page.locator('strong',{hasText:new RegExp(`^${name.replaceAll('.','\\.')}$`)})}).click();
        await current.first().waitFor();
        await page.waitForFunction(()=>!document.querySelector('.search-visual-preview[aria-busy=true]'));
        const pixels=await canvas.evaluate(canvas=>{
          const svg=canvas.parentElement.querySelector('svg');
          const box=svg.querySelector('rect.is-current');
          const x=Number(box.getAttribute('x')),y=Number(box.getAttribute('y')),w=Number(box.getAttribute('width')),h=Number(box.getAttribute('height'));
          const context=canvas.getContext('2d');
          const data=context.getImageData(Math.floor(x*canvas.width),Math.floor(y*canvas.height),Math.max(1,Math.floor(w*canvas.width)),Math.max(1,Math.floor(h*canvas.height))).data;
          let dark=0;for(let i=0;i<data.length;i+=4)if(data[i]<160&&data[i+1]<160&&data[i+2]<160)dark++;
          const a=canvas.getBoundingClientRect(),b=svg.getBoundingClientRect();
          return {dark,pixels:data.length/4,canvasWidth:a.width,overlayWidth:b.width,canvasHeight:a.height,overlayHeight:b.height};
        });
        if(process.env.LUMETRACE_SCREENSHOT_DIR)await page.screenshot({path:path.join(process.env.LUMETRACE_SCREENSHOT_DIR,`search-${theme}-${language}-${width}-${name}.png`)});
        assert(pixels.dark>10&&pixels.dark/pixels.pixels>0.015,`${name}: highlighted region must contain actual text ink: ${JSON.stringify(pixels)}`);
        assert(Math.abs(pixels.canvasWidth-pixels.overlayWidth)<1&&Math.abs(pixels.canvasHeight-pixels.overlayHeight)<1,'overlay and displayed raster must have identical bounds');
        assert.equal(await page.locator('.file-space-global-search-preview-document').count(),0,'no extracted-text sheet for visual files');
      }
      await inspect('document.png');
      const reads=imageReads;
      await input.fill('orchid');await current.first().waitFor();await page.waitForTimeout(300);
      assert.equal(imageReads,reads,'query edits reuse image bytes');
      for(const name of names.slice(1))await inspect(name);
      const pdfReads=await page.evaluate(()=>window.__visualCalls.read_file_space_pdf);
      await input.fill('ORCHID');await page.waitForTimeout(300);await current.first().waitFor();
      // Changing query returns to the first result (existing behavior), so test
      // hit navigation without changing the file by resizing the preview instead.
      await inspect('mixed.pdf');
      assert.match(await page.locator('.search-visual-preview .file-space-global-search-preview-location').innerText(),/2/);
      const beforeResize=await page.evaluate(()=>window.__visualCalls.read_file_space_pdf);
      await page.setViewportSize({width:width+60,height:width===920?640:800});
      await page.waitForTimeout(300);await current.first().waitFor();
      assert.equal(await page.evaluate(()=>window.__visualCalls.read_file_space_pdf),beforeResize,'resizing rerenders only, no repeated PDF download');
      const scrolling=await page.locator('.file-space-global-search-results').evaluate(el=>el.scrollHeight>el.clientHeight);
      assert(scrolling,'dense result list must own its scroll');
      await page.evaluate(()=>{window.__delayPreview=true;});
      await page.locator('.file-space-global-search-results > button').filter({hasText:'scan.pdf'}).click();
      await page.locator('.file-space-global-search-results > button').filter({hasText:'document.png'}).first().click();
      await page.waitForTimeout(650);await current.first().waitFor();
      assert.equal(await canvas.getAttribute('aria-label'),'document.png','late PDF result must not replace selected image');
      await page.evaluate(()=>{window.__delayPreview=false;});
      failPdf=true;
      await page.locator('.file-space-global-search-results > button').filter({hasText:'scan.pdf'}).click();
      await page.locator('.search-visual-preview [role=alert]').waitFor();
      assert.equal(await current.count(),0,'failed source must not show stale boxes');
      failPdf=false;
      await page.locator('.search-visual-preview [role=alert] button').click();await current.first().waitFor();
      await input.fill('filename');await page.waitForTimeout(250);
      assert.equal(await page.locator('.file-space-global-search-preview').count(),0,'filename-only search retains compact layout');
      await input.fill('');await page.waitForTimeout(200);
      assert.equal(await page.locator('.file-space-global-search-preview').count(),0,'empty search retains compact layout');
      await input.fill('ORCHID');await current.first().waitFor();
      await page.keyboard.press('Escape');await page.locator('.file-space-global-search-panel').waitFor({state:'detached'});
      await page.locator('#reopen').click();assert.equal(await input.inputValue(),'ORCHID','reopening keeps query');
      assert.deepEqual(errors,[]);
      console.log(JSON.stringify({theme,language,width,realNativeGeometry:true,pdfReads,loadingRetry:true,staleResults:true,denseScrolling:true}));
      await page.close();
    }
  } finally { await browser.close(); }
})().catch(error=>{console.error(error);process.exitCode=1;});
