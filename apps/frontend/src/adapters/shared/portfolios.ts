import { invoke } from "./platform";
import type { NewPortfolio, Portfolio } from "@/lib/types";

export const getPortfolios = async (): Promise<Portfolio[]> => {
  return invoke<Portfolio[]>("get_portfolios");
};

export const getPortfolioGroup = async (id: string): Promise<Portfolio | null> => {
  return invoke<Portfolio | null>("get_portfolio_group", { id });
};

export const createPortfolioGroup = async (portfolio: NewPortfolio): Promise<Portfolio> => {
  return invoke<Portfolio>("create_portfolio_group", { portfolio });
};

export const updatePortfolioGroup = async (portfolio: Portfolio): Promise<Portfolio> => {
  return invoke<Portfolio>("update_portfolio_group", { portfolio });
};

export const deletePortfolioGroup = async (id: string): Promise<void> => {
  return invoke<void>("delete_portfolio_group", { id });
};

export const findPortfolioByAccounts = async (accountIds: string[]): Promise<Portfolio | null> => {
  return invoke<Portfolio | null>("find_portfolio_by_accounts", { accountIds });
};

/// Build a composite account selection string for the backend.
/// Single account → its UUID, all accounts → "PORTFOLIO",
/// multiple accounts → "MULTI:uuid1,uuid2,..."
export function buildAccountSelection(accountIds: string[]): string {
  if (accountIds.length === 0) return "PORTFOLIO";
  if (accountIds.length === 1) return accountIds[0];
  return `MULTI:${[...accountIds].sort().join(",")}`;
}
