import {
  Button,
  Icons,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
  useAmountFormatting,
} from "@wealthfolio/ui";
import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import type { Account, Asset, LatestQuoteSnapshot, WorksheetAccountFunding } from "@/lib/types";
import { cn } from "@/lib/utils";

import { AccountAllocation } from "./allocation-worksheet-account-allocation";
import {
  activeTarget,
  partialShareBps,
  partitionAmountsRows,
  rowEmphasis,
  type HighlightTarget,
} from "./allocation-worksheet-amounts";
import { useHighlight, useHighlightActions } from "./allocation-worksheet-highlight";
import {
  AMOUNT_EPSILON,
  decimalInputOrZero,
  formatDecimalInput,
  formatSignedAmount,
  type PositionAdjustment,
  type WorksheetEditMode,
  type WorksheetPosition,
} from "./allocation-worksheet-utils";

/** One security's line in the Amounts panel, as the worksheet resolves it. */
export interface AmountsRowModel {
  position: WorksheetPosition;
  adjustment: PositionAdjustment | undefined;
  displayInput: string;
  /** The entered change, or zero while it cannot be read. */
  changeAmount: number;
  /** A change is entered, including one that cannot be read yet. */
  isChanged: boolean;
  projectedValue: number;
  resolved: { amount: number; quantity: number } | undefined;
  quote: LatestQuoteSnapshot | undefined;
  asset: Asset | undefined;
  isExpanded: boolean;
  /** Where the change may be placed, while there is one. */
  placement:
    | { accounts: Account[]; impliedAccountId: string | undefined; unitPrice: number | undefined }
    | undefined;
}

export interface AmountsRowActions {
  onInputChange: (position: WorksheetPosition, value: string) => void;
  onReduceToZero: (position: WorksheetPosition) => void;
  onRemove: (assetId: string) => void;
  onToggleExpanded: (assetId: string) => void;
  onAccountAmountChange: (assetId: string, accountId: string, value: string) => void;
  onPriceAction: (assetId: string, asset: Asset | undefined) => void;
}

interface AmountsListProps {
  rows: readonly AmountsRowModel[];
  /** Rows the worksheet or the preview has something to say about. */
  flaggedAssetIds: ReadonlySet<string>;
  /** Classes the calculation left an amount unresolved in. */
  unresolvedCategoryIds: ReadonlySet<string>;
  editMode: WorksheetEditMode;
  currency: string;
  allowSells: boolean;
  wholeSharesOnly: boolean;
  fundingByAccount: Map<string, WorksheetAccountFunding>;
  isPriceSyncing: boolean;
  quotesFetched: boolean;
  actions: AmountsRowActions;
}

/**
 * The list of securities, opened short: rows with nothing to decide collapse
 * behind one counted line. The list is never filtered or reordered, and a row
 * touched during this visit stays out of the group until the list is left.
 */
export function AmountsList({
  rows,
  flaggedAssetIds,
  unresolvedCategoryIds,
  actions,
  ...shared
}: AmountsListProps) {
  const [touchedAssetIds, setTouchedAssetIds] = useState<ReadonlySet<string>>(() => new Set());
  const [showCollapsed, setShowCollapsed] = useState(false);
  const { reset } = useHighlightActions();
  // Leaving the list ends pointing and selection, so nothing stays lit in the rail.
  useEffect(() => reset, [reset]);

  const touch = (assetId: string) =>
    setTouchedAssetIds((current) =>
      current.has(assetId) ? current : new Set(current).add(assetId),
    );
  const rowActions: AmountsRowActions = {
    ...actions,
    onInputChange: (position, value) => {
      touch(position.assetId);
      actions.onInputChange(position, value);
    },
    onReduceToZero: (position) => {
      touch(position.assetId);
      actions.onReduceToZero(position);
    },
    onAccountAmountChange: (assetId, accountId, value) => {
      touch(assetId);
      actions.onAccountAmountChange(assetId, accountId, value);
    },
  };

  const rowByAsset = new Map(rows.map((row) => [row.position.assetId, row]));
  const { open, collapsed, collapsedValue } = partitionAmountsRows(
    rows.map((row) => row.position),
    {
      changedAssetIds: new Set(
        rows.filter((row) => row.isChanged).map((row) => row.position.assetId),
      ),
      flaggedAssetIds,
      unresolvedCategoryIds,
      touchedAssetIds,
    },
  );
  const renderRow = (position: WorksheetPosition) => (
    <AmountRow
      key={position.assetId}
      model={rowByAsset.get(position.assetId)!}
      actions={rowActions}
      {...shared}
    />
  );

  // One keyed list rather than three blocks: a row typed into from the open
  // group moves among its siblings instead of remounting, so it keeps focus.
  const items: ReactNode[] = open.map(renderRow);
  if (collapsed.length > 0) {
    items.push(
      <CollapsedLine
        key="collapsed-rows"
        rows={collapsed}
        value={collapsedValue}
        currency={shared.currency}
        isOpen={showCollapsed}
        onToggle={() => setShowCollapsed((current) => !current)}
      />,
    );
    if (showCollapsed) items.push(...collapsed.map(renderRow));
  }
  return <div className="divide-y">{items}</div>;
}

interface CollapsedLineProps {
  rows: readonly WorksheetPosition[];
  value: number;
  currency: string;
  isOpen: boolean;
  onToggle: () => void;
}

function CollapsedLine({ rows, value, currency, isOpen, onToggle }: CollapsedLineProps) {
  const { t } = useTranslation();
  const { formatAmount } = useAmountFormatting();
  const activeCategoryId = useHighlight((state) => {
    const active = activeTarget(state);
    return active?.kind === "category" ? active.categoryId : null;
  });
  const touching = activeCategoryId
    ? rows.filter((row) =>
        row.categoryExposures.some(
          (exposure) => exposure.categoryId === activeCategoryId && exposure.weightBps > 0,
        ),
      )
    : [];
  const categoryName = touching[0]?.categoryExposures.find(
    (exposure) => exposure.categoryId === activeCategoryId,
  )?.categoryName;

  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={isOpen}
      data-collapsed-rows
      className={cn(
        "flex w-full items-center justify-between gap-3 px-4 py-3 text-left sm:px-5",
        touching.length > 0 && "bg-muted/60",
      )}
    >
      <span className="flex min-w-0 items-center gap-2.5 text-xs">
        <Icons.ChevronDown
          className={cn("h-3.5 w-3.5 shrink-0 transition-transform", isOpen && "rotate-180")}
        />
        <span className="font-medium">
          {t("allocation:worksheet.collapsedRows", { count: rows.length })}
        </span>
        <span className="text-muted-foreground font-mono tabular-nums">
          {formatAmount(value, currency)}
        </span>
      </span>
      <span className={cn("shrink-0 text-xs", touching.length > 0 && "font-semibold")}>
        {touching.length > 0 && categoryName
          ? t("allocation:worksheet.collapsedTouching", {
              count: touching.length,
              category: categoryName,
            })
          : isOpen
            ? t("allocation:worksheet.hideRows")
            : t("allocation:worksheet.showRows")}
      </span>
    </button>
  );
}

interface AmountRowProps extends Omit<
  AmountsListProps,
  "rows" | "flaggedAssetIds" | "unresolvedCategoryIds"
> {
  model: AmountsRowModel;
}

function AmountRow({
  model,
  actions,
  editMode,
  currency,
  allowSells,
  wholeSharesOnly,
  fundingByAccount,
  isPriceSyncing,
  quotesFetched,
}: AmountRowProps) {
  const { t } = useTranslation();
  const { formatAmount } = useAmountFormatting();
  const {
    position,
    adjustment,
    displayInput,
    changeAmount,
    projectedValue,
    resolved,
    quote,
    asset,
    isExpanded,
    placement,
  } = model;
  const emphasis = useHighlight((state) =>
    rowEmphasis(activeTarget(state), position.assetId, position.categoryExposures),
  );
  const shareBps = useHighlight((state) =>
    partialShareBps(activeTarget(state), position.categoryExposures),
  );
  const isSelected = useHighlight(
    (state) => state.selected?.kind === "row" && state.selected.assetId === position.assetId,
  );
  const { point, unpoint, select, toggleSelected } = useHighlightActions();
  const target: HighlightTarget = { kind: "row", assetId: position.assetId };

  return (
    <div
      id={`worksheet-position-${position.assetId}`}
      data-amounts-row={position.assetId}
      data-emphasis={emphasis}
      aria-current={isSelected || undefined}
      onPointerEnter={(event) => event.pointerType !== "touch" && point(target)}
      onPointerLeave={(event) => event.pointerType !== "touch" && unpoint(target)}
      // Focus inside the row, in its amount field above all, behaves like pointing.
      onFocus={() => point(target)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) unpoint(target);
      }}
      onClick={(event) => {
        if ((event.target as HTMLElement).closest("input, button, a")) return;
        toggleSelected(target);
      }}
      className={cn(
        "cursor-pointer px-4 py-4 transition-[opacity,background-color] sm:px-5",
        (emphasis === "active" || emphasis === "lit") && "bg-muted/60",
        emphasis === "dim" && "opacity-40",
        isSelected && "ring-foreground ring-[1.5px] ring-inset",
      )}
    >
      <div className="grid gap-3 xl:grid-cols-[minmax(12rem,1.6fr)_7rem_4rem_8.5rem_8.5rem_2rem] xl:items-center">
        <div className="min-w-0">
          <div className="flex min-w-0 items-baseline gap-2">
            <span className="shrink-0 font-mono text-sm font-semibold">{position.symbol}</span>
            {shareBps !== null && (
              <span className="border-foreground shrink-0 rounded border px-1 font-mono text-[11px]">
                {Math.round(shareBps / 100)}%
              </span>
            )}
            <span className="text-muted-foreground truncate text-xs">{position.name}</span>
          </div>
          <p className="text-muted-foreground mt-1 truncate text-[11px]">
            {position.categoryNames.length > 0
              ? position.categoryNames.join(" · ")
              : t("allocation:worksheet.unclassified")}
            {position.accountHoldings.length > 0 &&
              ` · ${t("allocation:worksheet.accountCount", { count: position.accountHoldings.length })}`}
          </p>
          {resolved && (
            <p className="text-muted-foreground mt-1 font-mono text-[10px]">
              {t("allocation:worksheet.resolvedPositionSummary", {
                value: formatAmount(projectedValue, currency),
                quantity: `${resolved.quantity > 0 ? "+" : "−"}${formatDecimalInput(Math.abs(resolved.quantity), 6)}`,
              })}
            </p>
          )}
          {resolved &&
            wholeSharesOnly &&
            Math.abs(resolved.amount - changeAmount) >= AMOUNT_EPSILON && (
              <p className="mt-1 text-[10px] text-amber-800 dark:text-amber-200">
                {t("allocation:worksheet.roundedToWholeShares", {
                  amount: formatAmount(Math.abs(resolved.amount), currency),
                  requested: formatAmount(Math.abs(changeAmount), currency),
                })}
              </p>
            )}
        </div>

        <div className="flex items-center justify-between xl:block xl:text-right">
          <span className="text-muted-foreground text-[10px] uppercase xl:hidden">
            {t("allocation:worksheet.currentValue")}
          </span>
          <span className="font-mono text-xs tabular-nums">
            {formatAmount(position.value, currency)}
          </span>
        </div>
        <div className="flex items-center justify-between xl:block xl:text-right">
          <span className="text-muted-foreground text-[10px] uppercase xl:hidden">
            {t("allocation:worksheet.now")}
          </span>
          <span className="font-mono text-xs tabular-nums">{position.currentPct.toFixed(1)}%</span>
        </div>
        <div className="flex items-center justify-between gap-3 xl:justify-end">
          <span className="text-muted-foreground text-[10px] uppercase xl:hidden">
            {editMode === "amount"
              ? t("allocation:worksheet.changeAmount")
              : t("allocation:worksheet.projectedPercent")}
          </span>
          <div className="flex w-44 items-center gap-1 xl:w-full">
            <div className="border-input bg-background flex h-9 min-w-0 flex-1 items-center rounded-md border px-2.5 focus-within:border-[#557866] focus-within:ring-1 focus-within:ring-[#557866]/30">
              <span className="text-muted-foreground mr-1.5 text-xs">
                {editMode === "amount" ? currency : ""}
              </span>
              <input
                aria-label={t("allocation:worksheet.positionInputLabel", {
                  symbol: position.symbol,
                })}
                value={displayInput}
                onChange={(event) => actions.onInputChange(position, event.target.value)}
                // A tap on the amount field selects the row too, so its classes
                // stay lit while typing.
                onPointerDown={(event) => event.pointerType === "touch" && select(target)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    toggleSelected(target);
                    return;
                  }
                  if (
                    editMode !== "after_percentage" ||
                    (event.key !== "ArrowUp" && event.key !== "ArrowDown")
                  ) {
                    return;
                  }
                  event.preventDefault();
                  const current = decimalInputOrZero(displayInput);
                  const step = event.shiftKey ? 1 : 0.5;
                  const next = Math.min(
                    100,
                    Math.max(0, current + (event.key === "ArrowUp" ? step : -step)),
                  );
                  actions.onInputChange(position, formatDecimalInput(next, 4));
                }}
                inputMode="decimal"
                placeholder={editMode === "amount" ? "±0" : undefined}
                className="min-w-0 flex-1 bg-transparent text-right font-mono text-xs outline-none"
              />
              {editMode === "after_percentage" && (
                <span className="text-muted-foreground ml-1 text-xs">%</span>
              )}
            </div>
            {position.value > AMOUNT_EPSILON && allowSells && (
              <TooltipProvider delayDuration={150}>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="h-8 w-8 shrink-0"
                      disabled={projectedValue <= AMOUNT_EPSILON}
                      aria-label={t("allocation:worksheet.reducePositionToZero")}
                      onClick={() => actions.onReduceToZero(position)}
                    >
                      <Icons.MinusCircle className="h-4 w-4" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>{t("allocation:worksheet.reducePositionToZero")}</TooltipContent>
                </Tooltip>
              </TooltipProvider>
            )}
          </div>
        </div>
        <div className="flex items-center justify-between xl:block xl:text-right">
          <span className="text-muted-foreground text-[10px] uppercase xl:hidden">
            {t("allocation:worksheet.projectedChange")}
          </span>
          <span className="font-mono text-xs font-medium tabular-nums">
            {formatSignedAmount(changeAmount, currency, formatAmount)}
          </span>
        </div>
        <div className="flex items-center justify-end gap-1">
          {Math.abs(changeAmount) >= AMOUNT_EPSILON && (
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              aria-label={t("allocation:worksheet.toggleAccountAllocation")}
              onClick={() => actions.onToggleExpanded(position.assetId)}
            >
              <Icons.ChevronDown
                className={cn("h-4 w-4 transition-transform", isExpanded && "rotate-180")}
              />
            </Button>
          )}
          {position.isAdded && (
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              aria-label={t("allocation:worksheet.removePosition")}
              onClick={() => actions.onRemove(position.assetId)}
            >
              <Icons.X className="h-4 w-4" />
            </Button>
          )}
        </div>
      </div>

      {quote?.quote ? (
        <p className="text-muted-foreground mt-2 text-[10px]">
          {t("allocation:worksheet.priceSourceInline", {
            price: formatAmount(quote.quote.close, quote.quote.currency),
            date: quote.quoteDate ?? quote.quote.timestamp,
          })}
          {quote.isStale ? ` · ${t("allocation:worksheet.dated")}` : ""}
        </p>
      ) : position.isAdded && quotesFetched ? (
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <span className="text-destructive text-xs">{t("allocation:worksheet.noQuoteShort")}</span>
          <Button
            size="sm"
            variant="outline"
            className="h-7 px-2 text-xs"
            disabled={isPriceSyncing}
            onClick={() => actions.onPriceAction(position.assetId, asset)}
          >
            {asset?.quoteMode === "MARKET"
              ? t("allocation:worksheet.refreshPrice")
              : t("allocation:worksheet.addManualPrice")}
          </Button>
        </div>
      ) : null}

      {adjustment && isExpanded && placement && (
        <AccountAllocation
          position={position}
          changeAmount={changeAmount}
          accounts={placement.accounts}
          adjustment={adjustment}
          currency={currency}
          fundingByAccount={fundingByAccount}
          unitPrice={placement.unitPrice}
          wholeSharesOnly={wholeSharesOnly}
          impliedAccountId={placement.impliedAccountId}
          onAmountChange={(accountId, value) =>
            actions.onAccountAmountChange(position.assetId, accountId, value)
          }
        />
      )}
    </div>
  );
}
