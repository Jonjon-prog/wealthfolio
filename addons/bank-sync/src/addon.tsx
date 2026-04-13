/**
 * Woob Bank Sync Addon
 *
 * Syncs French bank transactions via Woob (https://woob.tech).
 * Credentials are stored in the OS keychain — never in plain text.
 *
 * Prerequisites:
 *   pip3 install woob
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

interface BankModule {
  name: string;
  description: string;
}

interface ModuleField {
  name: string;
  label: string;
  masked: boolean;
}

interface ConfiguredBackend {
  name: string;
  module: string;
}

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
  latestTransactionDate: string | null; // YYYY-MM-DD from Rust
  syncedAt: string; // ISO timestamp set client-side when sync completes
}

interface TransferPairCandidate {
  id: string;
  outActivityId: string;
  outAccountId: string;
  outAccountName: string;
  outDate: string;
  inActivityId: string;
  inAccountId: string;
  inAccountName: string;
  inDate: string;
  amount: number;
  currency: string;
  sameDay: boolean;
}

interface WfAccount {
  id: string;
  name: string;
  currency: string;
}

// ─────────────────────────────────────────────────────────────────────────────
// Persistence helpers
// ─────────────────────────────────────────────────────────────────────────────

/** Survives page navigation but not app restarts. */
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

/** Survives app restarts (localStorage). */
function useLocalState<T>(key: string, initial: T) {
  const [value, setValue] = useState<T>(() => {
    try {
      const stored = localStorage.getItem(`woob_addon_${key}`);
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
          localStorage.setItem(`woob_addon_${key}`, JSON.stringify(resolved));
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
// Add Backend Modal
// ─────────────────────────────────────────────────────────────────────────────

function AddBackendModal({ onClose, onAdded }: { onClose: () => void; onAdded: () => void }) {
  const [step, setStep] = useState<"select" | "configure">("select");
  const [modules, setModules] = useState<BankModule[]>([]);
  const [search, setSearch] = useState("");
  const [selectedModule, setSelectedModule] = useState<BankModule | null>(null);
  const [backendName, setBackendName] = useState("");
  const [fields, setFields] = useState<ModuleField[]>([]);
  const [credentials, setCredentials] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Load module list on open
  useEffect(() => {
    setLoading(true);
    invoke<BankModule[]>("bank_sync_list_modules")
      .then(setModules)
      .catch((e: unknown) => setError(String(e)))
      .finally(() => setLoading(false));
  }, []);

  async function selectModule(mod: BankModule) {
    setSelectedModule(mod);
    setBackendName(mod.name);
    setError(null);
    setLoading(true);
    try {
      const f = await invoke<ModuleField[]>("bank_sync_module_config", { module: mod.name });
      setFields(f);
      setCredentials(Object.fromEntries(f.map((field) => [field.name, ""])));
      setStep("configure");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function save() {
    if (!selectedModule) return;
    setError(null);
    setLoading(true);
    try {
      await invoke("bank_sync_setup_backend", {
        backendName,
        module: selectedModule.name,
        credentials,
      });
      onAdded();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  const filtered = modules.filter(
    (m) =>
      m.name.toLowerCase().includes(search.toLowerCase()) ||
      m.description.toLowerCase().includes(search.toLowerCase()),
  );

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-background flex max-h-[80vh] w-full max-w-lg flex-col rounded-xl border shadow-xl">
        {/* Header — fixed */}
        <div className="flex shrink-0 items-center justify-between border-b px-6 py-4">
          <h2 className="text-base font-semibold">
            {step === "select" ? "Select bank" : `Configure ${selectedModule?.description}`}
          </h2>
          <button onClick={onClose} className="text-muted-foreground hover:text-foreground">
            <Icons.X className="h-4 w-4" />
          </button>
        </div>

        {/* Body — scrollable */}
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
          {error && (
            <div className="mb-4 rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300">
              {error}
            </div>
          )}

          {/* Step 1 — module selection */}
          {step === "select" && (
            <>
              <input
                autoFocus
                type="text"
                placeholder="Search banks…"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                className="border-input mb-2 w-full rounded-md border bg-transparent px-3 py-2 text-sm"
              />
              {loading ? (
                <div className="flex justify-center py-8">
                  <Icons.Loader className="text-muted-foreground h-5 w-5 animate-spin" />
                </div>
              ) : (
                <ul className="space-y-0.5">
                  {filtered.map((mod) => (
                    <li key={mod.name}>
                      <button
                        onClick={() => void selectModule(mod)}
                        className="hover:bg-muted flex w-full items-baseline gap-2 rounded-md px-3 py-2 text-left text-sm"
                      >
                        <span className="font-medium">{mod.description}</span>
                        <span className="text-muted-foreground text-xs">{mod.name}</span>
                      </button>
                    </li>
                  ))}
                  {filtered.length === 0 && (
                    <p className="text-muted-foreground py-4 text-center text-sm">No banks found</p>
                  )}
                </ul>
              )}
            </>
          )}

          {/* Step 2 — credential form */}
          {step === "configure" && (
            <div className="space-y-4">
              <div>
                <label className="mb-1 block text-sm font-medium">Backend name</label>
                <input
                  type="text"
                  value={backendName}
                  onChange={(e) => setBackendName(e.target.value)}
                  className="border-input w-full rounded-md border bg-transparent px-3 py-2 text-sm"
                />
                <p className="text-muted-foreground mt-1 text-xs">
                  Unique identifier for this account (e.g. "boursorama" or "boursorama-pro")
                </p>
              </div>

              {fields.map((field) => (
                <div key={field.name}>
                  <label className="mb-1 block text-sm font-medium">{field.label}</label>
                  <input
                    type={field.masked ? "password" : "text"}
                    value={credentials[field.name] ?? ""}
                    onChange={(e) =>
                      setCredentials((prev) => ({ ...prev, [field.name]: e.target.value }))
                    }
                    className="border-input w-full rounded-md border bg-transparent px-3 py-2 text-sm"
                  />
                </div>
              ))}

              <div className="rounded-md border border-blue-200 bg-blue-50 p-3 text-xs text-blue-700 dark:border-blue-800 dark:bg-blue-950 dark:text-blue-300">
                Credentials are stored in the OS keychain and never written to disk as plain text.
              </div>
            </div>
          )}
        </div>

        {/* Footer */}
        {step === "configure" && (
          <div className="flex justify-between border-t px-6 py-4">
            <Button variant="outline" onClick={() => setStep("select")}>
              Back
            </Button>
            <Button onClick={() => void save()} disabled={loading || !backendName.trim()}>
              {loading ? <Icons.Loader className="mr-2 h-4 w-4 animate-spin" /> : null}
              Save to keychain
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Main page
// ─────────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────────
// Link Transfers Panel
// ─────────────────────────────────────────────────────────────────────────────

function LinkTransfersPanel() {
  const [candidates, setCandidates] = useState<TransferPairCandidate[] | null>(null);
  const [selected, setSelected] = useState<Record<string, boolean>>({});
  const [scanning, setScanning] = useState(false);
  const [applying, setApplying] = useState(false);
  const [result, setResult] = useState<{ pairsLinked: number } | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function scan() {
    setScanning(true);
    setError(null);
    setResult(null);
    setCandidates(null);
    try {
      const pairs = await invoke<TransferPairCandidate[]>("bank_sync_find_transfer_pairs");
      setCandidates(pairs);
      // Pre-select same-day matches; leave ±1 day unchecked
      const initial: Record<string, boolean> = {};
      for (const p of pairs) initial[p.id] = p.sameDay;
      setSelected(initial);
    } catch (e) {
      setError(String(e));
    } finally {
      setScanning(false);
    }
  }

  async function apply() {
    if (!candidates) return;
    const toApply = candidates
      .filter((c) => selected[c.id])
      .map((c) => ({ outActivityId: c.outActivityId, inActivityId: c.inActivityId }));
    if (toApply.length === 0) return;
    setApplying(true);
    setError(null);
    try {
      const res = await invoke<{ pairsLinked: number }>("bank_sync_apply_transfer_pairs", {
        pairs: toApply,
      });
      setResult(res);
      setCandidates(null);
      setSelected({});
    } catch (e) {
      setError(String(e));
    } finally {
      setApplying(false);
    }
  }

  const selectedCount = Object.values(selected).filter(Boolean).length;

  return (
    <div className="rounded-lg border">
      <div className="flex items-center justify-between px-4 py-3">
        <div>
          <p className="text-sm font-medium">Link transfers</p>
          <p className="text-muted-foreground text-xs">
            Match DEPOSIT / WITHDRAWAL pairs across accounts and convert them to linked TRANSFER_IN
            / TRANSFER_OUT.
          </p>
        </div>
        <Button size="sm" variant="outline" onClick={() => void scan()} disabled={scanning}>
          {scanning ? (
            <Icons.Loader className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <Icons.Search className="mr-2 h-4 w-4" />
          )}
          {scanning ? "Scanning…" : "Find transfers"}
        </Button>
      </div>

      {error && (
        <div className="mx-4 mb-3 rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300">
          {error}
        </div>
      )}

      {result && (
        <div className="mx-4 mb-3 rounded-md border border-green-200 bg-green-50 p-3 text-sm text-green-700 dark:border-green-800 dark:bg-green-950 dark:text-green-300">
          {result.pairsLinked} transfer pair{result.pairsLinked !== 1 ? "s" : ""} linked.
        </div>
      )}

      {candidates !== null && (
        <div className="border-t">
          {candidates.length === 0 ? (
            <p className="text-muted-foreground px-4 py-4 text-center text-sm">
              No unlinked transfer pairs found.
            </p>
          ) : (
            <>
              <ul className="divide-y">
                {candidates.map((c) => (
                  <li key={c.id} className="flex items-center gap-3 px-4 py-3">
                    <input
                      type="checkbox"
                      checked={!!selected[c.id]}
                      onChange={(e) =>
                        setSelected((prev) => ({ ...prev, [c.id]: e.target.checked }))
                      }
                      className="h-4 w-4 shrink-0"
                    />
                    <div className="min-w-0 flex-1 text-sm">
                      <span className="font-medium">{c.outAccountName}</span>
                      <span className="text-muted-foreground mx-1.5">→</span>
                      <span className="font-medium">{c.inAccountName}</span>
                      <span className="text-muted-foreground ml-2">
                        {c.amount.toLocaleString("fr-FR", {
                          style: "currency",
                          currency: c.currency,
                        })}
                      </span>
                    </div>
                    <div className="shrink-0 text-right text-xs">
                      {c.sameDay ? (
                        <span className="text-muted-foreground">{c.outDate}</span>
                      ) : (
                        <span className="text-amber-600 dark:text-amber-400">
                          <Icons.AlertTriangle className="mr-1 inline h-3 w-3" />
                          {c.outDate} / {c.inDate}
                        </span>
                      )}
                    </div>
                  </li>
                ))}
              </ul>
              <div className="flex justify-end border-t px-4 py-3">
                <Button
                  size="sm"
                  onClick={() => void apply()}
                  disabled={applying || selectedCount === 0}
                >
                  {applying ? <Icons.Loader className="mr-2 h-4 w-4 animate-spin" /> : null}
                  Apply selected ({selectedCount})
                </Button>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

function defaultSinceDate() {
  const d = new Date();
  d.setMonth(d.getMonth() - 3);
  return d.toISOString().split("T")[0];
}

/** Returns YYYY-MM-DD one day before the given YYYY-MM-DD string. */
function subtractOneDay(date: string): string {
  const d = new Date(date);
  d.setDate(d.getDate() - 1);
  return d.toISOString().split("T")[0];
}

function formatSyncTime(iso: string): string {
  const d = new Date(iso);
  const diffMs = Date.now() - d.getTime();
  const mins = Math.floor(diffMs / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}min ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24)
    return `today at ${d.toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" })}`;
  if (hours < 48)
    return `yesterday at ${d.toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" })}`;
  return d.toLocaleDateString("fr-FR", {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Number of days in the [since, until] range. until defaults to today. */
function dateRangeDays(since: string, until: string): number {
  const from = since ? new Date(since).getTime() : 0;
  const to = until ? new Date(until).getTime() : Date.now();
  return Math.max(0, Math.round((to - from) / 86400000));
}

function BankSyncPage({ ctx }: { ctx: AddonContext }) {
  const [woobPath, setWoobPath] = useState<string | null>(null);
  const [woobError, setWoobError] = useState<string | null>(null);
  const [backends, setBackends] = useState<ConfiguredBackend[]>([]);
  // Accounts grouped by backend name
  const [accountsByBackend, setAccountsByBackend] = useSessionState<Record<string, BankAccount[]>>(
    "accounts_by_backend",
    {},
  );
  const [wfAccounts, setWfAccounts] = useState<WfAccount[]>([]);
  const [mapping, setMapping] = useSessionState<Record<string, string>>("mapping", {});
  const [sinceDate, setSinceDate] = useLocalState<string>("since_date", defaultSinceDate());
  const [untilDate, setUntilDate] = useSessionState<string>("until_date", "");
  // Collapsed state per backend (true = collapsed)
  const [collapsed, setCollapsed] = useSessionState<Record<string, boolean>>("collapsed", {});
  // Per-backend loading state
  const [loadingBackend, setLoadingBackend] = useState<string | null>(null);
  const [syncing, setSyncing] = useState<string | null>(null);
  // Persisted across restarts so "last synced" info survives app close/reopen
  const [results, setResults] = useLocalState<BankSyncResult[]>("results", []);
  // Per-account last transaction date (YYYY-MM-DD), persisted across restarts
  const [lastTxDates, setLastTxDates] = useLocalState<Record<string, string>>("last_tx_dates", {});
  const [error, setError] = useState<string | null>(null);
  const [needsMigration, setNeedsMigration] = useState(false);
  const [migrating, setMigrating] = useState(false);
  const [showAddModal, setShowAddModal] = useState(false);
  const [deletingBackend, setDeletingBackend] = useState<string | null>(null);

  const allAccounts = Object.values(accountsByBackend).flat();
  const hasMappedAccounts = allAccounts.some((a) => mapping[a.id]);

  function toggleCollapsed(name: string) {
    setCollapsed((prev) => ({ ...prev, [name]: !prev[name] }));
  }

  function loadBackends() {
    void invoke<ConfiguredBackend[]>("bank_sync_list_configured_backends").then(setBackends);
  }

  useEffect(() => {
    void invoke<string>("bank_sync_check_woob")
      .then((path) => setWoobPath(path))
      .catch((e: unknown) => setWoobError(String(e)));

    void invoke<boolean>("bank_sync_needs_migration")
      .then(setNeedsMigration)
      .catch(() => {});

    void ctx.api.accounts
      .getAll()
      .then((accs) =>
        setWfAccounts(accs.map((a) => ({ id: a.id, name: a.name, currency: a.currency }))),
      );

    loadBackends();
  }, [ctx.api.accounts]);

  async function migrateToKeychain() {
    setMigrating(true);
    setError(null);
    try {
      await invoke("bank_sync_migrate_to_keychain");
      setNeedsMigration(false);
      ctx.api.toast.success("Credentials migrated to keychain.");
    } catch (e) {
      setError(String(e));
    } finally {
      setMigrating(false);
    }
  }

  async function deleteBackend(name: string) {
    setDeletingBackend(name);
    try {
      await invoke("bank_sync_delete_backend", { backendName: name });
      setAccountsByBackend((prev) => {
        const next = { ...prev };
        delete next[name];
        return next;
      });
      setResults([]);
      loadBackends();
      ctx.api.toast.success(`Bank "${name}" removed.`);
    } catch (e) {
      setError(String(e));
    } finally {
      setDeletingBackend(null);
    }
  }

  async function loadAccountsForBackend(backendName: string) {
    setLoadingBackend(backendName);
    setError(null);
    try {
      const accounts = await invoke<BankAccount[]>("bank_sync_list_accounts", {
        backend: backendName,
      });
      setAccountsByBackend((prev) => ({ ...prev, [backendName]: accounts }));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoadingBackend(null);
    }
  }

  async function syncAccount(bankAccountId: string) {
    const wfAccountId = mapping[bankAccountId];
    if (!wfAccountId) return;
    setSyncing(bankAccountId);
    setError(null);
    try {
      // Use stored last tx date (- 1 day buffer) if the user hasn't set a manual since date.
      // This makes regular syncs fast: only fetch what's new since last time.
      const storedDate = lastTxDates[wfAccountId];
      const effectiveSince =
        sinceDate || storedDate ? subtractOneDay(sinceDate || storedDate!) : null;

      const raw = await invoke<Omit<BankSyncResult, "syncedAt">>("bank_sync_sync_account", {
        woobAccountId: bankAccountId,
        wfAccountId,
        sinceDate: effectiveSince,
        untilDate: untilDate || null,
      });
      const result: BankSyncResult = { ...raw, syncedAt: new Date().toISOString() };

      // Persist the latest transaction date for next sync
      if (result.latestTransactionDate) {
        setLastTxDates((prev) => ({ ...prev, [wfAccountId]: result.latestTransactionDate! }));
      }

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
    for (const acc of allAccounts) {
      if (mapping[acc.id]) await syncAccount(acc.id);
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
              <p>pip3 install woob</p>
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
    <>
      {showAddModal && (
        <AddBackendModal
          onClose={() => setShowAddModal(false)}
          onAdded={() => {
            loadBackends();
          }}
        />
      )}

      <Page>
        <PageHeader
          actions={
            <div className="flex items-center gap-2">
              {hasMappedAccounts && (
                <>
                  <div className="flex items-center gap-1.5">
                    <label className="text-muted-foreground whitespace-nowrap text-sm">From</label>
                    <input
                      type="date"
                      value={sinceDate}
                      onChange={(e) => setSinceDate(e.target.value)}
                      className="border-input rounded-md border bg-transparent px-2 py-1.5 text-sm"
                    />
                  </div>
                  <div className="flex items-center gap-1.5">
                    <label className="text-muted-foreground whitespace-nowrap text-sm">To</label>
                    <input
                      type="date"
                      value={untilDate}
                      onChange={(e) => setUntilDate(e.target.value)}
                      placeholder="Today"
                      className="border-input rounded-md border bg-transparent px-2 py-1.5 text-sm"
                    />
                  </div>
                  {dateRangeDays(sinceDate, untilDate) > 365 && (
                    <span
                      title="Fetching a large date range may take several minutes"
                      className="text-amber-600 dark:text-amber-400"
                    >
                      <Icons.AlertTriangle className="h-4 w-4" />
                    </span>
                  )}
                  <Button onClick={() => void syncAll()} disabled={syncing !== null} size="sm">
                    {syncing !== null ? (
                      <Icons.Loader className="mr-2 h-4 w-4 animate-spin" />
                    ) : (
                      <Icons.RefreshCw className="mr-2 h-4 w-4" />
                    )}
                    Sync all
                  </Button>
                </>
              )}
              <Button size="sm" variant="outline" onClick={() => setShowAddModal(true)}>
                <Icons.Plus className="mr-1.5 h-4 w-4" />
                Add bank
              </Button>
            </div>
          }
        >
          <div>
            <h1 className="text-lg font-semibold sm:text-xl">Bank Sync</h1>
            <p className="text-muted-foreground text-sm">Via Woob — {woobPath ?? "checking…"}</p>
          </div>
        </PageHeader>

        <PageContent>
          <div className="mx-auto max-w-2xl space-y-6">
            {/* Migration banner */}
            {needsMigration && (
              <div className="rounded-lg border border-amber-200 bg-amber-50 p-4 dark:border-amber-800 dark:bg-amber-950">
                <p className="font-medium text-amber-800 dark:text-amber-200">
                  Security: credentials stored in plain text
                </p>
                <p className="mt-1 text-sm text-amber-700 dark:text-amber-300">
                  Your Woob credentials are stored as plain text in{" "}
                  <code>~/.config/woob/backends</code>. Migrate them to the OS keychain.
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  className="mt-3"
                  onClick={() => void migrateToKeychain()}
                  disabled={migrating}
                >
                  {migrating ? (
                    <Icons.Loader className="mr-2 h-4 w-4 animate-spin" />
                  ) : (
                    <Icons.Shield className="mr-2 h-4 w-4" />
                  )}
                  {migrating ? "Migrating…" : "Migrate to Keychain"}
                </Button>
              </div>
            )}

            {error && (
              <div className="rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300">
                {error}
              </div>
            )}

            {/* Empty state */}
            {backends.length === 0 && (
              <div className="text-muted-foreground rounded-lg border border-dashed px-4 py-12 text-center text-sm">
                <Icons.Link className="text-muted-foreground/50 mx-auto mb-3 h-8 w-8" />
                <p className="font-medium">No banks configured</p>
                <p className="mt-1">Click "Add bank" to connect your first bank account.</p>
              </div>
            )}

            {/* One section per backend */}
            {backends.map((backend) => {
              const accounts = accountsByBackend[backend.name] ?? [];
              const isLoading = loadingBackend === backend.name;
              const hasLoaded = backend.name in accountsByBackend;
              const isCollapsed = !!collapsed[backend.name];

              return (
                <div key={backend.name} className="rounded-lg border">
                  {/* Backend header — click to collapse */}
                  <button
                    className="flex w-full items-center justify-between px-4 py-3 text-left"
                    onClick={() => toggleCollapsed(backend.name)}
                  >
                    <div className="flex items-center gap-2">
                      <Icons.ChevronDown
                        className={`text-muted-foreground h-4 w-4 transition-transform ${isCollapsed ? "-rotate-90" : ""}`}
                      />
                      <Icons.Building className="text-muted-foreground h-4 w-4" />
                      <span className="font-medium">{backend.name}</span>
                      <span className="text-muted-foreground text-xs">{backend.module}</span>
                      {hasLoaded && (
                        <span className="text-muted-foreground text-xs">
                          · {accounts.length} account{accounts.length !== 1 ? "s" : ""}
                        </span>
                      )}
                    </div>
                    <div className="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
                      <Button
                        size="sm"
                        variant="ghost"
                        className="text-muted-foreground"
                        disabled={isLoading}
                        onClick={() => void loadAccountsForBackend(backend.name)}
                      >
                        <Icons.RefreshCw
                          className={`h-3.5 w-3.5 ${isLoading ? "animate-spin" : ""}`}
                        />
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        className="text-destructive hover:text-destructive"
                        disabled={deletingBackend === backend.name}
                        onClick={() => void deleteBackend(backend.name)}
                      >
                        {deletingBackend === backend.name ? (
                          <Icons.Loader className="h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <Icons.Trash className="h-3.5 w-3.5" />
                        )}
                      </Button>
                    </div>
                  </button>

                  {/* Accounts — hidden when collapsed */}
                  {!isCollapsed && (
                    <div className="border-t">
                      {!hasLoaded ? (
                        <div className="text-muted-foreground flex items-center justify-center gap-2 px-4 py-6 text-sm">
                          <button
                            className="hover:text-foreground flex items-center gap-1.5 underline underline-offset-2 transition-colors"
                            onClick={() => void loadAccountsForBackend(backend.name)}
                          >
                            <Icons.RefreshCw className="h-3.5 w-3.5" />
                            Load accounts
                          </button>
                        </div>
                      ) : isLoading ? (
                        <div className="flex justify-center py-6">
                          <Icons.Loader className="text-muted-foreground h-5 w-5 animate-spin" />
                        </div>
                      ) : accounts.length === 0 ? (
                        <p className="text-muted-foreground px-4 py-6 text-center text-sm">
                          No accounts found.
                        </p>
                      ) : (
                        <ul className="divide-y">
                          {accounts.map((account) => {
                            const result = results.find(
                              (r) => r.wfAccountId === mapping[account.id],
                            );
                            return (
                              <li key={account.id} className="px-4 py-3">
                                <div className="flex items-center justify-between gap-4">
                                  <div className="min-w-0">
                                    <p className="truncate text-sm font-medium">{account.label}</p>
                                    <p className="text-muted-foreground text-xs">
                                      {account.balance.toLocaleString("fr-FR", {
                                        style: "currency",
                                        currency: account.currency,
                                      })}
                                      {result ? (
                                        <span className="ml-2">
                                          · Synced {formatSyncTime(result.syncedAt)}
                                          {result.transactionsImported > 0 &&
                                            ` (${result.transactionsImported} imported)`}
                                        </span>
                                      ) : mapping[account.id] &&
                                        lastTxDates[mapping[account.id]] ? (
                                        <span className="ml-2">
                                          · Last tx: {lastTxDates[mapping[account.id]]}
                                        </span>
                                      ) : null}
                                    </p>
                                  </div>
                                  <div className="flex shrink-0 items-center gap-2">
                                    <select
                                      value={mapping[account.id] ?? ""}
                                      onChange={(e) =>
                                        setMapping((m) => ({
                                          ...m,
                                          [account.id]: e.target.value,
                                        }))
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
                                      variant="outline"
                                      disabled={!mapping[account.id] || syncing === account.id}
                                      onClick={() => void syncAccount(account.id)}
                                    >
                                      {syncing === account.id ? (
                                        <Icons.Loader className="h-4 w-4 animate-spin" />
                                      ) : (
                                        <Icons.RefreshCw className="h-4 w-4" />
                                      )}
                                    </Button>
                                  </div>
                                </div>
                              </li>
                            );
                          })}
                        </ul>
                      )}
                    </div>
                  )}
                </div>
              );
            })}

            <LinkTransfersPanel />
          </div>
        </PageContent>
      </Page>
    </>
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
