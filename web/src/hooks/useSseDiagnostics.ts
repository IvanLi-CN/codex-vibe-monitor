import { useEffect, useState } from "react";
import {
  getCurrentSseDiagnostics,
  type SseDiagnostics,
  subscribeToSseDiagnostics,
} from "../lib/sse";

export default function useSseDiagnostics() {
  const [diagnostics, setDiagnostics] = useState<SseDiagnostics>(() => getCurrentSseDiagnostics());

  useEffect(() => {
    const unsubscribe = subscribeToSseDiagnostics((next) => {
      setDiagnostics(next);
    });
    return unsubscribe;
  }, []);

  return diagnostics;
}
