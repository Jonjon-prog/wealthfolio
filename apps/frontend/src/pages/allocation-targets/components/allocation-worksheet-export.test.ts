import { describe, expect, it } from "vitest";

import type { AllocationWorksheetResult, CalculatedAdjustments } from "@/lib/types";

import {
  toCsv,
  toTsv,
  worksheetExportRows,
  type WorksheetExportLabels,
} from "./allocation-worksheet-export";

const labels: WorksheetExportLabels = {
  title: "Rebalancing worksheet",
  target: "Target",
  calculatedAt: "Calculated at",
  accounts: "Accounts to change",
  mode: "Mode",
  modeValue: "Rebalance",
  rule: "Allocation rule",
  ruleValue: "Allocate by current holding proportions",
  trackedCash: "Recorded cash used",
  externalCash: "Cash not yet recorded",
  eligible: "Eligible securities",
  eligibleValue: "All recorded securities",
  scaling: [],
  status: "Status",
  category: "Category",
  direction: "Direction",
  security: "Security",
  account: "Account",
  amount: "Amount",
  quantity: "Estimated quantity",
  price: "Unit price",
  priceDate: "Price date",
  warnings: "Warnings",
  statusAdjustment: "Adjustment",
  statusUnresolved: "Unresolved",
  increase: "Increase",
  reduce: "Reduce",
  unknownAccount: "Unknown account",
  limitationsTitle: "About this preview",
  limitations: "Nothing is submitted or executed.",
};

const quote = {
  id: "q",
  sourceType: "quote",
  value: 100,
  fromCurrency: "USD",
  toCurrency: "USD",
  timestamp: "2026-09-20T16:00:00Z",
  isStale: false,
};

const result = {
  targetName: "Balanced",
  baseCurrency: "USD",
  calculatedAt: "2026-09-24T10:00:00Z",
  lines: [
    {
      lineId: "l1",
      direction: "increase",
      assetId: "vbiax",
      accountId: "acc-1",
      symbol: "VBIAX",
      name: "Balanced Index",
      estimatedAmount: 1200,
      quantity: 12,
      unitPrice: 100,
      quoteSource: quote,
      categoryExposures: [
        { categoryId: "us", categoryName: "US equity", weightBps: 6000 },
        { categoryId: "bond", categoryName: "Bonds", weightBps: 4000 },
      ],
    },
    {
      lineId: "l2",
      direction: "reduce",
      assetId: "bnd",
      accountId: "acc-2",
      symbol: "BND",
      name: "=HYPERLINK(evil)",
      estimatedAmount: 300.004,
      quantity: 4,
      unitPrice: 75,
      quoteSource: quote,
      categoryExposures: [{ categoryId: "bond", categoryName: "Bonds", weightBps: 10000 }],
    },
  ],
  warnings: [{ id: "w1", kind: "stale_quote", lineId: "l2", message: "BND quote is dated." }],
} as unknown as AllocationWorksheetResult;

const calculated = {
  unresolved: [
    { categoryId: "gold", categoryName: "Gold", amount: 50, reason: "no_eligible_security" },
  ],
} as unknown as CalculatedAdjustments;

function rows(overrides: Partial<WorksheetExportLabels> = {}) {
  return worksheetExportRows(
    {
      result,
      calculated,
      accountNames: new Map([
        ["acc-1", "Brokerage"],
        ["acc-2", "Retirement"],
      ]),
      accountIds: ["acc-1", "acc-2"],
      trackedCashToUse: 900,
      externalCash: 300,
    },
    { ...labels, ...overrides },
  );
}

describe("worksheet export", () => {
  it("describes what the worksheet was calculated from before its lines", () => {
    const table = rows({ scaling: ["Increases were scaled to 80% to fit the available funding."] });
    expect(table.slice(0, 10)).toEqual([
      ["Rebalancing worksheet"],
      ["Target", "Balanced"],
      ["Calculated at", "2026-09-24T10:00:00Z"],
      ["Accounts to change", "Brokerage, Retirement"],
      ["Mode", "Rebalance"],
      ["Allocation rule", "Allocate by current holding proportions"],
      ["Recorded cash used (USD)", 900],
      ["Cash not yet recorded (USD)", 300],
      ["Eligible securities", "All recorded securities"],
      ["Increases were scaled to 80% to fit the available funding."],
    ]);
  });

  it("says in the file when inputs changed after the calculation", () => {
    expect(rows({ outOfDate: "Inputs changed after this calculation." })).toContainEqual([
      "Inputs changed after this calculation.",
    ]);
    expect(rows().flat()).not.toContain("Inputs changed after this calculation.");
  });

  it("signs amounts and quantities, and gives a mixed fund both its classes", () => {
    const table = rows();
    expect(table).toContainEqual([
      "Adjustment",
      "US equity 60% · Bonds 40%",
      "Increase",
      "VBIAX — Balanced Index",
      "Brokerage",
      1200,
      12,
      100,
      "2026-09-20",
      "",
    ]);
    expect(table).toContainEqual([
      "Adjustment",
      "Bonds",
      "Reduce",
      "BND — =HYPERLINK(evil)",
      "Retirement",
      -300,
      -4,
      75,
      "2026-09-20",
      "BND quote is dated.",
    ]);
  });

  it("keeps unresolved amounts in the table, with no security or account", () => {
    expect(rows()).toContainEqual(["Unresolved", "Gold", "", "", "", 50]);
  });

  it("ends with the warnings and the limitations", () => {
    expect(rows().slice(-6)).toEqual([
      [],
      ["Warnings"],
      ["BND quote is dated."],
      [],
      ["About this preview"],
      ["Nothing is submitted or executed."],
    ]);
  });

  it("keeps a text a spreadsheet would run as a formula as text, in both formats", () => {
    const table = [["=SUM(A1)", -300, "plain"]];
    expect(toCsv(table)).toBe(`"'=SUM(A1)","-300","plain"`);
    expect(toTsv(table)).toBe("'=SUM(A1)\t-300\tplain");
    expect(toCsv([['say "hi"']])).toBe(`"say ""hi"""`);
    expect(toTsv([["two\tcells\nhere"]])).toBe("two cells here");
  });
});
