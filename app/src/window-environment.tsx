import { createContext, useContext, useEffect, type ReactNode } from "react";
import { PlatformContext } from "./platform";
import type { WindowEnvironment } from "./window-environment-state";
const EnvironmentContext = createContext<WindowEnvironment | null>(null);
export const useWindowEnvironment = () => useContext(EnvironmentContext);
export const useAppName = () => useWindowEnvironment()?.productName ?? "MinutesOn";
export function WindowEnvironmentProvider({ value, children }: { value: WindowEnvironment; children: ReactNode }) {
  useEffect(() => { document.title = `${value.productName} — 会議記録`; }, [value.productName]);
  return <EnvironmentContext.Provider value={value}><PlatformContext.Provider value={value.platform}>{children}</PlatformContext.Provider></EnvironmentContext.Provider>;
}
