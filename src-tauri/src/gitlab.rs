//! GitLab client (gitlab.com or self-managed) over the REST API v4.
//!
//! GitLab projects are stored as `Repo`s with ids like `gl:<host>:<project id>`
//! and their issues as `Item`s with ids like `gl:<host>:<issue id>`, so they
//! never collide with GitHub ids.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::github::cached_body;
use crate::model::{Account, Candidate, Item, ItemKind, Label, Provider, Repo};

const MAX_PAGES: usize = 10;

#[derive(Clone)]
pub struct GitLab {
    http: reqwest::Client,
}

impl Default for GitLab {
    fn default() -> Self {
        Self::new()
    }
}

fn api(host: &str) -> String {
    format!("https://{host}/api/v4")
}

/// `gl:<host>:<id>` for repos.
pub fn repo_id(host: &str, project_id: i64) -> String {
    format!("gl:{host}:{project_id}")
}

/// The numeric GitLab project id from a repo id.
pub fn project_id(repo_id: &str) -> Result<i64> {
    repo_id
        .rsplit(':')
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| Error::msg(format!("not a GitLab project id: {repo_id}")))
}

#[derive(Deserialize)]
struct GlIssue {
    id: i64,
    iid: i64,
    title: String,
    description: Option<String>,
    state: String,
    web_url: String,
    updated_at: Option<String>,
    due_date: Option<String>,
    #[serde(default)]
    labels: Vec<GlLabel>,
    #[serde(default)]
    assignees: Vec<GlUser>,
    milestone: Option<GlMilestone>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GlLabel {
    /// With `with_labels_details=true`.
    Detailed { name: String, color: String },
    Plain(String),
}

#[derive(Deserialize)]
struct GlUser {
    username: String,
}

#[derive(Deserialize)]
struct GlMilestone {
    due_date: Option<String>,
}

/// An issue with its full description (the cache truncates bodies).
pub struct IssueDetails {
    pub item: Item,
    pub body: Option<String>,
}

impl GitLab {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("organizer/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        Self { http }
    }

    async fn call(
        &self,
        method: reqwest::Method,
        host: &str,
        token: &str,
        path: &[&str],
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<(Value, Option<String>)> {
        let mut url = reqwest::Url::parse(&api(host)).map_err(|e| Error::msg(e.to_string()))?;
        url.path_segments_mut().map_err(|_| Error::msg("bad URL"))?.extend(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let mut req = self.http.request(method, url).header("PRIVATE-TOKEN", token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let next_page = resp
            .headers()
            .get("x-next-page")
            .and_then(|h| h.to_str().ok())
            .filter(|s| !s.is_empty())
            .map(String::from);
        let text = resp.text().await?;
        let value: Value = if text.is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::Null) };
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::GitHub("GitLab rejected the token — check it has the `api` scope".into()));
        }
        if !status.is_success() {
            let msg = value["message"]
                .as_str()
                .map(String::from)
                .or_else(|| value["error"].as_str().map(String::from))
                .or_else(|| value["message"].as_object().map(|_| value["message"].to_string()))
                .unwrap_or_else(|| "request failed".into());
            return Err(Error::Msg(format!("GitLab: {status}: {msg}")));
        }
        Ok((value, next_page))
    }

    async fn get(&self, host: &str, token: &str, path: &[&str], query: &[(&str, &str)]) -> Result<Value> {
        Ok(self.call(reqwest::Method::GET, host, token, path, query, None).await?.0)
    }

    /// Follows `x-next-page` through up to `MAX_PAGES` pages of 100.
    async fn get_all(&self, host: &str, token: &str, path: &[&str], query: &[(&str, &str)]) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut page = "1".to_string();
        for _ in 0..MAX_PAGES {
            let mut q: Vec<(&str, &str)> = query.to_vec();
            q.push(("per_page", "100"));
            q.push(("page", &page));
            let (value, next) = self.call(reqwest::Method::GET, host, token, path, &q, None).await?;
            out.extend(value.as_array().cloned().unwrap_or_default());
            match next {
                Some(n) => page = n,
                None => break,
            }
        }
        Ok(out)
    }

    // ---- accounts & discovery ---------------------------------------------

    pub async fn viewer(&self, host: &str, token: &str) -> Result<Account> {
        let v = self.get(host, token, &["user"], &[]).await?;
        let login = v["username"]
            .as_str()
            .ok_or_else(|| Error::msg(format!("unexpected GitLab response: {v}")))?
            .to_string();
        // Personal access tokens can report their scopes; other token types can't.
        let scopes = match self.get(host, token, &["personal_access_tokens", "self"], &[]).await {
            Ok(t) => t["scopes"]
                .as_array()
                .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            Err(_) => vec![],
        };
        Ok(Account {
            id: format!("{login}@{host}"),
            login,
            host: host.into(),
            name: v["name"].as_str().map(Into::into),
            avatar_url: v["avatar_url"].as_str().map(Into::into),
            scopes,
            provider: Provider::Gitlab,
        })
    }

    /// Projects you're a member of, most recently active first.
    pub async fn projects(&self, host: &str, token: &str) -> Result<Vec<Candidate>> {
        let values = self
            .get_all(
                host,
                token,
                &["projects"],
                &[("membership", "true"), ("archived", "false"), ("order_by", "last_activity_at"), ("simple", "true")],
            )
            .await?;
        Ok(values.iter().take(300).filter_map(|p| candidate(host, p)).collect())
    }

    /// Looks up `group/subgroup/project`.
    pub async fn project(&self, host: &str, token: &str, path: &str) -> Result<Candidate> {
        let v = self.get(host, token, &["projects", path], &[]).await?;
        candidate(host, &v).ok_or_else(|| Error::msg(format!("GitLab project {path} not found")))
    }

    // ---- issues -----------------------------------------------------------

    pub async fn issues(&self, host: &str, token: &str, repo: &Repo) -> Result<Vec<Item>> {
        let pid = project_id(&repo.id)?.to_string();
        let values = self
            .get_all(
                host,
                token,
                &["projects", &pid, "issues"],
                &[("state", "opened"), ("with_labels_details", "true"), ("order_by", "updated_at")],
            )
            .await?;
        values
            .into_iter()
            .map(|v| Ok(to_item(host, repo, &account_of(repo), serde_json::from_value(v)?)))
            .collect()
    }

    pub async fn issue(&self, host: &str, token: &str, repo: &Repo, iid: i64) -> Result<IssueDetails> {
        let pid = project_id(&repo.id)?.to_string();
        let v = self
            .get(host, token, &["projects", &pid, "issues", &iid.to_string()], &[("with_labels_details", "true")])
            .await?;
        let issue: GlIssue = serde_json::from_value(v)?;
        let body = issue.description.clone().filter(|d| !d.trim().is_empty());
        Ok(IssueDetails { item: to_item(host, repo, &account_of(repo), issue), body })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_issue(
        &self,
        host: &str,
        token: &str,
        repo: &Repo,
        title: &str,
        description: Option<&str>,
        labels: &[String],
        assign_self: bool,
    ) -> Result<Item> {
        let pid = project_id(&repo.id)?.to_string();
        let mut body = json!({ "title": title, "description": description, "labels": labels.join(",") });
        if assign_self {
            let me = self.get(host, token, &["user"], &[]).await?;
            body["assignee_ids"] = json!([me["id"]]);
        }
        let (v, _) = self
            .call(reqwest::Method::POST, host, token, &["projects", &pid, "issues"], &[], Some(body))
            .await?;
        Ok(to_item(host, repo, &account_of(repo), serde_json::from_value(v)?))
    }

    /// Edits an issue. `fields` uses GitLab's names: `title`, `description`,
    /// `labels` (comma-separated), `assignee_ids`, `due_date`, `state_event`.
    pub async fn update_issue(&self, host: &str, token: &str, repo: &Repo, iid: i64, fields: Value) -> Result<IssueDetails> {
        let pid = project_id(&repo.id)?.to_string();
        let (v, _) = self
            .call(reqwest::Method::PUT, host, token, &["projects", &pid, "issues", &iid.to_string()], &[], Some(fields))
            .await?;
        let issue: GlIssue = serde_json::from_value(v)?;
        let body = issue.description.clone().filter(|d| !d.trim().is_empty());
        Ok(IssueDetails { item: to_item(host, repo, &account_of(repo), issue), body })
    }

    pub async fn labels(&self, host: &str, token: &str, repo: &Repo) -> Result<Vec<Label>> {
        let pid = project_id(&repo.id)?.to_string();
        let values = self.get_all(host, token, &["projects", &pid, "labels"], &[]).await?;
        Ok(values
            .iter()
            .filter_map(|l| {
                Some(Label {
                    name: l["name"].as_str()?.into(),
                    color: l["color"].as_str().unwrap_or_default().trim_start_matches('#').into(),
                })
            })
            .collect())
    }

    /// `(user id, username)` for everyone who can be assigned.
    pub async fn members(&self, host: &str, token: &str, repo: &Repo) -> Result<Vec<(i64, String)>> {
        let pid = project_id(&repo.id)?.to_string();
        let values = self.get_all(host, token, &["projects", &pid, "members", "all"], &[]).await?;
        Ok(values
            .iter()
            .filter_map(|m| Some((m["id"].as_i64()?, m["username"].as_str()?.to_string())))
            .collect())
    }
}

fn account_of(repo: &Repo) -> String {
    repo.account_id.clone()
}

fn candidate(host: &str, p: &Value) -> Option<Candidate> {
    if p["issues_enabled"] == false {
        return None;
    }
    let id = p["id"].as_i64()?;
    let path = p["path_with_namespace"].as_str()?;
    let (owner, name) = path.rsplit_once('/')?;
    Some(Candidate {
        id: repo_id(host, id),
        owner: owner.into(),
        name: name.into(),
        number: None,
        url: p["web_url"].as_str().unwrap_or_default().into(),
        description: p["description"].as_str().filter(|d| !d.is_empty()).map(Into::into),
    })
}

fn to_item(host: &str, repo: &Repo, account_id: &str, i: GlIssue) -> Item {
    Item {
        id: format!("gl:{host}:{}", i.id),
        kind: ItemKind::Issue,
        provider: Provider::Gitlab,
        container_id: repo.id.clone(),
        container_name: repo.full_name(),
        account_id: Some(account_id.into()),
        title: i.title,
        body: cached_body(i.description.as_deref()),
        url: Some(i.web_url),
        number: Some(i.iid),
        completed: i.state == "closed",
        due: i.due_date.or(i.milestone.and_then(|m| m.due_date)),
        priority: 0,
        labels: i
            .labels
            .into_iter()
            .map(|l| match l {
                GlLabel::Detailed { name, color } => Label { name, color: color.trim_start_matches('#').into() },
                GlLabel::Plain(name) => Label { name, color: String::new() },
            })
            .collect(),
        assignees: i.assignees.into_iter().map(|a| a.username).collect(),
        status: None,
        project_ids: vec![],
        updated_at: i.updated_at,
        start: None,
        project_dates: None,
        parent_id: None,
        sub_issues: None,
        categories: vec![],
    }
}

/// Assignee ids for usernames, from the project's member list.
pub fn assignee_ids(members: &[(i64, String)], usernames: &[String]) -> Result<Vec<i64>> {
    usernames
        .iter()
        .map(|u| {
            members
                .iter()
                .find(|(_, name)| name.eq_ignore_ascii_case(u))
                .map(|(id, _)| *id)
                .ok_or_else(|| Error::msg(format!("{u} isn't a member of this GitLab project")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> Repo {
        Repo {
            id: repo_id("gitlab.com", 42),
            account_id: "me@gitlab.com".into(),
            owner: "group/sub".into(),
            name: "app".into(),
            url: String::new(),
            default_category: None,
            provider: Provider::Gitlab,
        }
    }

    #[test]
    fn repo_ids_round_trip() {
        assert_eq!(project_id(&repo_id("git.example.com", 7)).unwrap(), 7);
        assert!(project_id("R_kgDO").is_err());
    }

    #[test]
    fn parses_issues_with_detailed_or_plain_labels() {
        let v = json!({
            "id": 1001, "iid": 12, "project_id": 42, "title": "Fix login", "description": "Steps…",
            "state": "opened", "web_url": "https://gitlab.com/group/sub/app/-/issues/12",
            "updated_at": "2026-10-01T00:00:00Z", "due_date": null,
            "labels": [{ "name": "school", "color": "#9b6dff" }, "bug"],
            "assignees": [{ "id": 5, "username": "me" }],
            "milestone": { "due_date": "2026-11-01" }
        });
        let it = to_item("gitlab.com", &repo(), "me@gitlab.com", serde_json::from_value(v).unwrap());
        assert_eq!(it.id, "gl:gitlab.com:1001");
        assert_eq!(it.number, Some(12));
        assert_eq!(it.container_name, "group/sub/app");
        assert_eq!(it.labels[0], Label { name: "school".into(), color: "9b6dff".into() });
        assert_eq!(it.labels[1].name, "bug");
        assert_eq!(it.due.as_deref(), Some("2026-11-01"));
        assert_eq!(it.provider, Provider::Gitlab);
    }

    #[test]
    fn maps_usernames_to_member_ids() {
        let members = vec![(5, "me".to_string()), (9, "Pat".to_string())];
        assert_eq!(assignee_ids(&members, &["pat".into(), "me".into()]).unwrap(), vec![9, 5]);
        assert!(assignee_ids(&members, &["ghost".into()]).is_err());
    }

    #[test]
    fn skips_projects_without_issues() {
        let p = json!({ "id": 1, "path_with_namespace": "a/b", "issues_enabled": false });
        assert!(candidate("gitlab.com", &p).is_none());
        let p = json!({ "id": 1, "path_with_namespace": "a/b/c", "web_url": "u" });
        let c = candidate("gitlab.com", &p).unwrap();
        assert_eq!((c.owner.as_str(), c.name.as_str()), ("a/b", "c"));
    }
}
