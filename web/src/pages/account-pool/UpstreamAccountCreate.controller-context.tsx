// biome-ignore-all lint/suspicious/noExplicitAny: controller context is an incremental migration boundary shared by extracted page modules
import { createContext, useContext } from "react";

export type UpstreamAccountCreateControllerContext = Record<string, any>;

const UpstreamAccountCreateViewContext =
  createContext<UpstreamAccountCreateControllerContext | null>(null);

export const UpstreamAccountCreateViewProvider = UpstreamAccountCreateViewContext.Provider;

export function useUpstreamAccountCreateViewContext() {
  const context = useContext(UpstreamAccountCreateViewContext);
  if (!context) {
    throw new Error("UpstreamAccountCreateViewContext is missing");
  }
  return context;
}
