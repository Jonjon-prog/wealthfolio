/** A row or an asset class the user points at or selects in the Amounts panel. */
export type HighlightTarget =
  | { kind: "row"; assetId: string }
  | { kind: "category"; categoryId: string };

export interface HighlightState {
  /** Temporary: the pointer is over it, or focus is inside it. */
  pointed: HighlightTarget | null;
  /** Kept by a click or a tap until cleared. */
  selected: HighlightTarget | null;
}

export interface CategoryShare {
  categoryId: string;
  weightBps: number;
}

/** How a row or a class reads while something is highlighted. */
export type Emphasis = "none" | "active" | "lit" | "dim";

export function sameTarget(left: HighlightTarget | null, right: HighlightTarget | null): boolean {
  if (left === null || right === null) return left === right;
  if (left.kind === "row") return right.kind === "row" && left.assetId === right.assetId;
  return right.kind === "category" && left.categoryId === right.categoryId;
}

/** Pointing previews something else while the selection waits underneath. */
export function activeTarget(state: HighlightState): HighlightTarget | null {
  return state.pointed ?? state.selected;
}

/** A row lights when it is the active row, or when it touches the active class. */
export function rowEmphasis(
  active: HighlightTarget | null,
  assetId: string,
  shares: readonly CategoryShare[],
): Emphasis {
  if (!active) return "none";
  if (active.kind === "row") return active.assetId === assetId ? "active" : "dim";
  return shares.some((share) => share.categoryId === active.categoryId && share.weightBps > 0)
    ? "lit"
    : "dim";
}

/**
 * The row's share of the active class, shown beside the symbol only when the
 * row is partly in it, so a 60/40 fund does not read as fully in Bonds.
 */
export function partialShareBps(
  active: HighlightTarget | null,
  shares: readonly CategoryShare[],
): number | null {
  if (active?.kind !== "category") return null;
  const weightBps = shares.find((share) => share.categoryId === active.categoryId)?.weightBps ?? 0;
  return weightBps > 0 && weightBps < 10_000 ? weightBps : null;
}

/**
 * A class lights when it is the active class, or when the active row touches
 * it. `activeRowShares` is undefined unless a row is active.
 */
export function categoryEmphasis(
  active: HighlightTarget | null,
  categoryId: string,
  activeRowShares: readonly CategoryShare[] | undefined,
): Emphasis {
  if (!active) return "none";
  if (active.kind === "category") return active.categoryId === categoryId ? "active" : "dim";
  return activeRowShares?.some((share) => share.categoryId === categoryId && share.weightBps > 0)
    ? "lit"
    : "dim";
}

/**
 * Half the width of a class's track: 8 points, or twice the tolerance band when
 * that is wider, so the band never fills the track.
 */
export function trackHalfWindowBps(bandBps: number): number {
  return Math.max(800, bandBps * 2);
}

/**
 * Where a weight sits on a class's track, in percent of its width. The track is
 * centred on the target so the distance to it reads directly. A weight beyond
 * the window stops at the edge; the figures beside the track stay exact.
 */
export function trackPosition(weightBps: number, targetBps: number, halfWindowBps: number): number {
  const position = 50 + ((weightBps - targetBps) / halfWindowBps) * 50;
  return Math.min(97, Math.max(3, position));
}

/** The part of a row's change that moves one of its classes. */
export function changeInCategory(change: number, weightBps: number): number {
  return (change * weightBps) / 10_000;
}

/** The one fact a row's Status cell states, most pressing first. */
export type RowStatus =
  | { kind: "needs_account" }
  | { kind: "check_amount"; message: string }
  | { kind: "price_required" }
  | { kind: "warnings"; messages: readonly string[] }
  | { kind: "rounded"; amount: number }
  | { kind: "stale_price"; date: string }
  | { kind: "not_eligible" }
  | { kind: "placed"; accountIds: readonly string[] };

export interface RowStatusFacts {
  /** What keeps this row out of the preview, if anything. */
  issue?: { kind: "cash" | "position" | "allocation"; message: string };
  /** An added security with no price to size it at. */
  needsPrice: boolean;
  /** The preview's warnings about this row's lines. */
  warnings: readonly string[];
  /** The amount the preview placed, when whole units moved it off the entered one. */
  roundedAmount?: number;
  /** The date of a price the preview marks as dated. */
  stalePriceDate?: string;
  /** Left out of the eligible securities: the calculation will not increase it. */
  isExcluded: boolean;
  /** Where the change sits when more than one account could take it. */
  placedAccountIds: readonly string[];
}

/**
 * One fact per row, so the list stays readable: what blocks the worksheet
 * first, then what the preview says, then where the change sits.
 */
export function rowStatus(facts: RowStatusFacts): RowStatus | null {
  if (facts.issue?.kind === "allocation") return { kind: "needs_account" };
  if (facts.issue) return { kind: "check_amount", message: facts.issue.message };
  if (facts.needsPrice) return { kind: "price_required" };
  if (facts.warnings.length > 0) return { kind: "warnings", messages: facts.warnings };
  if (facts.roundedAmount !== undefined) return { kind: "rounded", amount: facts.roundedAmount };
  if (facts.stalePriceDate) return { kind: "stale_price", date: facts.stalePriceDate };
  if (facts.isExcluded) return { kind: "not_eligible" };
  if (facts.placedAccountIds.length > 0) {
    return { kind: "placed", accountIds: facts.placedAccountIds };
  }
  return null;
}

export interface AmountsRow {
  assetId: string;
  value: number;
  categoryIds: readonly string[];
}

export interface OpenRowFacts {
  /** Rows with a change, including one that cannot be read yet. */
  changedAssetIds: ReadonlySet<string>;
  /** Rows the worksheet or the preview has something to say about. */
  flaggedAssetIds: ReadonlySet<string>;
  /** Classes the calculation left an amount unresolved in. */
  unresolvedCategoryIds: ReadonlySet<string>;
  /** Rows touched during this visit to the panel. */
  touchedAssetIds: ReadonlySet<string>;
}

/**
 * Whether a row stays out of the collapsed group.
 *
 * A row in a class with an unresolved amount stays out because making one of
 * its securities eligible is how that amount gets resolved. A row touched
 * during the visit stays out even when cleared back to zero, so it never
 * vanishes under the cursor.
 */
export function rowStaysOpen(row: AmountsRow, facts: OpenRowFacts): boolean {
  return (
    facts.changedAssetIds.has(row.assetId) ||
    facts.flaggedAssetIds.has(row.assetId) ||
    facts.touchedAssetIds.has(row.assetId) ||
    row.categoryIds.some((categoryId) => facts.unresolvedCategoryIds.has(categoryId))
  );
}

export interface AmountsPartition<T extends AmountsRow> {
  open: T[];
  collapsed: T[];
  collapsedValue: number;
}

/** Splits the list without reordering it: both parts keep the incoming order. */
export function partitionAmountsRows<T extends AmountsRow>(
  rows: readonly T[],
  facts: OpenRowFacts,
): AmountsPartition<T> {
  const open: T[] = [];
  const collapsed: T[] = [];
  let collapsedValue = 0;
  for (const row of rows) {
    if (rowStaysOpen(row, facts)) {
      open.push(row);
    } else {
      collapsed.push(row);
      collapsedValue += row.value;
    }
  }
  return { open, collapsed, collapsedValue };
}
