import { buildAccountSelection } from "@/adapters";
import { usePortfolios } from "@/hooks/use-portfolios";
import { cn } from "@/lib/utils";
import type { Account } from "@/lib/types";
import {
  Badge,
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  Icons,
  Separator,
} from "@wealthfolio/ui";
import { Popover, PopoverContent, PopoverTrigger } from "@wealthfolio/ui/components/ui/popover";
import { useMemo } from "react";

interface ActivityAccountFilterProps {
  accounts: Account[];
  selectedAccountIds: string[];
  onAccountIdsChange: (ids: string[]) => void;
}

export function ActivityAccountFilter({
  accounts,
  selectedAccountIds,
  onAccountIdsChange,
}: ActivityAccountFilterProps) {
  const { data: portfolios = [] } = usePortfolios();
  const selected = useMemo(() => new Set(selectedAccountIds), [selectedAccountIds]);

  const isPortfolioSelected = (accountIds: string[]) =>
    accountIds.length > 0 && accountIds.every((id) => selected.has(id));

  const togglePortfolio = (accountIds: string[]) => {
    if (isPortfolioSelected(accountIds)) {
      // Deselect: remove all portfolio accounts
      onAccountIdsChange(selectedAccountIds.filter((id) => !accountIds.includes(id)));
    } else {
      // Select: add all portfolio accounts
      const merged = Array.from(new Set([...selectedAccountIds, ...accountIds]));
      onAccountIdsChange(merged);
    }
  };

  const toggleAccount = (accountId: string) => {
    if (selected.has(accountId)) {
      onAccountIdsChange(selectedAccountIds.filter((id) => id !== accountId));
    } else {
      onAccountIdsChange([...selectedAccountIds, accountId]);
    }
  };

  // Labels for the trigger badge
  const selectedLabels = useMemo(() => {
    const labels: string[] = [];
    portfolios.forEach((p) => {
      if (isPortfolioSelected(p.accountIds)) labels.push(p.name);
    });
    accounts.forEach((a) => {
      if (selected.has(a.id)) {
        // Only show individually if not already covered by a portfolio badge
        const inPortfolio = portfolios.some(
          (p) => isPortfolioSelected(p.accountIds) && p.accountIds.includes(a.id),
        );
        if (!inPortfolio) labels.push(a.name);
      }
    });
    return labels;
  }, [selectedAccountIds, portfolios, accounts]);

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          className={cn(
            'hover:bg-muted/80" bg-secondary/30 h-8 gap-1.5 rounded-md border-[1.5px] border-none px-3 py-1 text-sm font-medium',
            selected.size > 0 ? "bg-muted/40" : "shadow-inner-xs bg-muted/90",
          )}
        >
          <Icons.PlusCircle className="mr-2 h-4 w-4" />
          Account
          {selectedLabels.length > 0 && (
            <>
              <Separator orientation="vertical" className="mx-2 h-4" />
              <Badge variant="secondary" className="rounded-sm px-1 font-normal lg:hidden">
                {selectedLabels.length}
              </Badge>
              <div className="hidden space-x-1 lg:flex">
                {selectedLabels.length > 2 ? (
                  <Badge variant="secondary" className="text-foreground rounded-sm px-1 font-normal">
                    {selectedLabels.length} selected
                  </Badge>
                ) : (
                  selectedLabels.map((label) => (
                    <Badge
                      key={label}
                      variant="secondary"
                      className="text-foreground rounded-sm px-1 font-normal"
                    >
                      {label}
                    </Badge>
                  ))
                )}
              </div>
            </>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[220px] p-0" align="start">
        <Command>
          <CommandInput placeholder="Account" />
          <CommandList>
            <CommandEmpty>No results found.</CommandEmpty>

            {portfolios.length > 0 && (
              <>
                <CommandGroup heading="Portfolios">
                  {portfolios.map((portfolio) => {
                    const compositeId = buildAccountSelection(portfolio.accountIds);
                    const checked = isPortfolioSelected(portfolio.accountIds);
                    return (
                      <CommandItem
                        key={compositeId}
                        onSelect={() => togglePortfolio(portfolio.accountIds)}
                      >
                        <div
                          className={cn(
                            "border-primary mr-2 flex h-4 w-4 items-center justify-center rounded-sm border",
                            checked
                              ? "bg-primary text-primary-foreground"
                              : "opacity-50 [&_svg]:invisible",
                          )}
                        >
                          <Icons.Check className="h-4 w-4" />
                        </div>
                        <Icons.Briefcase className="text-muted-foreground mr-2 h-4 w-4" />
                        <span>{portfolio.name}</span>
                      </CommandItem>
                    );
                  })}
                </CommandGroup>
                <CommandSeparator />
              </>
            )}

            <CommandGroup heading="Accounts">
              {accounts.map((account) => {
                const checked = selected.has(account.id);
                return (
                  <CommandItem key={account.id} onSelect={() => toggleAccount(account.id)}>
                    <div
                      className={cn(
                        "border-primary mr-2 flex h-4 w-4 items-center justify-center rounded-sm border",
                        checked
                          ? "bg-primary text-primary-foreground"
                          : "opacity-50 [&_svg]:invisible",
                      )}
                    >
                      <Icons.Check className="h-4 w-4" />
                    </div>
                    <span>{account.name}</span>
                  </CommandItem>
                );
              })}
            </CommandGroup>

            {selected.size > 0 && (
              <>
                <CommandSeparator />
                <CommandGroup>
                  <CommandItem
                    onSelect={() => onAccountIdsChange([])}
                    className="text-destructive hover:bg-destructive/10 justify-center text-center text-sm"
                  >
                    Clear filters
                  </CommandItem>
                </CommandGroup>
              </>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
