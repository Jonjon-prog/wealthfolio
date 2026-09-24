import { Button, Icons, useAmountFormatting, useNumberFormatting } from "@wealthfolio/ui";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { Account, WorksheetAccountFunding } from "@/lib/types";
import { cn } from "@/lib/utils";

import {
  allocationProgress,
  AMOUNT_EPSILON,
  decimalInputOrZero,
  formatDecimalInput,
  placementAccountIds,
  type PositionAdjustment,
  type WorksheetPosition,
} from "./allocation-worksheet-utils";

interface AccountAllocationProps {
  position: WorksheetPosition;
  changeAmount: number;
  accounts: Account[];
  adjustment: PositionAdjustment;
  currency: string;
  fundingByAccount: Map<string, WorksheetAccountFunding>;
  /** Resolved in the base currency, when the position records one. */
  unitPrice: number | undefined;
  wholeSharesOnly: boolean;
  /** The sole eligible account recording this security, which takes the change until the user splits it (§6). */
  impliedAccountId: string | undefined;
  onAmountChange: (accountId: string, value: string) => void;
}

/**
 * Where a change sits (§6). A single eligible account takes the whole change;
 * with several, the user places it and nothing is assigned by default.
 */
export function AccountAllocation({
  position,
  changeAmount,
  accounts,
  adjustment,
  currency,
  fundingByAccount,
  unitPrice,
  wholeSharesOnly,
  impliedAccountId,
  onAmountChange,
}: AccountAllocationProps) {
  const { t } = useTranslation();
  const { formatAmount } = useAmountFormatting();
  const { formatQuantity } = useNumberFormatting();
  const [showOtherAccounts, setShowOtherAccounts] = useState(false);
  const requested = Math.abs(changeAmount);
  const hasEnteredAmount = accounts.some(
    (account) => (adjustment.accountAmounts[account.id] ?? "").trim() !== "",
  );
  const impliedHolder = hasEnteredAmount ? undefined : impliedAccountId;
  const amountFor = (accountId: string) =>
    impliedHolder === accountId
      ? requested
      : Math.max(0, decimalInputOrZero(adjustment.accountAmounts[accountId] ?? ""));
  const assigned =
    accounts.length === 1
      ? requested
      : accounts.reduce((sum, account) => sum + amountFor(account.id), 0);
  const { remaining, overallocated, isFullyAllocated } = allocationProgress(
    requested,
    assigned,
    AMOUNT_EPSILON,
  );
  const isReduce = changeAmount < 0;
  const { shown, hidden } = placementAccountIds(
    accounts.map((account) => account.id),
    position.accountHoldings
      .filter((holding) => holding.quantity > 0)
      .map((holding) => holding.accountId),
    accounts
      .filter((account) => (adjustment.accountAmounts[account.id] ?? "").trim() !== "")
      .map((account) => account.id),
  );
  const listedAccounts = showOtherAccounts
    ? accounts
    : accounts.filter((account) => shown.includes(account.id));
  // Which accounts record the security is a fact about the portfolio, so it is
  // stated whether or not the user has since placed the change by hand. Only
  // the first sentence — that the app placed it — depends on that.
  const holderName = accounts.find((account) => account.id === impliedAccountId)?.name;
  const hint = isReduce
    ? "allocation:worksheet.reductionAccountAllocationHint"
    : accounts.length === 1
      ? "allocation:worksheet.singleAccountAllocationHint"
      : impliedHolder
        ? "allocation:worksheet.soleHolderAllocationHint"
        : impliedAccountId
          ? "allocation:worksheet.soleHolderSplitHint"
          : "allocation:worksheet.increaseAccountAllocationHint";

  return (
    <div
      data-account-allocation
      className="border-border/60 bg-muted/20 mt-3 rounded-xl border p-4"
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div>
          <p className="font-mono text-[11px] font-medium uppercase tracking-[0.12em]">
            {t("allocation:worksheet.accountAllocation")}
          </p>
          <p className="text-muted-foreground mt-1 text-xs">{t(hint, { account: holderName })}</p>
        </div>
        <span
          className={cn(
            "rounded-full px-2.5 py-1 font-mono text-[11px]",
            isFullyAllocated
              ? "bg-emerald-100 text-emerald-900 dark:bg-emerald-950/35 dark:text-emerald-200"
              : overallocated > AMOUNT_EPSILON
                ? "bg-red-100 text-red-900 dark:bg-red-950/35 dark:text-red-200"
                : "bg-amber-100 text-amber-900 dark:bg-amber-950/35 dark:text-amber-200",
          )}
        >
          {isFullyAllocated
            ? t("allocation:worksheet.fullyAllocated")
            : overallocated > AMOUNT_EPSILON
              ? t("allocation:worksheet.overAllocatedBy", {
                  amount: formatAmount(overallocated, currency),
                })
              : t("allocation:worksheet.remainingToAllocate", {
                  amount: formatAmount(remaining, currency),
                })}
        </span>
      </div>

      <div className="mt-3 divide-y">
        {accounts.length === 0 && (
          <p className="text-muted-foreground py-3 text-xs leading-relaxed">
            {t("allocation:worksheet.noEligibleAccounts")}
          </p>
        )}
        {listedAccounts.map((account) => {
          const holding = position.accountHoldings.find((item) => item.accountId === account.id);
          const funding = fundingByAccount.get(account.id);
          const currentAmount = amountFor(account.id);
          const rowRemaining = Math.max(0, requested - (assigned - currentAmount));
          // The unit price is derived from the recorded holding rather than the
          // quote the core resolves against, so the floor gets a tolerance and
          // never drops a unit over the last decimal. A remainder that buys
          // nothing is still offered: the core reports such a line now, and
          // withholding the button only forces the same amount in by hand.
          const wholeUnitRemaining =
            wholeSharesOnly && unitPrice
              ? Math.floor(rowRemaining / unitPrice + 1e-9) * unitPrice
              : rowRemaining;
          const remainingToUse =
            wholeUnitRemaining > AMOUNT_EPSILON ? wholeUnitRemaining : rowRemaining;
          const currentUnits = unitPrice ? currentAmount / unitPrice : undefined;
          return (
            <div
              key={account.id}
              className="grid gap-2 py-3 first:pt-0 last:pb-0 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center"
            >
              <div className="min-w-0">
                <p className="flex min-w-0 items-center gap-1.5 text-xs font-medium">
                  <span className="truncate">{account.name}</span>
                  {impliedHolder === account.id && (
                    <span className="text-muted-foreground inline-flex shrink-0 items-center gap-1 rounded-full border px-1.5 py-0.5 text-[10px] font-normal">
                      <Icons.Check className="h-2.5 w-2.5" />
                      {t("allocation:worksheet.holdsThisSecurity")}
                    </span>
                  )}
                </p>
                <p className="text-muted-foreground mt-0.5 text-[11px]">
                  {holding &&
                    t("allocation:worksheet.accountHoldingSummary", {
                      amount: formatAmount(holding.value, currency),
                      quantity: formatQuantity(holding.quantity),
                    })}
                  {holding && !isReduce && " · "}
                  {!isReduce &&
                    t("allocation:worksheet.accountCashSummary", {
                      amount: formatAmount(funding?.availableCash ?? 0, currency),
                    })}
                </p>
                {funding && funding.remaining < -AMOUNT_EPSILON && (
                  <p className="mt-0.5 text-[11px] font-medium text-amber-800 dark:text-amber-200">
                    {t("allocation:worksheet.fundingNeeded", {
                      amount: formatAmount(-funding.remaining, currency),
                    })}
                  </p>
                )}
              </div>
              <div className="flex flex-col items-end gap-1">
                <div className="flex items-center gap-2">
                  {accounts.length === 1 ? (
                    <span className="font-mono text-sm font-semibold tabular-nums">
                      {formatAmount(requested, currency)}
                    </span>
                  ) : (
                    <>
                      <div className="border-input bg-background focus-within:ring-ring flex h-9 w-40 items-center rounded-md border px-2.5 focus-within:ring-1">
                        <span className="text-muted-foreground mr-1.5 text-xs">{currency}</span>
                        <input
                          aria-label={t("allocation:worksheet.accountAmountLabel", {
                            account: account.name,
                          })}
                          value={
                            impliedHolder === account.id
                              ? formatDecimalInput(requested, 6)
                              : (adjustment.accountAmounts[account.id] ?? "")
                          }
                          onChange={(event) => onAmountChange(account.id, event.target.value)}
                          inputMode="decimal"
                          placeholder="0"
                          className="min-w-0 flex-1 bg-transparent text-right font-mono text-xs outline-none"
                        />
                      </div>
                      {rowRemaining > AMOUNT_EPSILON && (
                        <Button
                          size="sm"
                          variant="ghost"
                          className="h-8 px-2 text-[11px]"
                          onClick={() =>
                            onAmountChange(account.id, formatDecimalInput(remainingToUse, 6))
                          }
                        >
                          {t("allocation:worksheet.useRemaining")}
                        </Button>
                      )}
                    </>
                  )}
                </div>
                {currentUnits !== undefined && currentAmount > AMOUNT_EPSILON && (
                  <span className="text-muted-foreground font-mono text-[10px] tabular-nums">
                    {t("allocation:worksheet.accountUnitsSummary", {
                      quantity: formatQuantity(
                        wholeSharesOnly ? Math.floor(currentUnits) : currentUnits,
                      ),
                    })}
                  </span>
                )}
              </div>
            </div>
          );
        })}
      </div>
      {/* Any other account can still receive the change; it opens a new position there. */}
      {!showOtherAccounts && hidden.length > 0 && (
        <button
          type="button"
          onClick={() => setShowOtherAccounts(true)}
          className="text-foreground mt-3 inline-flex items-center gap-1 text-xs underline-offset-4 hover:underline"
        >
          <Icons.Plus className="h-3.5 w-3.5" />
          {t("allocation:worksheet.placeInAnotherAccount")}
        </button>
      )}
    </div>
  );
}
