# Organizer

A macOS app that puts your Apple Reminders, calendars, and GitHub and GitLab work in one place, sorted into areas: **Personal**, **Business**, **School** and **Fun**. You can add more areas.

- **Apple Reminders:** reads and writes your reminders directly through EventKit. Changes you make in the Reminders app show up here right away.
- **Calendars:** events from every calendar on your Mac (iCloud, plus Google/Gmail and Outlook/Microsoft 365 accounts added in System Settings → Internet Accounts) appear in the Calendar views with their real durations. Create them from quick-add, edit them in a side panel, drag them to another time or day, and drag their bottom edge to change when they end. Repeating events change one occurrence at a time.
- **GitHub:** link several accounts (github.com or Enterprise), then track any number of repositories (open issues) and Projects (issues, PRs and drafts, with their Status).
- **GitLab:** link gitlab.com or self-managed accounts — imported from the `glab` CLI or with a personal access token (`api` scope) — and track any number of projects. Issues get areas from labels like GitHub issues do, can be created, closed, edited and relabeled, and their due dates can be changed in the editor, Calendar and Gantt chart.
- **Areas from labels:** an item goes into an area when it has that area's label. On GitHub that's an issue label like `school`. In Reminders it's a hashtag like `#school` in the title or notes. Anything without a label falls back to the default area of its list, repo or project. Items with no area at all go to the **Inbox**.
- **Edit in place:** double-click any item (or select it and press Enter) to edit it in a side panel. For reminders you can change the title, notes, list, due date, time and priority. For issues and PRs you can change the title, description, labels and assignees, and for project drafts the title and body. Changing the area edits the right thing underneath: GitHub labels, or a reminder's list or hashtag.
- **Calendar:** switch from List to Calendar (or press `V`) for Month, Week and Day views of everything with a date. Timed reminders appear at their time; GitHub issues use their milestone due date or a date field from their Project (e.g. "Due" or "Target date"). Drag reminders between days or onto an hour to reschedule them, drag them from the "No date" tray to schedule them, and double-click a day or time slot to add a to-do there.
- **Gantt chart:** a timeline of everything with dates, grouped by project/list or by area, at Days, Weeks or Months zoom. GitHub Project items get bars from the project's start and due date fields (or their iteration); reminders and milestone-dated issues show as ◆ on their due date. Drag a bar to move it, drag its edges to change the start or end, or drag a ◆ to change a due date. Changes to GitHub items are written back to the project's date fields, and everything can be undone.
- **Create anywhere:** the quick-add bar creates a reminder in any list, or a GitHub issue in any tracked repo. A GitHub issue gets the current area's label and can also be added to a project.

## Architecture

```
src-tauri/            Rust backend (Tauri 2)
  src/reminders.rs    EventKit bridge (reminders + calendar events) on a dedicated thread
  src/github.rs       GraphQL reads, REST writes (reqwest)
  src/gitlab.rs       GitLab REST API v4
  src/db.rs           SQLite cache + config (rusqlite, WAL)
  src/categorize.rs   label / hashtag / default → area rules
  src/app.rs          snapshot merging, background sync, actions
  src/secrets.rs      Keychain token storage, gh / glab CLI import
src/                  Svelte 5 frontend
```

Performance comes from three things:

- The UI reads everything from a local SQLite cache, so the app opens instantly and also works offline.
- Syncs run in the background: GitHub every 2 minutes and whenever the window regains focus, with up to 6 sources in parallel. Reminders sync when EventKit sends a change notification.
- Completing an item or changing its area updates the screen immediately. The change is undone if the sync fails.

Tokens never go in the database. A token you paste is stored in the macOS
Keychain; a token imported from `gh` or `glab` isn't stored at all, since those
CLIs already hold it — Organizer re-reads it from them at launch and keeps it in
memory for that run only. Either way a token is read once per run.

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

### Keychain prompts while developing

If macOS keeps asking whether Organizer may use "your confidential information
stored in com.nathanaelsmith.organizer.github", it's the code signature, not the
app. `cargo build` leaves the binary ad-hoc signed, and an ad-hoc signature is
derived from the binary itself, so every rebuild looks like a different app and
the earlier "Always Allow" no longer applies.

Two ways out, which combine:

- **Import accounts from `gh`/`glab`** instead of pasting tokens. Those accounts
  keep nothing in the Keychain, so they never prompt. Settings shows which
  accounts are which.
- **Give dev builds a signature that stays put.** Create a self-signed
  certificate once (Keychain Access > Certificate Assistant > Create a
  Certificate…, identity type "Self Signed Root", certificate type "Code
  Signing") named `Organizer Dev`, then run:

  ```bash
  npm run dev:signed
  ```

  It builds, signs and launches, starting the vite server if it isn't already
  up. `ORGANIZER_SIGN_IDENTITY` picks a different certificate.

To clear out tokens an earlier build stored, delete them from the Keychain:

```bash
security delete-generic-password -s com.nathanaelsmith.organizer.github
```

Repeat it once per account — it deletes one item per run — then re-import.

## Connect your stuff

Open **Settings** (⌘,).

1. **Apple Reminders:** click **Connect Apple Reminders** and allow access. Then give each list a default area.
1. **Calendars:** click **Connect calendars** and allow access, then give each calendar a default area. To add Gmail or Outlook, use **Open Internet Accounts** and turn on Calendars for that account.
2. **Accounts:** for GitHub, click **Import gh accounts** to use every account the `gh` CLI is logged into. You can also paste a personal access token with the `repo` and `project` scopes. Projects need the `project` scope; for `gh`, run:

   ```bash
   gh auth refresh -s project
   ```

   The `project` scope is also what lets the Gantt chart write dates back to your projects.

   For GitLab, click **Import glab accounts** to use every host the `glab` CLI is logged into:

   ```bash
   glab auth login
   ```

   Or paste a personal access token with the `api` scope (change the host for a self-managed server).
3. **Repositories / Projects:** browse or paste `owner/repo`, a GitLab `group/project`, or a URL, and pick a default area.
4. **Areas:** rename areas, recolor them, change their labels, or add new ones.
5. **Display:** choose whether reminder notes show under every reminder (first two lines) or only for the one you've selected, and whether calendar weeks start on Sunday or Monday.

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
| ⌘Z / Ctrl+Z | Undo the last change (complete, move, edit or add) |
| ⌘⇧Z / Ctrl+Shift+Z | Redo the last undone change |
| `V` | Switch between List, Calendar and Gantt |
| ← → / `h` `l` | Previous / next month, week or day (Calendar) |
| `T` | Jump to today (Calendar, Gantt) |
| ⌘R | Sync now |

## Tests

```bash
cd src-tauri && cargo test
```

```bash
npm run check
```
