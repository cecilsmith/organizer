# Organizer

A macOS app that puts your Apple Reminders and GitHub work in one list, sorted into areas: **Personal**, **Business**, **School** and **Fun**. You can add more areas.

- **Apple Reminders:** reads and writes your reminders directly through EventKit. Changes you make in the Reminders app show up here right away.
- **GitHub:** link several accounts (github.com or Enterprise), then track any number of repositories (open issues) and Projects (issues, PRs and drafts, with their Status).
- **Areas from labels:** an item goes into an area when it has that area's label. On GitHub that's an issue label like `school`. In Reminders it's a hashtag like `#school` in the title or notes. Anything without a label falls back to the default area of its list, repo or project. Items with no area at all go to the **Inbox**.
- **Edit in place:** double-click any item (or select it and press Enter) to edit it in a side panel. For reminders you can change the title, notes, list, due date, time and priority. For issues and PRs you can change the title, description, labels and assignees, and for project drafts the title and body. Changing the area edits the right thing underneath: GitHub labels, or a reminder's list or hashtag.
- **Create anywhere:** the quick-add bar creates a reminder in any list, or a GitHub issue in any tracked repo. A GitHub issue gets the current area's label and can also be added to a project.

## Architecture

```
src-tauri/            Rust backend (Tauri 2)
  src/reminders.rs    EventKit bridge on a dedicated thread (objc2-event-kit)
  src/github.rs       GraphQL reads, REST writes (reqwest)
  src/db.rs           SQLite cache + config (rusqlite, WAL)
  src/categorize.rs   label / hashtag / default → area rules
  src/app.rs          snapshot merging, background sync, actions
  src/secrets.rs      Keychain token storage, gh CLI import
src/                  Svelte 5 frontend
```

Performance comes from three things:

- The UI reads everything from a local SQLite cache, so the app opens instantly and also works offline.
- Syncs run in the background: GitHub every 2 minutes and whenever the window regains focus, with up to 6 sources in parallel. Reminders sync when EventKit sends a change notification.
- Completing an item or changing its area updates the screen immediately. The change is undone if the sync fails.

GitHub tokens are stored in the macOS Keychain, never in the database.

## Run it

Requirements: macOS 14+, Rust, Node 20+.

```bash
npm install
```

```bash
npm run tauri dev
```

To work on the UI in a browser with sample data (no Rust, Reminders or GitHub needed), run `npm run dev` and open http://127.0.0.1:1420. The fake backend lives in `src/lib/dev/mock.ts` and is left out of production builds.

Build a `.app` / `.dmg`:

```bash
npm run tauri build
```

## Connect your stuff

Open **Settings** (⌘,).

1. **Apple Reminders:** click **Connect Apple Reminders** and allow access. Then give each list a default area.
2. **GitHub accounts:** click **Import gh accounts** to use every account the `gh` CLI is logged into. You can also paste a personal access token with the `repo` and `project` scopes. Projects need the `project` scope; for `gh`, run:

   ```bash
   gh auth refresh -s project
   ```

3. **Repositories / Projects:** browse or paste `owner/repo` (or a project URL), and pick a default area.
4. **Areas:** rename areas, recolor them, change their labels, or add new ones.
5. **Display:** choose whether reminder notes show under every reminder (first two lines) or only for the one you've selected.

## Keyboard

| Key | Action |
| --- | --- |
| ⌘1 … ⌘9 | All, Inbox, then each area |
| ⌘N / `n` | New to-do |
| ⌘F / `/` | Search (`#123` matches an issue number) |
| ↑ ↓ / `j` `k` | Move selection |
| Space / `x` | Complete or uncomplete |
| Enter / `e` / double-click | Edit |
| `o` | Open in Reminders or on GitHub |
| ⌘S / ⌘Enter | Save (in the editor) |
| Esc | Close the editor (press twice to discard changes) |
| `m` | Move to area |
| ⌘R | Sync now |

## Tests

```bash
cd src-tauri && cargo test
```

```bash
npm run check
```
