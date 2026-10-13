import type { Item, Provider } from "./api";

export const providerName = (p: Provider) => (p === "gitlab" ? "GitLab" : "GitHub");

/** Icon for an item's source: Reminders list, GitHub or GitLab. */
export function sourceIcon(item: Pick<Item, "kind" | "provider">): string {
  if (item.kind === "reminder") return "list";
  return item.provider === "gitlab" ? "gitlab" : "github";
}
