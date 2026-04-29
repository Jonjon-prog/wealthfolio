import { useAccounts } from "@/hooks/use-accounts";
import {
  useCreatePortfolio,
  useDeletePortfolio,
  usePortfolios,
  useUpdatePortfolio,
} from "@/hooks/use-portfolios";
import type { NewPortfolio, Portfolio } from "@/lib/types";
import {
  Button,
  Checkbox,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  EmptyPlaceholder,
  Icons,
  Input,
  Label,
  Separator,
  Skeleton,
} from "@wealthfolio/ui";
import { useState } from "react";
import { SettingsHeader } from "../settings-header";

function PortfolioFormDialog({
  open,
  portfolio,
  onClose,
}: {
  open: boolean;
  portfolio: Portfolio | null;
  onClose: () => void;
}) {
  const { accounts } = useAccounts({ filterActive: false, includeArchived: false });
  const createPortfolio = useCreatePortfolio();
  const updatePortfolio = useUpdatePortfolio();

  const [name, setName] = useState(portfolio?.name ?? "");
  const [selectedIds, setSelectedIds] = useState<string[]>(portfolio?.accountIds ?? []);

  // Reset state when dialog opens with new data
  const handleOpenChange = (isOpen: boolean) => {
    if (!isOpen) {
      onClose();
    }
  };

  // Re-initialize when portfolio prop changes
  const initKey = portfolio?.id ?? "new";

  function toggleAccount(id: string) {
    setSelectedIds((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim() || selectedIds.length < 2) return;

    if (portfolio) {
      await updatePortfolio.mutateAsync({
        ...portfolio,
        name: name.trim(),
        accountIds: selectedIds,
      });
    } else {
      const newPortfolio: NewPortfolio = { name: name.trim(), accountIds: selectedIds };
      await createPortfolio.mutateAsync(newPortfolio);
    }
    onClose();
  }

  const isPending = createPortfolio.isPending || updatePortfolio.isPending;

  return (
    <Dialog open={open} onOpenChange={handleOpenChange} key={initKey}>
      <DialogContent className="max-w-md">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>{portfolio ? "Edit Portfolio" : "New Portfolio"}</DialogTitle>
          </DialogHeader>

          <div className="space-y-4 py-4">
            <div className="space-y-1.5">
              <Label htmlFor="portfolio-name">Name</Label>
              <Input
                id="portfolio-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="e.g. Retirement, Tax-Free"
                autoFocus
              />
            </div>

            <div className="space-y-1.5">
              <Label>Accounts</Label>
              <p className="text-muted-foreground text-xs">Select 2 or more accounts.</p>
              <div className="divide-border max-h-56 divide-y overflow-y-auto rounded-md border">
                {accounts.map((account) => (
                  <label
                    key={account.id}
                    className="hover:bg-muted/50 flex cursor-pointer items-center gap-3 px-3 py-2.5"
                  >
                    <Checkbox
                      checked={selectedIds.includes(account.id)}
                      onCheckedChange={() => toggleAccount(account.id)}
                    />
                    <span className="text-sm">{account.name}</span>
                    <span className="text-muted-foreground ml-auto text-xs">
                      {account.currency}
                    </span>
                  </label>
                ))}
              </div>
            </div>
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose} disabled={isPending}>
              Cancel
            </Button>
            <Button type="submit" disabled={!name.trim() || selectedIds.length < 2 || isPending}>
              {isPending ? "Saving..." : portfolio ? "Save" : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

const PortfoliosPage = () => {
  const { data: portfolios, isLoading } = usePortfolios();
  const { accounts } = useAccounts({ filterActive: false, includeArchived: false });
  const deletePortfolio = useDeletePortfolio();

  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingPortfolio, setEditingPortfolio] = useState<Portfolio | null>(null);

  const accountNameById = Object.fromEntries(accounts.map((a) => [a.id, a.name]));

  const handleAdd = () => {
    setEditingPortfolio(null);
    setDialogOpen(true);
  };

  const handleEdit = (portfolio: Portfolio) => {
    setEditingPortfolio(portfolio);
    setDialogOpen(true);
  };

  const handleDelete = (portfolio: Portfolio) => {
    deletePortfolio.mutate(portfolio.id);
  };

  if (isLoading) {
    return (
      <div className="space-y-3">
        <Skeleton className="h-12" />
        <Skeleton className="h-12" />
      </div>
    );
  }

  return (
    <>
      <div className="space-y-6">
        <SettingsHeader
          heading="Portfolios"
          text="Group accounts into named portfolios for focused views."
        >
          <>
            <Button
              size="icon"
              className="sm:hidden"
              onClick={handleAdd}
              aria-label="Add portfolio"
            >
              <Icons.Plus className="h-4 w-4" />
            </Button>
            <Button size="sm" className="hidden sm:inline-flex" onClick={handleAdd}>
              <Icons.Plus className="mr-2 h-4 w-4" />
              Add portfolio
            </Button>
          </>
        </SettingsHeader>
        <Separator />

        {!portfolios?.length ? (
          <EmptyPlaceholder>
            <EmptyPlaceholder.Icon name="Briefcase" />
            <EmptyPlaceholder.Title>No portfolios yet</EmptyPlaceholder.Title>
            <EmptyPlaceholder.Description>
              Group two or more accounts into a portfolio to view them together.
            </EmptyPlaceholder.Description>
            <Button onClick={handleAdd}>
              <Icons.Plus className="mr-2 h-4 w-4" />
              Add portfolio
            </Button>
          </EmptyPlaceholder>
        ) : (
          <div className="divide-border bg-card divide-y rounded-md border">
            {portfolios.map((portfolio) => (
              <div key={portfolio.id} className="flex items-center justify-between p-4">
                <div className="min-w-0">
                  <p className="font-medium">{portfolio.name}</p>
                  <p className="text-muted-foreground mt-0.5 truncate text-sm">
                    {portfolio.accountIds.map((id) => accountNameById[id] ?? id).join(", ")}
                  </p>
                </div>
                <div className="ml-4 flex shrink-0 items-center gap-1">
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-8 w-8"
                    onClick={() => handleEdit(portfolio)}
                    aria-label="Edit portfolio"
                  >
                    <Icons.Pencil className="h-4 w-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="text-destructive hover:text-destructive h-8 w-8"
                    onClick={() => handleDelete(portfolio)}
                    aria-label="Delete portfolio"
                  >
                    <Icons.Trash className="h-4 w-4" />
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      <PortfolioFormDialog
        open={dialogOpen}
        portfolio={editingPortfolio}
        onClose={() => setDialogOpen(false)}
      />
    </>
  );
};

export default PortfoliosPage;
