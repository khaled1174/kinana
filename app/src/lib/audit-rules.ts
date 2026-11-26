// Audit rules versions: stored audits made by
// older rules are never rewritten silently; the app shows them as older and
// offers a preview whose confirmation re-audits every skill in one commit.
// Must equal `RULES_VERSION` in crates/core/src/audit.rs (checked by a test).
export const AUDIT_RULES_VERSION = 5;

export type LevelChange = { id: string; from: string; to: string };
export type RulesUpgrade = {
  rules_version: number;
  older: number;
  raised: LevelChange[];
  lowered: LevelChange[];
  unchanged: number;
};

/** "legacy": stored before rule versions existed; "older": an earlier version. */
export function auditRulesState(version: number): "legacy" | "older" | "current" {
  if (!version) return "legacy";
  return version < AUDIT_RULES_VERSION ? "older" : "current";
}

/** Rows for the preview: skills whose level goes up first, then those going down. */
export function upgradeRows(upgrade: RulesUpgrade): { direction: "raised" | "lowered"; change: LevelChange }[] {
  const byId = (a: LevelChange, b: LevelChange) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  return [
    ...[...upgrade.raised].sort(byId).map(change => ({ direction: "raised" as const, change })),
    ...[...upgrade.lowered].sort(byId).map(change => ({ direction: "lowered" as const, change })),
  ];
}
