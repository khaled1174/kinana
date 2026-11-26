import { statSync, existsSync, realpathSync } from 'node:fs';
import path from 'node:path';

const bundle = 'Kinana E2E.app';

export function isolatedSession() {
  const { KINANA_UI_BASE: base, KINANA_UI_HOME: home, KINANA_UI_EVIDENCE: evidence,
    KINANA_UI_TMP: tmp, KINANA_E2E_BINARY: binary, KINANA_TEST_CLI: cli } = process.env;
  if (!base || !/^kinana-ui-[a-zA-Z0-9_]+$/.test(path.basename(base))
      || ![home, tmp, evidence, binary, cli].every(value => value && existsSync(value))) {
    throw new Error('Launch native UI tests through scripts/run-isolated-ui.py.');
  }
  const parent = path.dirname(base);
  const privateDarwinTemp = /^\/private\/var\/folders\/[^/]+\/[^/]+\/T$/.test(parent)
    && statSync(parent).uid === process.getuid() && !(statSync(parent).mode & 0o077);
  if ((parent !== '/private/tmp' && !privateDarwinTemp)
    || realpathSync(base) !== base || [home, tmp, evidence, binary, cli].some(
    value => !realpathSync(value).startsWith(base + path.sep),
  ) || [home, tmp, evidence].some(value => realpathSync(value) !== value)
    || home !== path.join(base, 'home') || tmp !== path.join(base, 'tmp')
    || process.env.HOME !== home || process.env.TMPDIR !== tmp + path.sep
    || !binary.includes(path.sep + bundle + '/Contents/MacOS/')) {
    throw new Error('Native UI tests require their own temporary HOME, TMPDIR, and Kinana E2E bundle.');
  }
  return {base, home, tmp, evidence, binary, cli};
}

export function e2eService(session, extra = {}) {
  return ['@wdio/tauri-service', {
    driverProvider: 'embedded',
    appBinaryPath: session.binary,
    env: {...extra, HOME: session.home, TMPDIR: session.tmp + path.sep, KINANA_E2E_HOME: session.home},
    captureBackendLogs: true,
    captureFrontendLogs: true,
  }];
}

async function connectTestBridge() {
  // The release-parity profile ships the production frontend, so initialise
  // the embedded driver's direct-eval bridge from the test runner.
  await browser.waitUntil(() => browser.execute(() => {
    const core = window.__TAURI__?.core;
    if (!core?.invoke) return false;
    window.__wdio_original_core__ ??= core;
    return true;
  }));
}

export async function focusTestWindow() {
  await connectTestBridge();
  await browser.execute(() => { document.title = 'Kinana E2E'; });
  await browser.tauri.execute(({core}) => core.invoke('plugin:window|set_focus', {label: 'main'}));
  await browser.waitUntil(() => browser.execute(() => document.visibilityState === 'visible'));
}

async function settleAnimations() {
  await browser.execute(() => {
    // WebKit can park finite entry animations when macOS changes window focus.
    for (const animation of document.getAnimations()) {
      if (animation.playState !== 'finished' && Number.isFinite(animation.effect?.getComputedTiming().endTime)) {
        animation.finish();
      }
    }
  });
}

export function prepareTestDriver() {
  browser.overwriteCommand('refresh', async function (original) {
    const previousDocument = await browser.execute(() => performance.timeOrigin);
    await original();
    // WebKit may acknowledge refresh before replacing the old document.
    await browser.waitUntil(() => browser.execute(
      previous => performance.timeOrigin !== previous && document.readyState === 'complete',
      previousDocument,
    ));
    await connectTestBridge();
  });
  browser.overwriteCommand('isDisplayed', async function (original, options) {
    await settleAnimations();
    return original(options);
  }, true);
  browser.overwriteCommand('click', async function (original, options) {
    await settleAnimations();
    return original(options);
  }, true);
}
