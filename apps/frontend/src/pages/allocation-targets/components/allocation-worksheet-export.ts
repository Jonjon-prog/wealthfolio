import type {
  AllocationWorksheetLineResult,
  AllocationWorksheetResult,
  CalculatedAdjustments,
} from "@/lib/types";

export type ExportCell = string | number;

/** Every text the table carries, already translated by the caller. */
export interface WorksheetExportLabels {
  title: string;
  target: string;
  calculatedAt: string;
  accounts: string;
  mode: string;
  modeValue: string;
  rule: string;
  ruleValue: string;
  trackedCash: string;
  externalCash: string;
  eligible: string;
  eligibleValue: string;
  /** Sentences saying which amounts the calculation scaled, if any. */
  scaling: readonly string[];
  /** Said in the header when inputs changed after the calculation. */
  outOfDate?: string;
  status: string;
  category: string;
  direction: string;
  security: string;
  account: string;
  amount: string;
  quantity: string;
  price: string;
  priceDate: string;
  warnings: string;
  statusAdjustment: string;
  statusUnresolved: string;
  increase: string;
  reduce: string;
  unknownAccount: string;
  limitationsTitle: string;
  limitations: string;
}

export interface WorksheetExportInput {
  result: AllocationWorksheetResult;
  /** The last calculation, for the amounts it could not place. */
  calculated: CalculatedAdjustments | null;
  accountNames: ReadonlyMap<string, string>;
  accountIds: readonly string[];
  trackedCashToUse: number;
  externalCash: number;
}

function lineCategory(line: AllocationWorksheetLineResult): string {
  const exposures = line.categoryExposures.filter((exposure) => exposure.weightBps > 0);
  if (exposures.length === 1 && exposures[0].weightBps >= 10_000) {
    return exposures[0].categoryName;
  }
  return exposures
    .map((exposure) => `${exposure.categoryName} ${Math.round(exposure.weightBps / 100)}%`)
    .join(" · ");
}

function round(value: number, digits: number): number {
  return Number(value.toFixed(digits));
}

/**
 * The export table (§8): a header describing what the worksheet was calculated
 * from, one row per security and account, the amounts the calculation could
 * not place, then the warnings and the limitations. Amounts are signed and
 * come first; quantities are estimates. Nothing reads like an order ticket.
 */
export function worksheetExportRows(
  input: WorksheetExportInput,
  labels: WorksheetExportLabels,
): ExportCell[][] {
  const { result, calculated } = input;
  const currency = result.baseCurrency;
  const warningsByLine = new Map<string, string[]>();
  for (const warning of result.warnings) {
    if (!warning.lineId) continue;
    warningsByLine.set(warning.lineId, [
      ...(warningsByLine.get(warning.lineId) ?? []),
      warning.message,
    ]);
  }

  const rows: ExportCell[][] = [
    [labels.title],
    [labels.target, result.targetName],
    [labels.calculatedAt, result.calculatedAt],
    [
      labels.accounts,
      input.accountIds
        .map((accountId) => input.accountNames.get(accountId) ?? labels.unknownAccount)
        .join(", "),
    ],
    [labels.mode, labels.modeValue],
    [labels.rule, labels.ruleValue],
    [`${labels.trackedCash} (${currency})`, round(input.trackedCashToUse, 2)],
    [`${labels.externalCash} (${currency})`, round(input.externalCash, 2)],
    [labels.eligible, labels.eligibleValue],
    ...labels.scaling.map((sentence) => [sentence]),
    ...(labels.outOfDate ? [[labels.outOfDate]] : []),
    [],
    [
      labels.status,
      labels.category,
      labels.direction,
      labels.security,
      labels.account,
      `${labels.amount} (${currency})`,
      labels.quantity,
      `${labels.price} (${currency})`,
      labels.priceDate,
      labels.warnings,
    ],
  ];

  for (const line of result.lines) {
    const signed = line.direction === "increase" ? line.estimatedAmount : -line.estimatedAmount;
    rows.push([
      labels.statusAdjustment,
      lineCategory(line),
      line.direction === "increase" ? labels.increase : labels.reduce,
      `${line.symbol} — ${line.name}`,
      input.accountNames.get(line.accountId) ?? labels.unknownAccount,
      round(signed, 2),
      round(line.direction === "increase" ? line.quantity : -line.quantity, 6),
      round(line.unitPrice, 6),
      line.quoteSource.timestamp.slice(0, 10),
      (warningsByLine.get(line.lineId) ?? []).join("; "),
    ]);
  }
  // Part of the picture, so part of the file rather than only on screen.
  for (const item of calculated?.unresolved ?? []) {
    rows.push([labels.statusUnresolved, item.categoryName, "", "", "", round(item.amount, 2)]);
  }

  if (result.warnings.length > 0) {
    rows.push([], [labels.warnings], ...result.warnings.map((warning) => [warning.message]));
  }
  rows.push([], [labels.limitationsTitle], [labels.limitations]);
  return rows;
}

/** A text a spreadsheet would run as a formula is kept as text. */
const FORMULA_PREFIX = /^[=+\-@\t\r]/;

function textCell(value: ExportCell): string {
  const text = String(value);
  return typeof value === "string" && FORMULA_PREFIX.test(text) ? `'${text}` : text;
}

export function toCsv(rows: readonly ExportCell[][]): string {
  return rows
    .map((row) => row.map((cell) => `"${textCell(cell).replaceAll('"', '""')}"`).join(","))
    .join("\n");
}

/** The same table for the clipboard, so it pastes into a spreadsheet as cells. */
export function toTsv(rows: readonly ExportCell[][]): string {
  return rows
    .map((row) => row.map((cell) => textCell(cell).replace(/[\t\r\n]+/g, " ")).join("\t"))
    .join("\n");
}

/** Uses a Blob link, which works in both the web and Tauri builds. */
export function downloadCsv(csv: string, date: string) {
  // The byte order mark lets spreadsheet apps read accented names as UTF-8.
  const blob = new Blob([`﻿${csv}`], { type: "text/csv;charset=utf-8;" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `rebalancing-worksheet-${date}.csv`;
  document.body.appendChild(link);
  link.click();
  document.body.removeChild(link);
  URL.revokeObjectURL(url);
}
