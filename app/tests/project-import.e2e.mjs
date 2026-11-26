import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {cpSync, lstatSync, mkdirSync, readdirSync, readFileSync, readlinkSync, writeFileSync} from 'node:fs';
import path from 'node:path';
import {isolatedSession} from './support/isolated-ui.mjs';
import {syntheticSkill} from './support/synthetic-skill.mjs';

const session = isolatedSession();
const project = path.join(session.base, 'project-skill-import');
const vault = path.join(session.home, 'SkillsBox');
const testId = id => `[data-testid="${id}"]`;
const control = id => $(testId(id));

function tree(root) {
  return readdirSync(root).sort().map(name => {
    const entry = path.join(root, name);
    const info = lstatSync(entry);
    const metadata = {name, mode: info.mode, uid: info.uid, gid: info.gid};
    if (info.isDirectory()) return {...metadata, entries: tree(entry)};
    if (info.isSymbolicLink()) return {...metadata, target: readlinkSync(entry)};
    return {...metadata, sha256: digest(readFileSync(entry))};
  });
}

function digest(input) {
  return createHash('sha256').update(input).digest('hex');
}

function cli(args) {
  const result = spawnSync(session.cli, ['--vault', vault, '--json', ...args], {encoding: 'utf8'});
  assert.equal(result.status, 0, result.stdout + result.stderr);
  return JSON.parse(result.stdout);
}

function vaultIds() {
  return cli(['list']).map(skill => skill.id).sort();
}

async function selectProject() {
  await control('nav-projects').click();
  // The isolated fixture registers the folder before the suite. All project
  // and scan calls made by this UI path use the real native bridge.
  const row = $(`${testId('project-row')}[data-path="${project}"]`);
  await row.waitForDisplayed();
  await row.click();
}

async function reviewProject() {
  await control('review-project-skills').waitForEnabled();
  await control('review-project-skills').click();
  await control('scan-results').waitForDisplayed({timeout: 60000});
  await control('scan-progress').waitForExist({reverse: true, timeout: 60000});
}

async function closeScan() {
  await control('cancel-scan').click();
  await $('.scan-dialog').waitForExist({reverse: true});
}

async function inspectProject() {
  const projects = await browser.tauri.execute(({core}, selected) =>
    core.invoke('projects', {selected}), [project]);
  return projects.find(item => item.path === project);
}

describe('Opt-in import of project skills', () => {
  const evidence = [];
  const newNote = '.claude/skills/local-new';
  const uncheckedNote = '.codex/skills/local-unselected';
  const versionA = '.agents/skills/version-a';
  const versionB = '.cursor/skills/version-b';
  const equalCopy = '.cursor/skills/color-copy';
  const divergentCopy = '.claude/skills/color-local';
  let baseline, initialVault;

  before(() => {
    mkdirSync(project);
    writeFileSync(path.join(project, 'README.md'), 'Project fixture leaves this tree unchanged.\n');
    syntheticSkill(path.join(project, newNote), 'local-new');
    // Store a review finding as text. Nothing in SKILL.md is executed.
    const manifest = path.join(project, newNote, 'SKILL.md');
    writeFileSync(manifest, readFileSync(manifest, 'utf8') + '\n```sh\ncurl https://example.invalid/reference\n```\n');
    syntheticSkill(path.join(project, uncheckedNote), 'local-unselected');
    syntheticSkill(path.join(project, versionA), 'local-version', 'Project version A.');
    syntheticSkill(path.join(project, versionB), 'local-version', 'Project version B.');
    syntheticSkill(path.join(project, divergentCopy), 'emil-color', 'A different project copy.');
    mkdirSync(path.dirname(path.join(project, equalCopy)), {recursive: true});
    cpSync(path.join(vault, 'skills', 'emil-color'), path.join(project, equalCopy), {recursive: true});
    cli(['use', 'emil-ui-review', '--project', project, '--agent', 'claude']);
    cli(['unuse', 'emil-ui-review', '--project', project, '--agent', 'claude']);
    baseline = tree(project);
    initialVault = tree(vault);
  });

  afterEach(function () {
    const after = tree(project);
    evidence.push({
      test: this.currentTest.title,
      before_sha256: digest(JSON.stringify(baseline)),
      after_sha256: digest(JSON.stringify(after)),
      project_lock_present: after.some(item => item.name === '.kinana.lock'),
    });
    writeFileSync(path.join(session.evidence, 'project-import-tree.json'),
      JSON.stringify({selection: "registered synthetic project", tree: baseline, checks: evidence}, null, 2) + '\n');
    assert.deepEqual(after, baseline);
  });

  it('counts distinct local skills in each supported project root without writing', async () => {
    await selectProject();
    await control('project-import-offer').waitForDisplayed();
    assert.match(await control('project-import-offer').getText(), /وجدنا [6٦] مهارة/);
    const report = await inspectProject();
    const paths = report.local.map(item => path.relative(project, item.entry.path)).sort();
    assert.deepEqual(paths, [newNote, uncheckedNote, versionA, versionB, equalCopy, divergentCopy].sort());
    assert.deepEqual(tree(vault), initialVault);
  });

  it('skips the offer and leaves both project and vault alone', async () => {
    await control('skip-project-skills').click();
    await control('project-import-offer').waitForExist({reverse: true});
    assert.equal(await $('.scan-dialog').isExisting(), false);
    assert.deepEqual(tree(vault), initialVault);
    await control('nav-vault').click();
  });

  it('opens the existing scan with audit, duplicate, drift and explicit version selection', async () => {
    await selectProject();
    await reviewProject();
    assert.match(await control('scan-new').getText(), /local-new/);
    const audit = control('scan-audit-local-new');
    await audit.waitForDisplayed();
    await audit.$('summary').click();
    await audit.$('.audit-details').waitForDisplayed();
    await control('scan-duplicates').$('summary').click();
    assert.match(await control('scan-duplicates').getText(), /emil-color/);
    assert.match(await control('scan-drifted').getText(), /emil-color/);
    assert.match(await control('scan-new').$('h3').getText(), /[1١]\./);
    assert.match(await control('scan-existing-results').$('h3').getText(), /[2٢]\./);
    const folders = await $$(testId('scan-select-folder'));
    assert.ok(folders.length > 1);
    await folders[0].click();
    assert.equal(await control('scan-primary-local-version-none').isSelected(), true);
    assert.equal(await control('scan-primary-local-version-0').isSelected(), false);
    assert.equal(await control('scan-primary-local-version-1').isSelected(), false);
    await browser.saveScreenshot(path.join(session.evidence, 'project-import-review.png'));
    assert.deepEqual(tree(vault), initialVault);
  });

  it('cancels the scan without importing or recording a source', async () => {
    await closeScan();
    assert.deepEqual(tree(vault), initialVault);
    assert.equal(vaultIds().includes('local-new'), false);
  });

  it('imports only the confirmed skills and chosen version without activating them', async () => {
    const expected = [...vaultIds(), 'local-new', 'local-version'].sort();
    await reviewProject();
    const description = 'Reviewed description: "Project workflow"';
    await control('scan-description-local-new').setValue(description);
    await control('scan-select-local-unselected').click();
    await control('scan-primary-local-version-1').click();
    await control('scan-risky-hint').waitForDisplayed();
    assert.equal(await control('confirm-scan-import').isEnabled(), false);
    await control('scan-allow-risky').click();
    await control('confirm-scan-import').waitForEnabled();
    await control('confirm-scan-import').click();
    await $('.scan-dialog').waitForExist({reverse: true, timeout: 60000});
    assert.deepEqual(vaultIds(), expected);
    assert.equal(cli(["show","local-new"]).metadata.description.improved,description);
    assert.deepEqual(readFileSync(path.join(vault, 'skills/local-version/SKILL.md')),
      readFileSync(path.join(project, versionB, 'SKILL.md')));
    const report = await inspectProject();
    assert.deepEqual(report.activations, []);
    assert.equal(report.state, 'no_lock');
  });

  it('previews just the imported batch with a temporary agent and can skip without writing', async () => {
    const before = tree(vault);
    await control('classify-project-import').click();
    const agent = control('scan-classification-agent');
    await agent.waitForDisplayed();
    await browser.waitUntil(async()=>/Claude Code/.test(await agent.getText()));
    await agent.click();
    await control("scan-classification-agent-keywords").waitForDisplayed();
    await control("scan-classification-agent-keywords").click();
    await control('confirm-classify').waitForEnabled();
    assert.match(await control('classify-count').getText(), /[2٢]/);
    await control('cancel-classify').click();
    await $('.classify-dialog').waitForExist({reverse: true});
    assert.deepEqual(tree(vault), before);
  });

  it('shows the vault ID after refreshing an exact content match', async () => {
    const linked = id => $(`${testId('project-linked-skill')}[data-skill-id="${id}"]`);
    await linked('local-new').waitForDisplayed();
    await linked('local-version').waitForDisplayed();
    await linked('emil-color').waitForDisplayed();
    assert.match(await linked('local-version').getText(), /version-b/);
    assert.match(await linked('emil-color').getText(), /color-copy/);
    await browser.saveScreenshot(path.join(session.evidence, 'project-import-linked.png'));
  });

  it('keeps different contents and unselected skills in the unmanaged section', async () => {
    const unmanaged = await $('.local-list').getText();
    for (const target of ['version-a', 'color-local', 'local-unselected']) {
      assert.ok(unmanaged.includes(target), target);
    }
    const linked = await control('project-linked-skills').getText();
    for (const target of ['version-a', 'color-local', 'local-unselected']) {
      assert.ok(!linked.includes(target), target);
    }
  });
});
