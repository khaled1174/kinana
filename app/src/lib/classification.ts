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
