import { createContext, useContext, type ReactNode } from "react";
import { PlatformContext } from "./platform";
import type { WindowEnvironment } from "./window-environment-state";
const EnvironmentContext = createContext<WindowEnvironment | null>(null);
export const useWindowEnvironment = () => useContext(EnvironmentContext);
export function WindowEnvironmentProvider({ value, children }: { value: WindowEnvironment; children: ReactNode }) {
  return <EnvironmentContext.Provider value={value}><PlatformContext.Provider value={value.platform}>{children}</PlatformContext.Provider></EnvironmentContext.Provider>;
}
