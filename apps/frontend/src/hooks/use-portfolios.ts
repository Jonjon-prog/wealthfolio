import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createPortfolioGroup,
  deletePortfolioGroup,
  findPortfolioByAccounts,
  getPortfolioGroup,
  getPortfolios,
  updatePortfolioGroup,
} from "@/adapters";
import { QueryKeys } from "@/lib/query-keys";
import type { NewPortfolio, Portfolio } from "@/lib/types";

export function usePortfolios() {
  return useQuery<Portfolio[], Error>({
    queryKey: [QueryKeys.PORTFOLIOS],
    queryFn: getPortfolios,
  });
}

export function usePortfolioGroup(id: string | null) {
  return useQuery<Portfolio | null, Error>({
    queryKey: QueryKeys.portfolioGroup(id ?? ""),
    queryFn: () => getPortfolioGroup(id!),
    enabled: !!id,
  });
}

export function useFindPortfolioByAccounts(accountIds: string[]) {
  return useQuery<Portfolio | null, Error>({
    // eslint-disable-next-line @tanstack/query/exhaustive-deps
    queryKey: [QueryKeys.PORTFOLIOS, "match", [...accountIds].sort().join(",")],
    queryFn: () => findPortfolioByAccounts(accountIds),
    enabled: accountIds.length >= 2,
  });
}

export function useCreatePortfolio() {
  const queryClient = useQueryClient();
  return useMutation<Portfolio, Error, NewPortfolio>({
    mutationFn: createPortfolioGroup,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: [QueryKeys.PORTFOLIOS] });
    },
  });
}

export function useUpdatePortfolio() {
  const queryClient = useQueryClient();
  return useMutation<Portfolio, Error, Portfolio>({
    mutationFn: updatePortfolioGroup,
    onSuccess: (updated) => {
      void queryClient.invalidateQueries({ queryKey: [QueryKeys.PORTFOLIOS] });
      queryClient.setQueryData(QueryKeys.portfolioGroup(updated.id), updated);
    },
  });
}

export function useDeletePortfolio() {
  const queryClient = useQueryClient();
  return useMutation<void, Error, string>({
    mutationFn: deletePortfolioGroup,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: [QueryKeys.PORTFOLIOS] });
    },
  });
}
