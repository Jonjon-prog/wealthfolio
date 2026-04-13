/**
 * Woob Bank Sync Addon
 *
 * Syncs French bank transactions via Woob (https://woob.tech).
 * No API key, no subscription — Woob scrapes bank websites locally.
 *
 * Prerequisites:
 *   pip3 install woob
 *   woob bank backends add boursorama   (or any supported bank)
 */

import { type AddonContext } from "@wealthfolio/addon-sdk";
import { Button, Icons, Page, PageContent, PageHeader } from "@wealthfolio/ui";
import React, { useCallback, useEffect, useState } from "react";

// ─────────────────────────────────────────────────────────────────────────────
// Tauri invoke (withGlobalTauri = true)
// ─────────────────────────────────────────────────────────────────────────────

declare global {
  interface Window {
    __TAURI__: {
      core: { invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> };
    };
  }
}

function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return window.__TAURI__.core.invoke<T>(cmd, args);
}

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

interface BankAccount {
  id: string;
  label: string;
  balance: number;
  currency: string;
}

interface BankSyncResult {
  wfAccountId: string;
  transactionsImported: number;
  transactionsSkipped: number;
}

interface WfAccount {
  id: string;
  name: string;
  currency: string;
}

// ─────────────────────────────────────────────────────────────────────────────
// Session state helper
// ─────────────────────────────────────────────────────────────────────────────

function useSessionState<T>(key: string, initial: T) {
  const [value, setValue] = useState<T>(() => {
    try {
      const stored = sessionStorage.getItem(`woob_addon_${key}`);
      return stored ? (JSON.parse(stored) as T) : initial;
    } catch {
      return initial;
    }
  });

  const set = useCallback(
    (next: T | ((prev: T) => T)) => {
      setValue((prev) => {
        const resolved = typeof next === "function" ? (next as (p: T) => T)(prev) : next;
        try {
          sessionStorage.setItem(`woob_addon_${key}`, JSON.stringify(resolved));
        } catch {
          // ignore
        }
        return resolved;
      });
    },
    [key],
  );

  return [value, set] as const;
}

// ─────────────────────────────────────────────────────────────────────────────
// Main page
// ─────────────────────────────────────────────────────────────────────────────

function BankSyncPage({ ctx }: { ctx: AddonContext }) {
  const [woobPath, setWoobPath] = useState<string | null>(null);
  const [woobError, setWoobError] = useState<string | null>(null);
  const [bankAccounts, setBankAccounts] = useSessionState<BankAccount[]>("bank_accounts", []);
  const [wfAccounts, setWfAccounts] = useState<WfAccount[]>([]);
  const [mapping, setMapping] = useSessionState<Record<string, string>>("mapping", {});
  const [count, setCount] = useState(200);
  const [loading, setLoading] = useState(false);
  const [syncing, setSyncing] = useState<string | null>(null);
  const [results, setResults] = useSessionState<BankSyncResult[]>("results", []);
  const [error, setError] = useState<string | null>(null);

  // Check Woob and load WF accounts on mount
  useEffect(() => {
    void invoke<string>("bank_sync_check_woob")
      .then((path) => setWoobPath(path))
      .catch((e: unknown) => setWoobError(String(e)));

    void ctx.api.accounts
      .getAll()
      .then((accs) =>
        setWfAccounts(accs.map((a) => ({ id: a.id, name: a.name, currency: a.currency }))),
      );
  }, [ctx.api.accounts]);

  async function loadBankAccounts() {
    setLoading(true);
    setError(null);
    try {
      const accounts = await invoke<BankAccount[]>("bank_sync_list_accounts", { backend: null });
      setBankAccounts(accounts);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function syncAccount(bankAccountId: string) {
    const wfAccountId = mapping[bankAccountId];
    if (!wfAccountId) return;
    setSyncing(bankAccountId);
    setError(null);
    try {
      const result = await invoke<BankSyncResult>("bank_sync_sync_account", {
        woobAccountId: bankAccountId,
        wfAccountId,
        count,
      });
      setResults((prev) => {
        const without = prev.filter((r) => r.wfAccountId !== wfAccountId);
        return [...without, result];
      });
      ctx.api.toast.success(
        `Synced: ${result.transactionsImported} imported, ${result.transactionsSkipped} skipped`,
      );
    } catch (e) {
      setError(String(e));
      ctx.api.toast.error(`Sync failed: ${String(e)}`);
    } finally {
      setSyncing(null);
    }
  }

  async function syncAll() {
    const pairs = bankAccounts.filter((a) => mapping[a.id]);
    for (const acc of pairs) {
      await syncAccount(acc.id);
    }
  }

  // ── Woob not installed ──────────────────────────────────────────────────────
  if (woobError) {
    return (
      <Page>
        <PageHeader>
          <h1 className="text-lg font-semibold">Bank Sync (Woob)</h1>
        </PageHeader>
        <PageContent>
          <div className="mx-auto max-w-lg space-y-4 pt-8">
            <div className="rounded-lg border border-amber-200 bg-amber-50 p-4 dark:border-amber-800 dark:bg-amber-950">
              <p className="font-medium text-amber-800 dark:text-amber-200">Woob not found</p>
              <p className="mt-1 text-sm text-amber-700 dark:text-amber-300">{woobError}</p>
            </div>
            <div className="bg-muted space-y-1 rounded-md p-4 font-mono text-sm">
              <p># Install Woob</p>
              <p>pip3 install woob</p>
              <p className="mt-2"># Add your bank (example: Boursobank)</p>
              <p>woob bank backends add boursorama</p>
            </div>
            <Button onClick={() => window.location.reload()} variant="outline" className="w-full">
              <Icons.RefreshCw className="mr-2 h-4 w-4" />
              Retry
            </Button>
          </div>
        </PageContent>
      </Page>
    );
  }

  // ── Main UI ─────────────────────────────────────────────────────────────────
  return (
    <Page>
      <PageHeader
        actions={
          <div className="flex items-center gap-2">
            <label className="whitespace-nowrap text-sm font-medium">Max transactions</label>
            <input
              type="number"
              value={count}
              min={10}
              max={1000}
              onChange={(e) => setCount(Number(e.target.value))}
              className="border-input w-24 rounded-md border bg-transparent px-2 py-1.5 text-sm"
            />
            <Button
              onClick={() => void syncAll()}
              disabled={Object.keys(mapping).length === 0 || syncing !== null}
            >
              {syncing !== null ? (
                <Icons.Loader className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Icons.RefreshCw className="mr-2 h-4 w-4" />
              )}
              Sync All
            </Button>
          </div>
        }
      >
        <div>
          <h1 className="text-lg font-semibold sm:text-xl">Bank Sync</h1>
          <p className="text-muted-foreground text-sm">Via Woob — {woobPath ?? "checking..."}</p>
        </div>
      </PageHeader>

      <PageContent>
        <div className="mx-auto max-w-2xl space-y-4">
          {error && (
            <div className="rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300">
              {error}
            </div>
          )}

          {/* Load accounts button */}
          {bankAccounts.length === 0 ? (
            <div className="flex flex-col items-center gap-4 py-12">
              <p className="text-muted-foreground text-sm">
                Load your bank accounts configured in Woob.
              </p>
              <Button onClick={() => void loadBankAccounts()} disabled={loading}>
                {loading ? (
                  <Icons.Loader className="mr-2 h-4 w-4 animate-spin" />
                ) : (
                  <Icons.RefreshCw className="mr-2 h-4 w-4" />
                )}
                Load Bank Accounts
              </Button>
            </div>
          ) : (
            <>
              <div className="flex items-center justify-between">
                <p className="text-muted-foreground text-sm">
                  {bankAccounts.length} account{bankAccounts.length !== 1 ? "s" : ""} found
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void loadBankAccounts()}
                  disabled={loading}
                >
                  <Icons.RefreshCw className={`mr-2 h-4 w-4 ${loading ? "animate-spin" : ""}`} />
                  Refresh
                </Button>
              </div>

              <div className="space-y-3">
                {bankAccounts.map((bank) => {
                  const result = results.find((r) => r.wfAccountId === mapping[bank.id]);
                  return (
                    <div key={bank.id} className="rounded-lg border p-4">
                      <div className="flex items-center justify-between gap-4">
                        <div className="min-w-0">
                          <p className="truncate font-medium">{bank.label}</p>
                          <p className="text-muted-foreground text-xs">
                            {bank.balance.toLocaleString("fr-FR", {
                              style: "currency",
                              currency: bank.currency,
                            })}
                          </p>
                        </div>

                        <div className="flex shrink-0 items-center gap-2">
                          <select
                            value={mapping[bank.id] ?? ""}
                            onChange={(e) =>
                              setMapping((m) => ({ ...m, [bank.id]: e.target.value }))
                            }
                            className="border-input rounded-md border bg-transparent px-2 py-1.5 text-sm"
                          >
                            <option value="">— select account —</option>
                            {wfAccounts.map((wf) => (
                              <option key={wf.id} value={wf.id}>
                                {wf.name} ({wf.currency})
                              </option>
                            ))}
                          </select>

                          <Button
                            size="sm"
                            disabled={!mapping[bank.id] || syncing === bank.id}
                            onClick={() => void syncAccount(bank.id)}
                          >
                            {syncing === bank.id ? (
                              <Icons.Loader className="h-4 w-4 animate-spin" />
                            ) : (
                              <Icons.RefreshCw className="h-4 w-4" />
                            )}
                          </Button>
                        </div>
                      </div>

                      {result && (
                        <p className="text-muted-foreground mt-2 text-xs">
                          Last sync: {result.transactionsImported} imported,{" "}
                          {result.transactionsSkipped} skipped
                        </p>
                      )}
                    </div>
                  );
                })}
              </div>
            </>
          )}
        </div>
      </PageContent>
    </Page>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Addon registration
// ─────────────────────────────────────────────────────────────────────────────

export default function enable(ctx: AddonContext) {
  const sidebarItem = ctx.sidebar.addItem({
    id: "bank-sync",
    label: "Bank Sync",
    icon: <Icons.Link className="h-5 w-5" />,
    route: "/addon/bank-sync",
    order: 300,
  });

  ctx.router.add({
    path: "/addon/bank-sync",
    component: React.lazy(() => Promise.resolve({ default: () => <BankSyncPage ctx={ctx} /> })),
  });

  ctx.onDisable(() => sidebarItem.remove());
}
