import { afterEach, vi } from "vitest";

function abortError(): Error {
  const error = new Error("aborted");
  error.name = "AbortError";
  return error;
}
function createAbortAwareFetch() {
  return vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
    return new Promise<Response>((_resolve, reject) => {
      const signal = init?.signal;
      if (!signal) return;
      if (signal.aborted) {
        reject(abortError());
        return;
      }
      signal.addEventListener(
        "abort",
        () => {
          reject(abortError());
        },
        { once: true },
      );
    });
  });
}
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});
afterEach(() => {
  vi.unstubAllGlobals();
});

export { abortError, createAbortAwareFetch };
