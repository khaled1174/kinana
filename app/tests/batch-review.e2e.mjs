import assert from 'node:assert/strict';

const ids = ['emil-design-foundations', 'emil-ui-review'];
async function detail(id) {
  return browser.tauri.execute(({core}, id) => core.invoke('skill_detail', {id}), id);
}

describe('Batch description review', () => {
  it('previews both descriptions, approves them together, then restores both on undo', async () => {
    await $('[data-testid="open-batch-review"]').waitForDisplayed();
    const before = await Promise.all(ids.map(detail));
    assert.ok(before.every(d => d.detail.metadata.proposal));
    await $('[data-testid="open-batch-review"]').click();
    for (const id of ids) await $(`[data-testid="batch-select-${id}"]`).click();
    await $('[data-testid="batch-accept"]').click();
    await $('[data-testid="batch-confirm"]').waitForDisplayed();
    assert.deepEqual(await Promise.all(ids.map(detail)), before);
    await $('[data-testid="batch-confirm"]').click();
    await browser.waitUntil(async () => (await Promise.all(ids.map(detail)))
      .every(d => d.detail.metadata.description.active === 'improved'));
    await $('[data-testid="undo"]').click();
    await $('[data-testid="confirm-undo"]').click();
    await browser.waitUntil(async () => (await Promise.all(ids.map(detail)))
      .every(d => d.detail.metadata.description.active === 'original' && d.detail.metadata.proposal));
  });
});
