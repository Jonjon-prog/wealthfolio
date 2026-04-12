/**
 * GoCardless (Nordigen) Bank Sync Addon
 *
 * Free EU bank sync via the GoCardless Bank Account Data API (PSD2).
 * No Wealthfolio Connect subscription required.
 *
 * Flow:
 *   1. User enters GoCardless client_id + client_secret (from bankaccountdata.gocardless.com)
 *   2. User picks their bank from a country-filtered list
 *   3. App creates a requisition and opens the OAuth bank login link in browser
 *   4. After login, user returns and maps GoCardless account to Wealthfolio account
 *   5. User clicks Sync to import transactions
 */

import { type AddonContext } from "@wealthfolio/addon-sdk";
import { Button, Icons, Page, PageContent, PageHeader } from "@wealthfolio/ui";
import React, { useCallback, useEffect, useState } from "react";

// ─────────────────────────────────────────────────────────────────────────────
// Tauri invoke helper — window.__TAURI__ is available because withGlobalTauri=true
// ─────────────────────────────────────────────────────────────────────────────

declare global {
  interface Window {
    __TAURI__: {
      core: {
        invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
      };
      shell?: {
        open(url: string): Promise<void>;
      };
    };
  }
}

function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return window.__TAURI__.core.invoke<T>(cmd, args);
}

async function openUrl(url: string): Promise<void> {
  if (window.__TAURI__?.shell) {
    await window.__TAURI__.shell.open(url);
  } else {
    window.open(url, "_blank");
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Types (mirroring nordigen_sync.rs)
// ─────────────────────────────────────────────────────────────────────────────

interface GcInstitution {
  id: string;
  name: string;
  bic?: string;
  transactionTotalDays?: string;
  countries: string[];
  logo?: string;
}

interface GcRequisition {
  id: string;
  status: string;
  link: string;
  accounts: string[];
  institutionId: string;
}

interface GcAccount {
  id: string;
  iban?: string;
  currency?: string;
  name?: string;
  product?: string;
  status?: string;
}

interface NordigenSyncResult {
  accountId: string;
  transactionsImported: number;
  transactionsSkipped: number;
}

interface WfAccount {
  id: string;
  name: string;
  currency: string;
}

// ─────────────────────────────────────────────────────────────────────────────
// Persist state in sessionStorage so it survives React re-mounts
// ─────────────────────────────────────────────────────────────────────────────

function useSessionState<T>(key: string, initial: T) {
  const [value, setValue] = useState<T>(() => {
    try {
      const stored = sessionStorage.getItem(`gc_addon_${key}`);
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
          sessionStorage.setItem(`gc_addon_${key}`, JSON.stringify(resolved));
        } catch {
          // ignore storage errors
        }
        return resolved;
      });
    },
    [key],
  );

  return [value, set] as const;
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 1 — Setup credentials
// ─────────────────────────────────────────────────────────────────────────────

function SetupStep({
  onDone,
  logger,
}: {
  onDone: () => void;
  logger: AddonContext["api"]["logger"];
}) {
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    if (!clientId.trim() || !clientSecret.trim()) {
      setError("Both Client ID and Client Secret are required.");
      return;
    }
    setLoading(true);
    setError(null);
    try {
      await invoke("nordigen_save_credentials", {
        clientId: clientId.trim(),
        clientSecret: clientSecret.trim(),
      });
      logger.info("GoCardless credentials saved");
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="mx-auto max-w-lg space-y-6 pt-8">
      <div>
        <h2 className="text-lg font-semibold">Connect GoCardless</h2>
        <p className="text-muted-foreground mt-1 text-sm">
          Create a free account at{" "}
          <button
            onClick={() => openUrl("https://bankaccountdata.gocardless.com")}
            className="text-primary underline"
          >
            bankaccountdata.gocardless.com
          </button>{" "}
          then go to Developers &rarr; User Secrets to get your credentials.
        </p>
      </div>

      <div className="space-y-4">
        <div>
          <label className="text-sm font-medium">Secret ID (Client ID)</label>
          <input
            type="text"
            value={clientId}
            onChange={(e) => setClientId(e.target.value)}
            placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
            className="border-input mt-1 w-full rounded-md border bg-transparent px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-blue-500"
          />
        </div>
        <div>
          <label className="text-sm font-medium">Secret Key (Client Secret)</label>
          <input
            type="password"
            value={clientSecret}
            onChange={(e) => setClientSecret(e.target.value)}
            placeholder="Your secret key"
            className="border-input mt-1 w-full rounded-md border bg-transparent px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-blue-500"
          />
        </div>

        {error && <p className="text-destructive text-sm">{error}</p>}

        <Button onClick={save} disabled={loading} className="w-full">
          {loading ? (
            <Icons.Loader className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <Icons.Check className="mr-2 h-4 w-4" />
          )}
          {loading ? "Connecting..." : "Save & Verify"}
        </Button>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 2 — Pick bank and create requisition
// ─────────────────────────────────────────────────────────────────────────────

function ConnectBankStep({
  onRequisition,
  logger,
}: {
  onRequisition: (req: GcRequisition) => void;
  logger: AddonContext["api"]["logger"];
}) {
  const [country, setCountry] = useState("FR");
  const [institutions, setInstitutions] = useState<GcInstitution[]>([]);
  const [search, setSearch] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function loadInstitutions() {
    setLoading(true);
    setError(null);
    try {
      const list = await invoke<GcInstitution[]>("nordigen_list_institutions", { country });
      setInstitutions(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadInstitutions();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [country]);

  const filtered = institutions.filter((i) => i.name.toLowerCase().includes(search.toLowerCase()));

  async function createRequisition() {
    if (!selectedId) return;
    setCreating(true);
    setError(null);
    try {
      const redirect = "wealthfolio://nordigen-callback";
      const req = await invoke<GcRequisition>("nordigen_create_requisition", {
        institutionId: selectedId,
        redirectUri: redirect,
      });
      logger.info(`Requisition created: ${req.id}`);
      await openUrl(req.link);
      onRequisition(req);
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  }

  const countries = [
    { code: "FR", label: "France" },
    { code: "DE", label: "Germany" },
    { code: "ES", label: "Spain" },
    { code: "IT", label: "Italy" },
    { code: "NL", label: "Netherlands" },
    { code: "BE", label: "Belgium" },
    { code: "PT", label: "Portugal" },
    { code: "GB", label: "United Kingdom" },
    { code: "SE", label: "Sweden" },
    { code: "NO", label: "Norway" },
    { code: "DK", label: "Denmark" },
    { code: "FI", label: "Finland" },
    { code: "PL", label: "Poland" },
    { code: "AT", label: "Austria" },
    { code: "CH", label: "Switzerland" },
  ];

  return (
    <div className="mx-auto max-w-2xl space-y-4 pt-4">
      <h2 className="text-lg font-semibold">Connect Your Bank</h2>

      <div className="flex gap-3">
        <select
          value={country}
          onChange={(e) => setCountry(e.target.value)}
          className="border-input rounded-md border bg-transparent px-3 py-2 text-sm"
        >
          {countries.map((c) => (
            <option key={c.code} value={c.code}>
              {c.label}
            </option>
          ))}
        </select>

        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search bank..."
          className="border-input flex-1 rounded-md border bg-transparent px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-blue-500"
        />
      </div>

      {error && <p className="text-destructive text-sm">{error}</p>}

      {loading ? (
        <div className="flex justify-center py-8">
          <Icons.Loader className="text-muted-foreground h-6 w-6 animate-spin" />
        </div>
      ) : (
        <div className="max-h-72 space-y-1 overflow-y-auto rounded-md border p-2">
          {filtered.length === 0 && (
            <p className="text-muted-foreground py-4 text-center text-sm">No banks found.</p>
          )}
          {filtered.map((inst) => (
            <button
              key={inst.id}
              onClick={() => setSelectedId(inst.id)}
              className={`flex w-full items-center gap-3 rounded px-3 py-2 text-left text-sm transition-colors hover:bg-blue-50 dark:hover:bg-blue-950 ${
                selectedId === inst.id ? "bg-blue-100 dark:bg-blue-900" : ""
              }`}
            >
              {inst.logo && (
                <img src={inst.logo} alt="" className="h-6 w-6 rounded object-contain" />
              )}
              <span className="flex-1 font-medium">{inst.name}</span>
              {inst.transactionTotalDays && (
                <span className="text-muted-foreground text-xs">
                  {inst.transactionTotalDays}d history
                </span>
              )}
            </button>
          ))}
        </div>
      )}

      <Button onClick={createRequisition} disabled={!selectedId || creating} className="w-full">
        {creating ? (
          <Icons.Loader className="mr-2 h-4 w-4 animate-spin" />
        ) : (
          <Icons.Link className="mr-2 h-4 w-4" />
        )}
        {creating ? "Opening bank login..." : "Connect Selected Bank"}
      </Button>

      <p className="text-muted-foreground text-xs">
        Your browser will open for secure bank authentication. Return here after completing login.
      </p>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 3 — Map GC accounts to WF accounts and sync
// ─────────────────────────────────────────────────────────────────────────────

function SyncStep({
  requisition,
  wfAccounts,
  onBack,
  logger,
  toast,
}: {
  requisition: GcRequisition;
  wfAccounts: WfAccount[];
  onBack: () => void;
  logger: AddonContext["api"]["logger"];
  toast: AddonContext["api"]["toast"];
}) {
  const [gcAccounts, setGcAccounts] = useSessionState<GcAccount[]>("gc_accounts", []);
  const [mapping, setMapping] = useSessionState<Record<string, string>>("mapping", {});
  const [dateFrom, setDateFrom] = useState<string>(() => {
    const d = new Date();
    d.setMonth(d.getMonth() - 3);
    return d.toISOString().slice(0, 10);
  });
  const [loadingAccounts, setLoadingAccounts] = useState(false);
  const [syncing, setSyncing] = useState<string | null>(null);
  const [results, setResults] = useState<NordigenSyncResult[]>([]);
  const [error, setError] = useState<string | null>(null);

  const [reqStatus, setReqStatus] = useSessionState<string>("req_status", requisition.status);
  const [polling, setPolling] = useState(reqStatus !== "LN");

  async function loadGcAccounts(ids: string[]) {
    if (ids.length === 0) return;
    setLoadingAccounts(true);
    try {
      const details = await Promise.all(
        ids.map((id) =>
          invoke<GcAccount>("nordigen_get_account_details", { gcAccountId: id }).then((acc) => ({
            ...acc,
            id,
          })),
        ),
      );
      setGcAccounts(details);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoadingAccounts(false);
    }
  }

  useEffect(() => {
    if (!polling) return;
    const timer = setInterval(async () => {
      try {
        const updated = await invoke<GcRequisition>("nordigen_get_requisition", {
          requisitionId: requisition.id,
        });
        setReqStatus(updated.status);
        if (updated.status === "LN") {
          setPolling(false);
          void loadGcAccounts(updated.accounts);
        }
      } catch {
        // ignore transient errors during polling
      }
    }, 3000);
    return () => clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [polling]);

  useEffect(() => {
    if (reqStatus === "LN" && gcAccounts.length === 0 && requisition.accounts.length > 0) {
      void loadGcAccounts(requisition.accounts);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function syncAccount(gcAccountId: string) {
    const wfAccountId = mapping[gcAccountId];
    if (!wfAccountId) return;
    setSyncing(gcAccountId);
    setError(null);
    try {
      const result = await invoke<NordigenSyncResult>("nordigen_sync_account", {
        accountId: wfAccountId,
        gcAccountId,
        dateFrom: dateFrom || null,
      });
      setResults((prev) => {
        const without = prev.filter((r) => r.accountId !== wfAccountId);
        return [...without, result];
      });
      toast.success(
        `Synced: ${result.transactionsImported} imported, ${result.transactionsSkipped} skipped`,
      );
      logger.info(`Sync complete for ${gcAccountId}: ${result.transactionsImported} imported`);
    } catch (e) {
      setError(String(e));
      toast.error(`Sync failed: ${String(e)}`);
    } finally {
      setSyncing(null);
    }
  }

  async function syncAll() {
    const pairs = gcAccounts.filter((a) => mapping[a.id]);
    for (const acc of pairs) {
      await syncAccount(acc.id);
    }
  }

  if (polling) {
    return (
      <div className="mx-auto max-w-lg space-y-4 pt-8 text-center">
        <Icons.Loader className="text-primary mx-auto h-8 w-8 animate-spin" />
        <p className="font-medium">Waiting for bank authentication...</p>
        <p className="text-muted-foreground text-sm">
          Status: <span className="font-mono">{reqStatus}</span>. Complete login in your browser
          then return here.
        </p>
        <Button variant="outline" onClick={() => setPolling(false)}>
          I have completed login
        </Button>
      </div>
    );
  }

  if (loadingAccounts) {
    return (
      <div className="mx-auto max-w-lg pt-8 text-center">
        <Icons.Loader className="text-primary mx-auto h-8 w-8 animate-spin" />
        <p className="mt-2 text-sm">Loading bank accounts...</p>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-2xl space-y-6 pt-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">Map & Sync Accounts</h2>
        <Button variant="outline" size="sm" onClick={onBack}>
          Connect another bank
        </Button>
      </div>

      <div className="flex items-center gap-3">
        <label className="whitespace-nowrap text-sm font-medium">Import from</label>
        <input
          type="date"
          value={dateFrom}
          onChange={(e) => setDateFrom(e.target.value)}
          className="border-input rounded-md border bg-transparent px-3 py-2 text-sm"
        />
      </div>

      {error && <p className="text-destructive text-sm">{error}</p>}

      {gcAccounts.length === 0 && (
        <div className="text-muted-foreground rounded-md border p-6 text-center text-sm">
          No accounts found. Make sure you completed bank login.
          <br />
          <button
            onClick={() => void loadGcAccounts(requisition.accounts)}
            className="text-primary mt-2 underline"
          >
            Retry loading accounts
          </button>
        </div>
      )}

      <div className="space-y-3">
        {gcAccounts.map((gc) => {
          const result = results.find((r) => r.accountId === mapping[gc.id]);
          return (
            <div key={gc.id} className="rounded-lg border p-4">
              <div className="flex items-start justify-between gap-4">
                <div className="min-w-0">
                  <p className="truncate font-medium">{gc.name ?? gc.iban ?? gc.id}</p>
                  {gc.iban && gc.name && <p className="text-muted-foreground text-xs">{gc.iban}</p>}
                  <p className="text-muted-foreground text-xs">{gc.currency}</p>
                </div>

                <div className="flex shrink-0 items-center gap-2">
                  <select
                    value={mapping[gc.id] ?? ""}
                    onChange={(e) => setMapping((m) => ({ ...m, [gc.id]: e.target.value }))}
                    className="border-input rounded-md border bg-transparent px-2 py-1.5 text-sm"
                  >
                    <option value="">select account</option>
                    {wfAccounts.map((wf) => (
                      <option key={wf.id} value={wf.id}>
                        {wf.name} ({wf.currency})
                      </option>
                    ))}
                  </select>

                  <Button
                    size="sm"
                    disabled={!mapping[gc.id] || syncing === gc.id}
                    onClick={() => void syncAccount(gc.id)}
                  >
                    {syncing === gc.id ? (
                      <Icons.Loader className="h-4 w-4 animate-spin" />
                    ) : (
                      <Icons.RefreshCw className="h-4 w-4" />
                    )}
                  </Button>
                </div>
              </div>

              {result && (
                <p className="text-muted-foreground mt-2 text-xs">
                  Last sync: {result.transactionsImported} imported, {result.transactionsSkipped}{" "}
                  skipped
                </p>
              )}
            </div>
          );
        })}
      </div>

      {gcAccounts.length > 1 && (
        <Button
          onClick={() => void syncAll()}
          disabled={Object.keys(mapping).length === 0 || syncing !== null}
          className="w-full"
        >
          {syncing !== null ? (
            <Icons.Loader className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <Icons.RefreshCw className="mr-2 h-4 w-4" />
          )}
          Sync All Mapped Accounts
        </Button>
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Main page component
// ─────────────────────────────────────────────────────────────────────────────

type Step = "setup" | "connect" | "sync";

function NordigenSyncPage({ ctx }: { ctx: AddonContext }) {
  const [step, setStep] = useSessionState<Step>("step", "setup");
  const [requisition, setRequisition] = useSessionState<GcRequisition | null>("req", null);
  const [wfAccounts, setWfAccounts] = useState<WfAccount[]>([]);
  const [resetting, setResetting] = useState(false);

  useEffect(() => {
    void ctx.api.accounts.getAll().then((accs) => {
      setWfAccounts(accs.map((a) => ({ id: a.id, name: a.name, currency: a.currency })));
    });

    void invoke<boolean>("nordigen_check_credentials").then((ok) => {
      if (ok && step === "setup") setStep("connect");
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function reset() {
    setResetting(true);
    try {
      await invoke("nordigen_clear_credentials");
      const keys = ["step", "req", "gc_accounts", "mapping", "req_status"];
      keys.forEach((k) => sessionStorage.removeItem(`gc_addon_${k}`));
      setStep("setup");
      setRequisition(null);
    } finally {
      setResetting(false);
    }
  }

  const headerActions =
    step !== "setup" ? (
      <Button variant="outline" size="sm" onClick={() => void reset()} disabled={resetting}>
        {resetting ? (
          <Icons.Loader className="mr-2 h-4 w-4 animate-spin" />
        ) : (
          <Icons.Trash className="mr-2 h-4 w-4" />
        )}
        Disconnect
      </Button>
    ) : undefined;

  return (
    <Page>
      <PageHeader actions={headerActions}>
        <div>
          <h1 className="text-lg font-semibold sm:text-xl">GoCardless Bank Sync</h1>
          <p className="text-muted-foreground text-sm">
            Free EU bank sync via GoCardless (Nordigen) — no subscription required
          </p>
        </div>
      </PageHeader>
      <PageContent>
        {step === "setup" && (
          <SetupStep onDone={() => setStep("connect")} logger={ctx.api.logger} />
        )}
        {step === "connect" && (
          <ConnectBankStep
            onRequisition={(req) => {
              setRequisition(req);
              setStep("sync");
            }}
            logger={ctx.api.logger}
          />
        )}
        {step === "sync" && requisition && (
          <SyncStep
            requisition={requisition}
            wfAccounts={wfAccounts}
            onBack={() => setStep("connect")}
            logger={ctx.api.logger}
            toast={ctx.api.toast}
          />
        )}
      </PageContent>
    </Page>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Addon registration
// ─────────────────────────────────────────────────────────────────────────────

export default function enable(ctx: AddonContext) {
  ctx.api.logger.info("GoCardless Bank Sync addon enabling");

  const sidebarItem = ctx.sidebar.addItem({
    id: "nordigen-sync",
    label: "Bank Sync",
    icon: <Icons.Link className="h-5 w-5" />,
    route: "/addon/nordigen-sync",
    order: 300,
  });

  ctx.router.add({
    path: "/addon/nordigen-sync",
    component: React.lazy(() =>
      Promise.resolve({
        default: () => <NordigenSyncPage ctx={ctx} />,
      }),
    ),
  });

  ctx.api.logger.info("GoCardless Bank Sync addon enabled");

  ctx.onDisable(() => {
    sidebarItem.remove();
  });
}
