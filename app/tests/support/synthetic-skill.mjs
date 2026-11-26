import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';

export function syntheticSkill(folder, id, description = 'A synthetic skill for the Kinana UI tests.') {
  mkdirSync(folder, {recursive: true});
  writeFileSync(path.join(folder, 'SKILL.md'), [
    '---', `name: ${JSON.stringify(id)}`, `description: ${JSON.stringify(description)}`,
    '---', '', '# Synthetic fixture', '', 'This fixture describes a test workflow.', '',
  ].join('\n'));
}
