/// <reference types="vite/client" />
/// <reference types="vite-plugin-pwa/client" />

declare module "*?suite" {
  const registerSuite: (scope: Record<string, unknown>) => void;
  export default registerSuite;
}
