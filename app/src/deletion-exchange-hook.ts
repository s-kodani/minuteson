import { useState, useSyncExternalStore } from "react";
import { DeletionExchange } from "./deletion-exchange";
export function useDeletionExchange(notStartedCode: string) {
  const [exchange] = useState(() => new DeletionExchange(notStartedCode));
  const state = useSyncExternalStore(exchange.subscribe, exchange.getSnapshot);
  return { exchange, ...state, running: exchange.running, pending: exchange.pending, choice: exchange.choice, setUnknown: exchange.setUnknown, setError: exchange.setError, run: exchange.run };
}
