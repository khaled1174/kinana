import assert from 'node:assert/strict';
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
const evidence = process.env.KINANA_UI_EVIDENCE||fileURLToPath(new URL('../../docs/evidence/phase-6/batch-4-5/', import.meta.url));
describe('Kinana native Tauri shell — batches 6.4–6.5', () => {
  it('starts in Arabic and RTL, with real core status', async () => {
    await $('[data-testid="search"]').waitForDisplayed({ timeout: 15000 });
    assert.equal(await $('html').getAttribute('lang'), 'ar');
    assert.equal(await $('html').getAttribute('dir'), 'rtl');
    assert.equal(await $$('h1').length, 1);
    assert.equal(await $('[data-testid="page-title"]').getText(), 'المخزن');
    assert.equal(await $('.engine-panel').isExisting(), false);
    assert.equal(await $('.intro').isExisting(), false);
  });
  it('calls the public Tauri command against the isolated HOME without mocking', async () => {
    const actual = await browser.tauri.execute(async ({ core }) => core.invoke('bootstrap'));
    assert.equal(actual.version, '0.2.0');
    assert.deepEqual(actual.agents.map(a => [a.name, a.project_skills, a.global_skills]), [
      ['claude', '.claude/skills', path.join(process.env.KINANA_UI_HOME, '.claude/skills')],
      ['codex', '.agents/skills', path.join(process.env.KINANA_UI_HOME, '.agents/skills')],
      ['cursor', '.agents/skills', path.join(process.env.KINANA_UI_HOME, '.agents/skills')],
    ]);
    writeFileSync(path.join(evidence, 'native-core-response.json'), JSON.stringify(actual, null, 2)+'\n');
  });
  it('navigates the three shell sections and switches language with direction', async () => {
    await $('[data-testid="nav-projects"]').click();
    assert.match(await $('[data-testid="page-title"]').getText(), /المشاريع/);
    assert.equal(await $('[data-testid="nav-projects"]').getAttribute('aria-current'), 'page');
    await $('[data-testid="nav-settings"]').click();
    assert.match(await $('[data-testid="page-title"]').getText(), /إعدادات/);
    await $('[data-testid="language"]').click();
    assert.equal(await $('html').getAttribute('lang'), 'en');
    assert.equal(await $('html').getAttribute('dir'), 'ltr');
    assert.equal(await $('[data-testid="page-title"]').getText(), 'Settings');
    await $('[data-testid="nav-vault"]').click();
    await $('[data-testid="language"]').click();
    assert.equal(await $('html').getAttribute('dir'), 'rtl');
  });
  it('enforces the configured CSP in the actual WebView while the core bridge works', async () => {
    const runtime = await browser.execute(async () => {
      const violations = [];
      const collect = event => violations.push({ directive: event.effectiveDirective, blocked: event.blockedURI, policy: event.originalPolicy, disposition: event.disposition });
      document.addEventListener('securitypolicyviolation', collect);
      const script = document.createElement('script');
      script.textContent = 'window.__phase61_inline_executed = true';
      document.body.appendChild(script);
      await fetch('https://kinana-csp.invalid/phase61-probe').catch(() => {});
      await new Promise(resolve => setTimeout(resolve, 150));
      script.remove();
      document.removeEventListener('securitypolicyviolation', collect);
      return { violations, inlineExecuted: window.__phase61_inline_executed === true,
        origin: location.origin, direction: getComputedStyle(document.documentElement).direction };
    });
    assert.equal(runtime.inlineExecuted, false);
    assert.ok(runtime.violations.some(v => v.directive.startsWith('script-src') && v.blocked === 'inline' && v.disposition === 'enforce'));
    assert.ok(runtime.violations.some(v => v.directive === 'connect-src' && v.blocked.startsWith('https://kinana-csp.invalid') && v.disposition === 'enforce'));
    runtime.csp = runtime.violations[0].policy;
    const policy = Object.fromEntries(runtime.csp.split(';').filter(x => x.trim()).map(x => { const [key, ...values] = x.trim().split(/\s+/); return [key, values]; }));
    for (const key of ['img-src', 'frame-src', 'object-src']) assert.deepEqual(policy[key], ["'none'"]);
    assert.deepEqual(policy['connect-src'], ["'self'"]);
    assert.ok(policy['script-src'].includes("'self'"));
    for (const values of Object.values(policy)) {
      assert.ok(!values.some(value => /unsafe-inline|unsafe-eval|https?:|data:|blob:|\*/.test(value)), `Unexpected CSP source: ${values}`);
    }
    assert.equal(runtime.direction, 'rtl');
    writeFileSync(path.join(evidence, 'runtime-csp.json'), JSON.stringify(runtime, null, 2)+'\n');
    await browser.execute(() => document.activeElement?.blur());

  });
});
