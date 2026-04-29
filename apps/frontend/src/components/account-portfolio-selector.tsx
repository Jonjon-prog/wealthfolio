import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Icons,
} from "@wealthfolio/ui";
import { Button } from "@wealthfolio/ui/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@wealthfolio/ui/components/ui/popover";
import { Separator } from "@wealthfolio/ui/components/ui/separator";
import { cn } from "@/lib/utils";
import { PORTFOLIO_ACCOUNT_ID } from "@/lib/constants";
import { useAccounts } from "@/hooks/use-accounts";
import { usePortfolios } from "@/hooks/use-portfolios";
import { buildAccountSelection } from "@/adapters";
import { useState } from "react";

interface AccountPortfolioSelectorProps {
  value: string; // "PORTFOLIO", "MULTI:uuid,uuid", or account UUID
  onChange: (id: string, label: string) => void;
  className?: string;
}

function labelForSelection(
  value: string,
  accounts: { id: string; name: string }[],
  portfolios: { id: string; name: string; accountIds: string[] }[],
): string {
  if (!value || value === PORTFOLIO_ACCOUNT_ID) return "All Accounts";
  const portfolio = portfolios.find((p) => buildAccountSelection(p.accountIds) === value);
  if (portfolio) return portfolio.name;
  const account = accounts.find((a) => a.id === value);
  return account?.name ?? value;
}

export function AccountPortfolioSelector({
  value,
  onChange,
  className,
}: AccountPortfolioSelectorProps) {
  const [open, setOpen] = useState(false);
  const { accounts } = useAccounts({ filterActive: true, includeArchived: false });
  const { data: portfolios = [] } = usePortfolios();

  const currentLabel = labelForSelection(value, accounts, portfolios);

  const select = (id: string, label: string) => {
    onChange(id, label);
    setOpen(false);
  };

  // Group individual accounts by type
  const accountsByType: Record<string, typeof accounts> = {};
  for (const account of accounts) {
    (accountsByType[account.accountType] ??= []).push(account);
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          role="combobox"
          aria-expanded={open}
          className={cn(
            "bg-secondary/30 hover:bg-muted/80 flex h-10 items-center gap-1.5 rounded-full border-none px-3 text-sm font-medium",
            className,
          )}
        >
          <Icons.Wallet className="h-4 w-4 shrink-0 opacity-70" />
          <span>{currentLabel}</span>
          <Icons.ChevronsUpDown className="ml-1 h-4 w-4 shrink-0 opacity-50" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[260px] p-0" align="start" sideOffset={8}>
        <Command>
          <CommandInput placeholder="Search..." />
          <CommandList>
            <CommandEmpty>No results found.</CommandEmpty>

            {/* Portfolio group: All + named portfolios */}
            <CommandGroup heading="Portfolio">
              <CommandItem
                value={PORTFOLIO_ACCOUNT_ID}
                keywords={["all", "portfolio", "total"]}
                onSelect={() => select(PORTFOLIO_ACCOUNT_ID, "All Accounts")}
              >
                <Icons.Wallet className="mr-2 h-4 w-4 opacity-70" />
                <span>All Accounts</span>
                <Icons.Check
                  className={cn(
                    "ml-auto h-4 w-4",
                    value === PORTFOLIO_ACCOUNT_ID ? "opacity-100" : "opacity-0",
                  )}
                />
              </CommandItem>

              {portfolios.map((portfolio) => {
                const compositeId = buildAccountSelection(portfolio.accountIds);
                return (
                  <CommandItem
                    key={portfolio.id}
                    value={portfolio.id}
                    keywords={[portfolio.name]}
                    onSelect={() => select(compositeId, portfolio.name)}
                  >
                    <Icons.Briefcase className="mr-2 h-4 w-4 opacity-70" />
                    <span>{portfolio.name}</span>
                    <Icons.Check
                      className={cn(
                        "ml-auto h-4 w-4",
                        value === compositeId ? "opacity-100" : "opacity-0",
                      )}
                    />
                  </CommandItem>
                );
              })}
            </CommandGroup>

            {/* Individual accounts grouped by type */}
            {Object.entries(accountsByType).length > 0 && (
              <>
                <Separator />
                {Object.entries(accountsByType).map(([type, typeAccounts]) => (
                  <CommandGroup key={type} heading={type}>
                    {typeAccounts.map((account) => (
                      <CommandItem
                        key={account.id}
                        value={account.id}
                        keywords={[account.name, account.currency]}
                        onSelect={() => select(account.id, account.name)}
                      >
                        <Icons.CreditCard className="mr-2 h-4 w-4 opacity-70" />
                        <span>{account.name}</span>
                        <Icons.Check
                          className={cn(
                            "ml-auto h-4 w-4",
                            value === account.id ? "opacity-100" : "opacity-0",
                          )}
                        />
                      </CommandItem>
                    ))}
                  </CommandGroup>
                ))}
              </>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
