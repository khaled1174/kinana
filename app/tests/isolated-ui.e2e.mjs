import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync} from 'node:fs';
import path from 'node:path';
import {isolatedSession} from './support/isolated-ui.mjs';
import {syntheticSkill} from './support/synthetic-skill.mjs';

const session = isolatedSession();
const vault = path.join(session.home, 'KinanaVault');

function projectTree(folder) {
  return readdirSync(folder, {withFileTypes: true}).sort((a, b) => a.name.localeCompare(b.name))
    .map(entry => {
      const location = path.join(folder, entry.name);
      return [entry.name, entry.isDirectory() ? projectTree(location)
        : createHash('sha256').update(readFileSync(location)).digest('hex')];
    });
}

async function invoke(command, args = {}) {
  return browser.tauri.execute(async ({core}, command, args) => core.invoke(command, args), command, args);
}

async function capture(stage) {
  const captureResult = async operation => {
    try {
      return {value: await operation()};
    } catch (error) {
      return {error: String(error)};
    }
  };
  const dom = await captureResult(() => browser.execute(() => ({
    url: location.href, title: document.title, readyState: document.readyState,
    visibility: document.visibilityState, root: document.querySelector('#root')?.innerHTML ?? null,
    dialogs: [...document.querySelectorAll('[role=dialog]')].map(dialog => dialog.outerHTML),
    body: document.body?.innerText ?? null, scripts: [...document.scripts].map(script => script.src),
    resources: performance.getEntriesByType('resource').map(entry => ({
      name: entry.name, duration: entry.duration, transferSize: entry.transferSize,
    })),
    hasTauri: typeof window.__TAURI_INTERNALS__?.invoke === 'function',
    startupErrors: window.__kinanaStartupErrors ?? null,
  })));
  if (dom.value?.root != null) {
    writeFileSync(path.join(session.evidence, `${stage}.html`),
      [dom.value.root, ...dom.value.dialogs].join('\n'));
    delete dom.value.root;
    delete dom.value.dialogs;
  }
  const screenshot = await captureResult(() => browser.saveScreenshot(
    path.join(session.evidence, `${stage}.png`),
  ).then(() => 'saved'));
  const backend = await captureResult(() => browser.execute(async () => {
    const call = command => Promise.race([
      window.__TAURI_INTERNALS__.invoke(command).then(
        value => ({value}), error => ({error}),
      ),
      new Promise(resolve => setTimeout(() => resolve({timeout: true}), 3000)),
    ]);
    return {bootstrap: await call('bootstrap'), settings: await call('desktop_settings')};
  }));
  writeFileSync(path.join(session.evidence, `${stage}.json`), JSON.stringify({
    dom, screenshot, backend, temporaryHomeExists: existsSync(session.home),
    desktopPreferencesExist: existsSync(path.join(session.home, '.kinana/desktop.toml')),
    vaultExists: existsSync(vault),
  }, null, 2) + '\n');
}

async function probeLoading() {
  const result = await browser.execute(async () => {
    const events = [];
    window.addEventListener('error', event => events.push({
      type: event.type, message: event.message, source: event.filename || event.target?.src,
    }), true);
    window.addEventListener('unhandledrejection', event => events.push({
      type: event.type, reason: String(event.reason), stack: event.reason?.stack,
    }));
    window.addEventListener('vite:preloadError', event => events.push({
      type: event.type, reason: String(event.payload), stack: event.payload?.stack,
    }));
    window.addEventListener('securitypolicyviolation', event => events.push({
      type: event.type, directive: event.violatedDirective, blockedURI: event.blockedURI,
    }));
    const check = async test => {
      try {
        return await Promise.race([test(), new Promise(resolve =>
          setTimeout(() => resolve({timeout: true}), 2500))]);
      } catch (error) {
        return {error: String(error), stack: error.stack};
      }
    };
    const script = document.querySelector('script[type=module]');
    const response = url => check(async () => {
      const response = await fetch(url);
      return {status: response.status, contentType: response.headers.get('Content-Type'),
        policy: response.headers.get('Content-Security-Policy'),
        firstBytes: (await response.text()).slice(0, 150)};
    });
    const documentResponse = await response(location.href);
    const scriptResponse = script ? await response(script.src) : null;
    const flags = {wdio: typeof window.wdioTauri?.execute, probe: typeof window.__kinanaE2EUnsafe};
    await new Promise(resolve => setTimeout(resolve, 100));
    return {location: location.href, markup: script?.outerHTML, documentResponse, scriptResponse,
      events, flags, rendered: document.querySelector('#root')?.childElementCount,
      userAgent: navigator.userAgent};
  });
  writeFileSync(path.join(session.evidence, 'module-loading.json'), JSON.stringify(result, null, 2) + '\n');
}

describe('Kinana E2E isolation', () => {
  afterEach(async function () {
    if (this.currentTest?.state === 'failed') {
      await capture('failure');
      await probeLoading();
    }
  });

  it('starts from a temporary HOME and writes only the synthetic vault and preferences', async () => {
    syntheticSkill(path.join(session.home, '.claude/skills/isolation-fixture'), 'isolation-fixture');
    await capture('startup');
    await $('[data-testid="onboarding"]').waitForDisplayed({timeout: 12000});
    const initial = await invoke('bootstrap');
    assert.equal(initial.home, session.home);
    assert.equal(initial.vault, vault);
    assert.equal(initial.first_run, true);
    assert.equal(existsSync(vault), false);

    await $('[data-testid="settings-initialize"]').click();
    await $('[data-testid="settings-preview"]').waitForDisplayed();
    await $('[data-testid="confirm-settings"]').click();
    const settings = path.join(session.home, '.kinana/desktop.toml');
    await browser.waitUntil(() => existsSync(settings) && existsSync(path.join(vault, 'box.toml')));
    assert.ok(readFileSync(settings, 'utf8').includes(vault));

    const marker = await browser.execute(() => {
      window.localStorage.setItem('kinana-e2e-isolation', 'synthetic only');
      return window.localStorage.getItem('kinana-e2e-isolation');
    });
    assert.equal(marker, 'synthetic only');
    await browser.saveScreenshot(path.join(session.evidence, 'isolation-ui.png'));
    writeFileSync(path.join(session.evidence, 'locations.json'), JSON.stringify({
      home: initial.home, temp: session.tmp, vault, preferences: settings,
    }, null, 2) + '\n');
  });

  if (process.env.KINANA_ISOLATED_SUITE === 'projects') {
    it('discovers a local skill for a new project without a lock and shows it in the project screen', async () => {
      const empty = path.join(session.base, 'project-empty');
      const local = path.join(session.base, 'project-local');
      mkdirSync(empty);
      syntheticSkill(path.join(local, '.claude/skills/local-note'), 'local-note');
      const projects = [empty, local];
      const before = projects.map(projectTree);
      assert.ok(projects.every(folder => !existsSync(path.join(folder, '.kinana.lock'))));
      const inspect = await invoke('projects', {selected: projects});

      // The observed symptom starts after a project appears in the list. The
      // synthetic registry supplies the rows without invoking a macOS picker.
      writeFileSync(path.join(session.home, '.kinana/projects.toml'), [
        'version = 1',
        ...projects.flatMap(folder => ['[[projects]]', `path = ${JSON.stringify(folder)}`,
          `vault = ${JSON.stringify(vault)}`]),
      ].join('\n') + '\n');
      await $('[data-testid="nav-projects"]').click();
      const rendered = [];
      for (const folder of projects) {
        const row = await $(`[data-testid="project-row"][data-path="${folder}"]`);
        await row.waitForDisplayed();
        await row.click();
        const localList = await $('.local-list');
        await localList.waitForDisplayed();
        rendered.push({
          folder, local: await localList.getText(),
          activations: await $('[data-testid="project-activations"]').getText(),
          localRows: (await localList.$$('.local-row')).length,
        });
        await browser.saveScreenshot(path.join(session.evidence, `${path.basename(folder)}.png`));
      }
      const after = projects.map(projectTree);
      writeFileSync(path.join(session.evidence, 'project-reproduction.json'), JSON.stringify({
        input: 'Direct IPC for newly selected roots, then UI selection from synthetic registry rows',
        before, after, inspect, rendered, projectTreesEqual: JSON.stringify(before) === JSON.stringify(after),
      }, null, 2) + '\n');
      assert.deepEqual(after, before);
      assert.ok(inspect.every(project => project.state === 'no_lock' && project.activations.length === 0));
      assert.equal(inspect.find(project => project.path === empty).local.length, 0);
      assert.equal(inspect.find(project => project.path === local).local[0]?.entry.name, 'local-note');
      assert.equal(rendered[0].localRows, 0);
      assert.match(rendered[1].local, /local-note/);
      assert.equal(rendered[1].localRows, 1);
    });
  }
});
