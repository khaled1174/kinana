// r5 (owner, 2026-11-24, item 4b): audit reasons grouped by rule and file.
import type { Audit } from "../bridge";

type Detail = Audit["details"][number];

export function placesLabel(count: number, locale: string) {
  const n = new Intl.NumberFormat(locale).format(count);
  if (locale !== "ar") return `${n} place${count === 1 ? "" : "s"}`;
  if (count === 1) return "موضع واحد";
  if (count === 2) return "موضعان";
  if (count >= 3 && count <= 10) return `${n} مواضع`;
  return `${n} موضعًا`;
}

export type AuditGroup = { rule: string; path: string; level: string; details: Detail[] };

/** Details grouped by rule and file: risky groups first, then by first appearance; risky lines first inside a group. */
export function auditGroups(details: Detail[]): AuditGroup[] {
  const groups: AuditGroup[] = [];
  for (const detail of details) {
    let group = groups.find(g => g.rule === detail.rule && g.path === detail.path);
    if (!group) {
      group = { rule: detail.rule, path: detail.path, level: detail.level, details: [] };
      groups.push(group);
    }
    if (detail.level === "risky") group.level = "risky";
    group.details.push(detail);
  }
  const rank = (level: string) => (level === "risky" ? 0 : 1);
  for (const group of groups) {
    group.details = group.details
      .map((detail, index) => ({ detail, index }))
      .sort((a, b) => rank(a.detail.level) - rank(b.detail.level) || (a.detail.line ?? 0) - (b.detail.line ?? 0) || a.index - b.index)
      .map(entry => entry.detail);
  }
  return groups
    .map((group, index) => ({ group, index }))
    .sort((a, b) => rank(a.group.level) - rank(b.group.level) || a.index - b.index)
    .map(entry => entry.group);
}

