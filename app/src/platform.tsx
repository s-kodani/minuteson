import {createContext, useContext} from "react";
export const PlatformContext = createContext<"windows" | "macos" | "other">("other");
export const usePlatform = () => useContext(PlatformContext);
