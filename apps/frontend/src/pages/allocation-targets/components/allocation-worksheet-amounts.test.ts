import { describe, expect, it } from "vitest";

import {
  activeTarget,
  categoryEmphasis,
  changeInCategory,
  partialShareBps,
  partitionAmountsRows,
  rowEmphasis,
  rowStaysOpen,
  sameTarget,
  type HighlightTarget,
  type OpenRowFacts,
} from "./allocation-worksheet-amounts";
import { createHighlightStore } from "./allocation-worksheet-highlight";

const balanced = [
  { categoryId: "us", weightBps: 6_000 },
  { categoryId: "bond", weightBps: 4_000 },
];
const usOnly = [{ categoryId: "us", weightBps: 10_000 }];
const row = (assetId: string): HighlightTarget => ({ kind: "row", assetId });
const category = (categoryId: string): HighlightTarget => ({ kind: "category", categoryId });

function facts(overrides: Partial<OpenRowFacts> = {}): OpenRowFacts {
  return {
    changedAssetIds: new Set(),
    flaggedAssetIds: new Set(),
    unresolvedCategoryIds: new Set(),
    touchedAssetIds: new Set(),
    ...overrides,
  };
}

describe("Amounts highlight", () => {
  it("previews what is pointed at while the selection waits underneath", () => {
    expect(activeTarget({ pointed: null, selected: row("vt") })).toEqual(row("vt"));
    expect(activeTarget({ pointed: category("bond"), selected: row("vt") })).toEqual(
      category("bond"),
    );
  });

  it("compares targets by what they point at, not by identity", () => {
    expect(sameTarget(row("vt"), row("vt"))).toBe(true);
    expect(sameTarget(row("vt"), category("vt"))).toBe(false);
    expect(sameTarget(null, null)).toBe(true);
    expect(sameTarget(row("vt"), null)).toBe(false);
  });

  it("lights every class a mixed fund touches, and only those", () => {
    const active = row("vbiax");
    expect(categoryEmphasis(active, "us", balanced)).toBe("lit");
    expect(categoryEmphasis(active, "bond", balanced)).toBe("lit");
    expect(categoryEmphasis(active, "gold", balanced)).toBe("dim");
    expect(categoryEmphasis(null, "gold", undefined)).toBe("none");
  });

  it("lights the rows touching a class, whatever their share of it", () => {
    const active = category("bond");
    expect(rowEmphasis(active, "vbiax", balanced)).toBe("lit");
    expect(rowEmphasis(active, "voo", usOnly)).toBe("dim");
    expect(rowEmphasis(row("voo"), "voo", usOnly)).toBe("active");
    expect(rowEmphasis(row("voo"), "vbiax", balanced)).toBe("dim");
  });

  it("shows a share only for a row partly in the active class", () => {
    expect(partialShareBps(category("bond"), balanced)).toBe(4_000);
    expect(partialShareBps(category("us"), usOnly)).toBeNull();
    expect(partialShareBps(category("bond"), usOnly)).toBeNull();
    expect(partialShareBps(row("vbiax"), balanced)).toBeNull();
  });

  it("splits a row's change by its share of each class", () => {
    expect(changeInCategory(1_200, 6_000)).toBe(720);
    expect(changeInCategory(1_200, 4_000)).toBe(480);
    expect(changeInCategory(-500, 10_000)).toBe(-500);
  });
});

describe("Amounts highlight store", () => {
  it("keeps a newer pointer when the previous one leaves late", () => {
    const store = createHighlightStore();
    store.getState().point(row("a"));
    store.getState().point(row("b"));
    store.getState().unpoint(row("a"));
    expect(store.getState().pointed).toEqual(row("b"));
    store.getState().unpoint(row("b"));
    expect(store.getState().pointed).toBeNull();
  });

  it("toggles a selection, and a tap on the amount field selects without toggling", () => {
    const store = createHighlightStore();
    store.getState().toggleSelected(row("vt"));
    expect(store.getState().selected).toEqual(row("vt"));
    store.getState().select(row("vt"));
    expect(store.getState().selected).toEqual(row("vt"));
    store.getState().toggleSelected(row("vt"));
    expect(store.getState().selected).toBeNull();
  });

  it("forgets a row that left the worksheet", () => {
    const store = createHighlightStore();
    store.getState().select(row("vt"));
    store.getState().point(row("vt"));
    store.getState().forgetRow("vt");
    expect(store.getState()).toMatchObject({ pointed: null, selected: null });
  });
});

describe("Amounts collapsed rows", () => {
  const vti = { assetId: "vti", value: 900, categoryIds: ["us"] };
  const iau = { assetId: "iau", value: 300, categoryIds: ["gold"] };
  const bnd = { assetId: "bnd", value: 200, categoryIds: ["bond"] };

  it("keeps out a row with a change, a warning, an unresolved class or a touch", () => {
    expect(rowStaysOpen(vti, facts())).toBe(false);
    expect(rowStaysOpen(vti, facts({ changedAssetIds: new Set(["vti"]) }))).toBe(true);
    expect(rowStaysOpen(vti, facts({ flaggedAssetIds: new Set(["vti"]) }))).toBe(true);
    expect(rowStaysOpen(iau, facts({ unresolvedCategoryIds: new Set(["gold"]) }))).toBe(true);
    // Cleared back to zero during the visit: still out until the panel is left.
    expect(rowStaysOpen(vti, facts({ touchedAssetIds: new Set(["vti"]) }))).toBe(true);
  });

  it("collapses the rest without reordering either part, and totals its value", () => {
    const partition = partitionAmountsRows(
      [vti, iau, bnd],
      facts({ changedAssetIds: new Set(["bnd"]), unresolvedCategoryIds: new Set(["gold"]) }),
    );
    expect(partition.open.map((item) => item.assetId)).toEqual(["iau", "bnd"]);
    expect(partition.collapsed.map((item) => item.assetId)).toEqual(["vti"]);
    expect(partition.collapsedValue).toBe(900);
  });
});
