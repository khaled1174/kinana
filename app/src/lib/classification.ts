import type { ProviderStatus } from "../bridge";

/** Stop codes after which the user picks the next step (Claude or keywords);
 * the same list as `STOPS_THAT_ASK` in crates/core/src/classify.rs. */
export const STOPS_THAT_ASK = ["AGENT_AUTO_UNAVAILABLE", "AGENT_PI_UNAVAILABLE", "AGENT_CLAUDE_UNAVAILABLE", "PI_CONFIRMATION_REQUIRED", "AGENT_USAGE_LIMIT", "AGENT_CREDIT_EXHAUSTED", "AGENT_NOT_LOGGED_IN", "AGENT_MODEL_UNAVAILABLE"];

/** What a finished classification report tells the user about fallbacks. */
export type ReportItem = { id: string; classified_by: string; proposal_created: boolean; reason_code?: string; searched_locations?: string[] };

export type Outcome = {
  /** At least one skill got the keyword classification. */
  fallback: boolean;
  /** Every item fell back to keywords. */
  allFallback: boolean;
  /** The one cause shared by every fallback, shown once instead of per skill. */
  sharedReason?: string;
};

export function classificationOutcome(items: ReportItem[]): Outcome {
  const fallbacks = items.filter(item => item.classified_by === "keywords" && !item.proposal_created);
  const codes = new Set(fallbacks.map(item => item.reason_code));
  const [only] = codes;
  return {
    fallback: fallbacks.length > 0,
    allFallback: fallbacks.length > 0 && fallbacks.length === items.length,
    sharedReason: codes.size === 1 && only ? only : undefined,
  };
}

/** The cause to show next to one skill; empty when it is shown once above. */
export function itemReason(item: ReportItem, outcome: Outcome): string | undefined {
  if (item.classified_by !== "keywords" || item.proposal_created || outcome.sharedReason) return undefined;
  return item.reason_code;
}

/** A stop with one failed provider still has a reason to show. */
export function showProviderDetails(providers: ProviderStatus[], stopped: boolean): boolean {
  return providers.length > 1 || (stopped && providers.some(provider => provider.status === "failed" && !!provider.reason_code));
}

export function unreviewedPiVersion(message: string, providers: ProviderStatus[]) {
  if (!providers.some(provider => provider.agent === "pi" && provider.reason_code === "AGENT_VERSION_UNVERIFIED")) return;
  const versions = message.match(/\bPi (\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.+-]+)?) is pending review; accepted versions: (\d+\.\d+\.\d+(?:, \d+\.\d+\.\d+)*)\./);
  if (versions) return {installed: versions[1], accepted: versions[2]};
}
