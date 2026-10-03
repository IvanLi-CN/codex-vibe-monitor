const DASHBOARDS = new Set(["cvm-overview", "cvm-proxy", "cvm-sqlite", "cvm-runtime", "cvm-web"]);
export function grafanaLink(base: string, uid: string, taskKey?: string): string | undefined {
  if (!DASHBOARDS.has(uid)) return undefined;
  try {
    const url = new URL(base);
    if (url.protocol !== "https:" || url.username || url.password || url.search || url.hash)
      return undefined;
    url.pathname = `${url.pathname.replace(/\/$/, "")}/d/${uid}`;
    url.searchParams.set("from", "now-30m");
    url.searchParams.set("to", "now");
    url.searchParams.set("timezone", "utc");
    if (taskKey) url.searchParams.set("var-task_key", taskKey);
    return url.toString();
  } catch {
    return undefined;
  }
}
