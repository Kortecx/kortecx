/**
 * The skill-catalog hooks: list the caller's skills (`ListSkills`), one
 * skill's form (`GetSkillForm` — the wish set with the ADVISORY `registered`
 * bit + the instructions preview), add (`AddSkill`), and remove (`RemoveSkill`).
 *
 * A skill is a DECLARATIVE `kortecx.skill/v1` bundle — instructions + a tool
 * grant-WISH set. Adding one grants NOTHING (SN-8): at `RunApp` the server
 * intersects the wish against the caller's grants and the live broker
 * (`wish ∩ grants ∩ fireable`). Identity (`skillRef` / `instructionsRef`) is
 * server-derived. Degrades to a not-wired empty state on an old gateway
 * (UNIMPLEMENTED).
 */

import type { AddSkillInput, AddSkillResult, SkillForm, SkillSummary } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useListSkills() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.skills(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<SkillSummary[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listSkills();
    },
  });
  return {
    skills: q.data ?? [],
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
    refetch: q.refetch,
  };
}

/** One skill's form, fetched on demand (drawer-open), never eagerly. */
export function useSkillForm(name: string | null) {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.skillForm(endpoint, name ?? ""),
    enabled: status === "connected" && client !== null && name !== null,
    queryFn: async (): Promise<SkillForm | null> => {
      if (!client || !name) {
        throw new Error("not connected");
      }
      return client.getSkillForm(name);
    },
  });
  return {
    form: q.data ?? null,
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
  };
}

export function useAddSkill() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<AddSkillResult, unknown, AddSkillInput>({
    mutationFn: async (input) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.addSkill(input);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.skills(endpoint) });
    },
  });
}

export function useRemoveSkill() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, string>({
    mutationFn: async (name) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.removeSkill(name);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.skills(endpoint) });
    },
  });
}
