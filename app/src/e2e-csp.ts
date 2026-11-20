// E2E bundle only. Never imported by the default user build.
// Recreate script nodes so CSP is tested on an actual executable insertion path.
export function installCspProbe() {
 const testWindow=window as unknown as {__kinanaE2EUnsafe?: (html:string)=>HTMLElement};
 testWindow.__kinanaE2EUnsafe=(html:string)=>{
  document.querySelector('[data-testid="csp-unsafe-preview"]')?.remove();
  const container=document.createElement('section');container.dataset.testid='csp-unsafe-preview';
  container.className='e2e-unsafe-preview';container.innerHTML=html;
  document.querySelector('main')?.appendChild(container);
  for(const original of container.querySelectorAll('script')) {
   const script=document.createElement('script');for(const attr of original.attributes)script.setAttribute(attr.name,attr.value);script.textContent=original.textContent;original.replaceWith(script);
  }
  return container;
 };
}
