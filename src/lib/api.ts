import { invoke } from "@tauri-apps/api/core";

export type ItemKind = "reminder" | "issue" | "pullRequest" | "draftIssue";

export interface Category {
  id: string;
  name: string;
  color: string;
  label: string;
  position: number;
}

export interface Account {
  id: string;
  login: string;
  host: string;
  name: string | null;
  avatarUrl: string | null;
  scopes: string[];
}

export interface Repo {
  id: string;
  accountId: string;
  owner: string;
  name: string;
  url: string;
  defaultCategory: string | null;
}

export interface Project {
  id: string;
  accountId: string;
  owner: string;
  number: number;
  title: string;
  url: string;
  defaultCategory: string | null;
}

export interface ReminderList {
  id: string;
  title: string;
  color: string | null;
  sourceTitle: string | null;
  writable: boolean;
  enabled: boolean;
  defaultCategory: string | null;
}

export interface Label {
  name: string;
  color: string;
}

export interface Item {
  id: string;
  kind: ItemKind;
  containerId: string;
  containerName: string;
  accountId: string | null;
  title: string;
  body: string | null;
  url: string | null;
  number: number | null;
  completed: boolean;
  due: string | null;
  priority: number;
  labels: Label[];
  assignees: string[];
  status: string | null;
  projectIds: string[];
  updatedAt: string | null;
  /** GitHub sub-issues: the parent issue's item id. */
  parentId: string | null;
  /** GitHub sub-issues: progress of this issue's own sub-issues. */
  subIssues: { total: number; completed: number } | null;
  categories: string[];
}

export interface SourceStatus {
  sourceId: string;
  label: string;
  lastSynced: string | null;
  error: string | null;
  syncing: boolean;
}

export type RemindersAccess = "granted" | "writeOnly" | "denied" | "restricted" | "notDetermined" | "unsupported";

export interface Snapshot {
  categories: Category[];
  accounts: Account[];
  repos: Repo[];
  projects: Project[];
  reminderLists: ReminderList[];
  remindersAccess: RemindersAccess;
  items: Item[];
  sources: SourceStatus[];
}

export interface Candidate {
  id: string;
  owner: string;
  name: string;
  number: number | null;
  url: string;
  description: string | null;
}

export type NewItem =
  | {
      target: "reminder";
      listId: string;
      title: string;
      notes?: string | null;
      due?: string | null;
      priority?: number;
      category?: string | null;
    }
  | {
      target: "issue";
      repoId: string;
      title: string;
      body?: string | null;
      category?: string | null;
      extraLabels?: string[];
      projectId?: string | null;
      assignSelf?: boolean;
    };

/** Editor changes; omitted fields are left alone, "" clears `body`/`due`. */
export interface ItemPatch {
  title?: string;
  body?: string;
  due?: string;
  priority?: number;
  listId?: string;
  labels?: string[];
  assignees?: string[];
}

export interface ItemDetails {
  item: Item;
  labelOptions: Label[];
  assigneeOptions: string[];
}

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  syncNow: () => invoke<void>("sync_now"),

  addAccount: (token: string, host?: string) => invoke<Account>("add_account", { token, host }),
  importGhAccounts: () => invoke<string[]>("import_gh_accounts"),
  removeAccount: (id: string) => invoke<void>("remove_account", { id }),

  repoCandidates: (accountId: string) => invoke<Candidate[]>("repo_candidates", { accountId }),
  projectCandidates: (accountId: string) => invoke<Candidate[]>("project_candidates", { accountId }),
  lookupRepo: (accountId: string, spec: string) => invoke<Candidate>("lookup_repo", { accountId, spec }),
  lookupProject: (accountId: string, spec: string) => invoke<Candidate>("lookup_project", { accountId, spec }),
  addRepo: (accountId: string, candidate: Candidate, category: string | null) =>
    invoke<void>("add_repo", { accountId, candidate, category }),
  addProject: (accountId: string, candidate: Candidate, category: string | null) =>
    invoke<void>("add_project", { accountId, candidate, category }),
  removeSource: (id: string) => invoke<void>("remove_source", { id }),
  setDefaultCategory: (containerId: string, category: string | null) =>
    invoke<void>("set_default_category", { containerId, category }),

  requestRemindersAccess: () => invoke<RemindersAccess>("request_reminders_access"),
  setListEnabled: (id: string, enabled: boolean) => invoke<void>("set_list_enabled", { id, enabled }),

  upsertCategory: (category: Category) => invoke<Category>("upsert_category", { category }),
  deleteCategory: (id: string) => invoke<void>("delete_category", { id }),

  createItem: (item: NewItem) => invoke<Item>("create_item", { item }),
  setCompleted: (id: string, completed: boolean) => invoke<void>("set_completed", { id, completed }),
  setCategory: (id: string, category: string | null) => invoke<void>("set_category", { id, category }),
  itemDetails: (id: string) => invoke<ItemDetails>("item_details", { id }),
  updateItem: (id: string, patch: ItemPatch) => invoke<Item>("update_item", { id, patch }),
  openItem: (id: string) => invoke<void>("open_item", { id }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
};

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
