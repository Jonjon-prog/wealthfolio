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
  note: "Note",
  status: "Status",
  category: "Category",
  direction: "Direction",
  symbol: "Symbol",
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
  total: "Total",
  cashLeft: "Cash remaining",
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

function line(overrides: Record<string, unknown>) {
  return {
    lineId: "line",
    direction: "increase",
    accountId: "acc-1",
    quantity: 1,
    unitPrice: 100,
    estimatedAmount: 100,
    quoteSource: quote,
    categoryExposures: [{ categoryId: "us", categoryName: "US equity", weightBps: 10000 }],
    ...overrides,
  };
}

// Lines arrive in the order the calculation produced them, accounts mixed.
const result = {
  targetName: "Balanced",
  baseCurrency: "USD",
  calculatedAt: "2026-09-24T10:00:00Z",
  lines: [
    line({
      lineId: "l1",
      assetId: "vbiax",
      accountId: "acc-1",
      symbol: "VBIAX",
      name: "Balanced Index",
      estimatedAmount: 1200,
      quantity: 12,
      // Stored as a 32-bit float and widened.
      unitPrice: 99.999998,
      categoryExposures: [
        { categoryId: "us", categoryName: "US equity", weightBps: 6000 },
        { categoryId: "bond", categoryName: "Bonds", weightBps: 4000 },
      ],
    }),
    line({
      lineId: "l2",
      direction: "reduce",
      assetId: "bnd",
      accountId: "acc-2",
      symbol: "BND",
      name: "=HYPERLINK(evil)",
      estimatedAmount: 300.004,
      quantity: 4,
      unitPrice: 75,
      categoryExposures: [{ categoryId: "bond", categoryName: "Bonds", weightBps: 10000 }],
    }),
    line({
      lineId: "l3",
      assetId: "aapl",
      accountId: "acc-1",
      symbol: "AAPL",
      name: "Apple",
      estimatedAmount: 200,
      quantity: 1,
      unitPrice: 200,
    }),
  ],
  accountFunding: [
    { accountId: "acc-1", remaining: -100 },
    { accountId: "acc-2", remaining: 300 },
  ],
  warnings: [{ id: "w1", kind: "stale_quote", lineId: "l2", message: "BND quote is dated." }],
} as unknown as AllocationWorksheetResult;

const calculated = {
  unresolved: [
    { categoryId: "gold", categoryName: "Gold", amount: 50, reason: "no_eligible_security" },
  ],
} as unknown as CalculatedAdjustments;

function csvLines(overrides: Partial<WorksheetExportLabels> = {}) {
  const rows = worksheetExportRows(
    {
      result,
      calculated,
      accountNames: new Map([
        ["acc-1", "Brokerage"],
        ["acc-2", "Retirement"],
      ]),
      accountIds: ["acc-1", "acc-2"],
      trackedCashToUse: 900,
      externalCash: 300.5,
    },
    { ...labels, ...overrides },
  );
  return toCsv(rows).split("\n");
}

describe("worksheet export", () => {
  it("describes what the worksheet was calculated from as label, value pairs", () => {
    const lines = csvLines({
      scaling: ["Increases were scaled to 80% to fit the available funding."],
      outOfDate: "Inputs changed after this calculation.",
    });

    expect(lines[0]).toBe(`"Rebalancing worksheet"`);
    expect(lines[1]).toBe(`"Target","Balanced"`);
    // Local time to the minute, not the raw timestamp.
    expect(lines[2]).toMatch(/^"Calculated at","2026-09-2\d \d{2}:\d{2}"$/);
    expect(lines.slice(3, 12)).toEqual([
      `"Accounts to change","Brokerage, Retirement"`,
      `"Mode","Rebalance"`,
      `"Allocation rule","Allocate by current holding proportions"`,
      `"Eligible securities","All recorded securities"`,
      `"Recorded cash used (USD)","900.00"`,
      `"Cash not yet recorded (USD)","300.50"`,
      `"Note","Increases were scaled to 80% to fit the available funding."`,
      `"Note","Inputs changed after this calculation."`,
      "",
    ]);
  });

  it("lists the lines by account, then by symbol, as the review does", () => {
    const lines = csvLines();
    const header = lines.indexOf(
      `"Account","Symbol","Security","Direction","Amount (USD)","Estimated quantity","Unit price (USD)","Price date","Category","Status","Warnings"`,
    );

    expect(header).toBeGreaterThan(0);
    expect(lines.slice(header + 1, header + 4)).toEqual([
      `"Brokerage","AAPL","Apple","Increase","200.00","1","200","2026-09-20","US equity","Adjustment",""`,
      // A mixed fund carries both its classes; the stored price loses its float residue.
      `"Brokerage","VBIAX","Balanced Index","Increase","1200.00","12","100","2026-09-20","US equity 60% · Bonds 40%","Adjustment",""`,
      // Signed amount and quantity; a name a spreadsheet would run stays text.
      `"Retirement","BND","'=HYPERLINK(evil)","Reduce","-300.00","-4","75","2026-09-20","Bonds","Adjustment","BND quote is dated."`,
    ]);
  });

  it("keeps unresolved amounts in the table, with no account or security", () => {
    expect(csvLines()).toContain(`"","","","","50.00","","","","Gold","Unresolved",""`);
  });

  it("totals each account apart from the lines, with the cash it has left", () => {
    const lines = csvLines();
    const header = lines.indexOf(`"Account","Total (USD)","Cash remaining (USD)"`);

    expect(header).toBeGreaterThan(0);
    expect(lines.slice(header + 1, header + 3)).toEqual([
      `"Brokerage","1400.00","-100.00"`,
      `"Retirement","-300.00","300.00"`,
    ]);
  });

  it("ends with the warnings and the limitations", () => {
    expect(csvLines().slice(-6)).toEqual([
      "",
      `"Warnings"`,
      `"BND quote is dated."`,
      "",
      `"About this preview"`,
      `"Nothing is submitted or executed."`,
    ]);
  });

  it("keeps a text a spreadsheet would run as a formula as text, in both formats", () => {
    const table = [["=SUM(A1)", -300, "plain", { decimal: -300, digits: 2 }]];
    expect(toCsv(table)).toBe(`"'=SUM(A1)","-300","plain","-300.00"`);
    expect(toTsv(table)).toBe("'=SUM(A1)\t-300\tplain\t-300.00");
    expect(toCsv([['say "hi"']])).toBe(`"say ""hi"""`);
    expect(toTsv([["two\tcells\nhere"]])).toBe("two cells here");
  });
});
