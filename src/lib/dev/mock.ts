// Fake backend for working on the UI in a plain browser (`npm run dev`).
// Only loaded in dev builds when the page isn't running inside Tauri.

import { mockIPC } from "@tauri-apps/api/mocks";
import type { Category, Item, Snapshot } from "$lib/api";
import { todayStr as day } from "$lib/store.svelte";

const categories: Category[] = [
  { id: "personal", name: "Personal", color: "#4f8cff", label: "personal", position: 0 },
  { id: "business", name: "Business", color: "#f2994a", label: "business", position: 1 },
  { id: "school", name: "School", color: "#9b6dff", label: "school", position: 2 },
  { id: "fun", name: "Fun", color: "#27ae60", label: "fun", position: 3 },
];

function item(p: Partial<Item> & Pick<Item, "id" | "title" | "kind">): Item {
  return {
    containerId: "",
    containerName: "",
    accountId: null,
    body: null,
    url: null,
    number: null,
    completed: false,
    due: null,
    priority: 0,
    labels: [],
    assignees: [],
    status: null,
    projectIds: [],
    updatedAt: new Date().toISOString(),
    parentId: null,
    subIssues: null,
    categories: [],
    ...p,
  };
}

const snap: Snapshot = {
  categories,
  accounts: [
    { id: "octo@github.com", login: "octo", host: "github.com", name: "Octo Cat", avatarUrl: null, scopes: ["repo"] },
  ],
  repos: [
    { id: "R1", accountId: "octo@github.com", owner: "octo", name: "thesis", url: "", defaultCategory: "school" },
    { id: "R2", accountId: "octo@github.com", owner: "acme", name: "storefront", url: "", defaultCategory: "business" },
  ],
  projects: [
    { id: "P1", accountId: "octo@github.com", owner: "acme", number: 3, title: "Q4 Roadmap", url: "", defaultCategory: "business" },
  ],
  reminderLists: [
    { id: "L1", title: "Errands", color: "#ff9500", sourceTitle: "iCloud", writable: true, enabled: true, defaultCategory: "personal" },
    { id: "L2", title: "Homework", color: "#af52de", sourceTitle: "iCloud", writable: true, enabled: true, defaultCategory: "school" },
    { id: "L3", title: "Someday", color: "#34c759", sourceTitle: "iCloud", writable: true, enabled: true, defaultCategory: null },
  ],
  remindersAccess: "granted",
  items: [
    item({ id: "rem:1", kind: "reminder", title: "Pick up dry cleaning", containerId: "L1", containerName: "Errands", body: "Ticket 4417, they close at 6pm.\nAsk about the stain on the blue shirt.\nPay with card.", due: day(-1), categories: ["personal"] }),
    item({ id: "rem:2", kind: "reminder", title: "Problem set 4", containerId: "L2", containerName: "Homework", due: `${day(0)}T23:59`, priority: 1, body: "Questions 1–7, show work for #5.", categories: ["school"] }),
    item({ id: "rem:3", kind: "reminder", title: "Book climbing gym #fun", containerId: "L3", containerName: "Someday", categories: ["fun"] }),
    item({ id: "rem:4", kind: "reminder", title: "Renew passport", containerId: "L3", containerName: "Someday", body: "#school", due: day(12) }),
    item({ id: "gh:I1", kind: "issue", title: "Write related-work section", containerId: "R1", containerName: "octo/thesis", number: 12, accountId: "octo@github.com", labels: [{ name: "writing", color: "0e8a16" }], due: day(3), categories: ["school"] }),
    item({ id: "gh:I2", kind: "issue", title: "Checkout fails on Safari 18", containerId: "R2", containerName: "acme/storefront", number: 231, accountId: "octo@github.com", labels: [{ name: "bug", color: "d73a4a" }, { name: "business", color: "f2994a" }], status: "In progress", projectIds: ["P1"], assignees: ["octo"], subIssues: { total: 3, completed: 1 }, categories: ["business"] }),
    item({ id: "gh:I3", kind: "issue", title: "Reproduce on iOS Safari", containerId: "R2", containerName: "acme/storefront", number: 232, accountId: "octo@github.com", parentId: "gh:I2", categories: ["business"] }),
    item({ id: "gh:I4", kind: "issue", title: "Fix payment form validation", containerId: "R2", containerName: "acme/storefront", number: 233, accountId: "octo@github.com", parentId: "gh:I2", due: day(2), labels: [{ name: "bug", color: "d73a4a" }], categories: ["business"] }),
    item({ id: "gh:PR1", kind: "pullRequest", title: "Add Apple Pay", containerId: "R2", containerName: "acme/storefront", number: 240, accountId: "octo@github.com", status: "In review", projectIds: ["P1"], categories: ["business"] }),
    item({ id: "ghpi:D1", kind: "draftIssue", title: "Plan holiday sale", containerId: "P1", containerName: "Q4 Roadmap", accountId: "octo@github.com", status: "Todo", categories: ["business"] }),
  ],
  sources: [
    { sourceId: "reminders", label: "Apple Reminders", lastSynced: new Date().toISOString(), error: null, syncing: false },
    { sourceId: "R1", label: "octo/thesis", lastSynced: new Date().toISOString(), error: null, syncing: false },
    { sourceId: "R2", label: "acme/storefront", lastSynced: new Date().toISOString(), error: null, syncing: false },
    { sourceId: "P1", label: "Q4 Roadmap", lastSynced: null, error: "GitHub: missing the `project` scope", syncing: false },
  ],
};

export function installMock() {
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "get_snapshot":
          return structuredClone(snap);
        case "set_completed": {
          const it = snap.items.find((i) => i.id === a.id);
          if (it) it.completed = a.completed as boolean;
          return null;
        }
        case "set_category": {
          const it = snap.items.find((i) => i.id === a.id);
          if (it) it.categories = a.category ? [a.category as string] : [];
          return null;
        }
        case "item_details": {
          const it = snap.items.find((i) => i.id === a.id);
          return new Promise((resolve) =>
            setTimeout(
              () =>
                resolve({
                  item: structuredClone(it),
                  labelOptions: [
                    { name: "bug", color: "d73a4a" },
                    { name: "enhancement", color: "a2eeef" },
                    { name: "writing", color: "0e8a16" },
                    ...categories.map((c) => ({ name: c.label, color: c.color.slice(1) })),
                  ],
                  assigneeOptions: ["octo", "hubot", "monalisa"],
                }),
              300,
            ),
          );
        }
        case "update_item": {
          const it = snap.items.find((i) => i.id === a.id);
          const p = a.patch as Record<string, unknown>;
          if (!it) throw "item not found";
          if (typeof p.title === "string") it.title = p.title;
          if (typeof p.body === "string") it.body = p.body || null;
          if (typeof p.due === "string") it.due = p.due || null;
          if (typeof p.priority === "number") it.priority = p.priority;
          if (typeof p.listId === "string") {
            const list = snap.reminderLists.find((l) => l.id === p.listId)!;
            it.containerId = list.id;
            it.containerName = list.title;
          }
          if (Array.isArray(p.labels)) it.labels = p.labels.map((name: string) => ({ name, color: "ededed" }));
          if (Array.isArray(p.assignees)) it.assignees = p.assignees as string[];
          // Re-derive areas the way the backend would.
          const tags = it.kind === "reminder"
            ? [...`${it.title} ${it.body ?? ""}`.matchAll(/(?:^|\s)#([\w:-]+)/g)].map((m) => m[1].toLowerCase())
            : it.labels.map((l) => l.name.toLowerCase());
          const fromTags = categories.filter((c) => tags.includes(c.label)).map((c) => c.id);
          const listDefault = snap.reminderLists.find((l) => l.id === it.containerId)?.defaultCategory;
          const repoDefault = snap.repos.find((r) => r.id === it.containerId)?.defaultCategory;
          it.categories = fromTags.length ? fromTags : [listDefault ?? repoDefault].filter(Boolean) as string[];
          return structuredClone(it);
        }
        case "create_item": {
          const req = a.item as { target: string; title: string; listId?: string; repoId?: string; category?: string };
          const list = snap.reminderLists.find((l) => l.id === req.listId);
          const repo = snap.repos.find((r) => r.id === req.repoId);
          const it = item({
            id: `mock:${Date.now()}`,
            kind: req.target === "reminder" ? "reminder" : "issue",
            title: req.title,
            containerId: list?.id ?? repo?.id ?? "",
            containerName: list?.title ?? `${repo?.owner}/${repo?.name}`,
            categories: req.category ? [req.category] : [],
          });
          snap.items.push(it);
          return it;
        }
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
