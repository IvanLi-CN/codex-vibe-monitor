import type { ManagedTask } from "../../lib/api";

export function managedTaskColor(task: ManagedTask | undefined, dark: boolean): string {
  const saved = dark ? task?.displayColorDark : task?.displayColorLight;
  if (saved) return saved;
  const hue =
    [...(task?.taskKey ?? "unknown")].reduce((total, char) => total + char.charCodeAt(0), 0) % 360;
  return `hsl(${hue} 68% ${dark ? "68%" : "42%"})`;
}
