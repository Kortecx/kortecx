import { useCallback, useEffect, useState } from "react";
import {
  DEFAULT_MODEL_CHANGED_EVENT,
  clearDefaultModel,
  loadDefaultModel,
  saveDefaultModel,
} from "../lib/default-model";

/**
 * React binding for the client-local default-model preference (POC-5c). Reactive to
 * both same-tab changes (a custom event) and other-tab changes (the `storage` event)
 * so the Models "Set as default" control and the New Chat picker stay in sync.
 */
export function useDefaultModel() {
  const [defaultModelId, setStateValue] = useState<string | undefined>(() => loadDefaultModel());

  useEffect(() => {
    const sync = () => setStateValue(loadDefaultModel());
    window.addEventListener(DEFAULT_MODEL_CHANGED_EVENT, sync);
    window.addEventListener("storage", sync);
    return () => {
      window.removeEventListener(DEFAULT_MODEL_CHANGED_EVENT, sync);
      window.removeEventListener("storage", sync);
    };
  }, []);

  const setDefault = useCallback((modelId: string) => {
    saveDefaultModel(modelId);
    setStateValue(modelId);
  }, []);

  const clearDefault = useCallback(() => {
    clearDefaultModel();
    setStateValue(undefined);
  }, []);

  return { defaultModelId, setDefault, clearDefault };
}
