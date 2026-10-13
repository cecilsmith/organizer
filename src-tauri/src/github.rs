//! GitHub client. Reads go through GraphQL (one round trip per 100 items,
//! Projects v2 is GraphQL-only); writes to issues use REST, which creates
//! missing labels on the fly.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::model::{Account, Candidate, Item, ItemKind, Label, Project, Repo, SubIssues};

/// Bodies are only used for search and previews; cap them to keep the cache lean.
const MAX_BODY: usize = 4000;
const MAX_PAGES: usize = 10;

#[derive(Clone)]
pub struct GitHub {
    http: reqwest::Client,
}

fn graphql_url(host: &str) -> String {
    if host == "github.com" {
        "https://api.github.com/graphql".into()
    } else {
        format!("https://{host}/api/graphql")
    }
}

fn rest_url(host: &str) -> String {
    if host == "github.com" {
        "https://api.github.com".into()
    } else {
        format!("https://{host}/api/v3")
    }
}

const ISSUE_FIELDS: &str = r#"
fragment IssueFields on Issue {
  id number title body url state updatedAt
  repository { id nameWithOwner }
  labels(first: 20) { nodes { name color } }
  assignees(first: 10) { nodes { login } }
  milestone { dueOn }
  parent { id }
  subIssuesSummary { total completed }
}
"#;

const PR_FIELDS: &str = r#"
fragment PrFields on PullRequest {
  id number title body url state updatedAt
  repository { id nameWithOwner }
  labels(first: 20) { nodes { name color } }
  assignees(first: 10) { nodes { login } }
  milestone { dueOn }
}
"#;

impl Default for GitHub {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHub {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("organizer/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .expect("failed to build HTTP client");
        Self { http }
    }

    // ---- transport --------------------------------------------------------

    /// Runs a query and returns `data` plus any GraphQL error messages.
    async fn graphql_partial(
        &self,
        host: &str,
        token: &str,
        query: &str,
        variables: Value,
    ) -> Result<(Value, Vec<String>)> {
        let resp = self
            .http
            .post(graphql_url(host))
            .bearer_auth(token)
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::GitHub("token is invalid or expired — re-link this account".into()));
        }
        let body: Value = resp.json().await?;
        if !status.is_success() {
            let msg = body["message"].as_str().unwrap_or("request failed");
            return Err(Error::GitHub(format!("{status}: {msg}")));
        }
        let errors = body["errors"]
            .as_array()
            .map(|errs| errs.iter().filter_map(|e| e["message"].as_str().map(String::from)).collect())
            .unwrap_or_default();
        Ok((body["data"].clone(), errors))
    }

    async fn graphql(&self, host: &str, token: &str, query: &str, variables: Value) -> Result<Value> {
        let (data, errors) = self.graphql_partial(host, token, query, variables).await?;
        if !errors.is_empty() {
            return Err(Error::GitHub(explain_scopes(errors.join("; "))));
        }
        Ok(data)
    }

    async fn rest(
        &self,
        method: reqwest::Method,
        host: &str,
        token: &str,
        path: &[&str],
        body: Option<Value>,
    ) -> Result<Value> {
        self.rest_query(method, host, token, path, &[], body).await
    }

    async fn rest_query(
        &self,
        method: reqwest::Method,
        host: &str,
        token: &str,
        path: &[&str],
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<Value> {
        let mut url = reqwest::Url::parse(&rest_url(host)).map_err(|e| Error::msg(e.to_string()))?;
        url.path_segments_mut().map_err(|_| Error::msg("bad URL"))?.extend(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let mut req = self
            .http
            .request(method, url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        let value: Value = if text.is_empty() { Value::Null } else { serde_json::from_str(&text)? };
        if !status.is_success() {
            let msg = value["message"].as_str().unwrap_or("request failed");
            return Err(Error::GitHub(format!("{status}: {msg}")));
        }
        Ok(value)
    }

    // ---- accounts & discovery ---------------------------------------------

    pub async fn viewer(&self, host: &str, token: &str) -> Result<Account> {
        let resp = self
            .http
            .post(graphql_url(host))
            .bearer_auth(token)
            .json(&json!({ "query": "query { viewer { login name avatarUrl } }" }))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::GitHub("that token was rejected".into()));
        }
        // Classic tokens report their scopes; fine-grained tokens don't send this header.
        let scopes = resp
            .headers()
            .get("x-oauth-scopes")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
            .unwrap_or_default();
        let body: Value = resp.json().await?;
        let v = &body["data"]["viewer"];
        let login = v["login"]
            .as_str()
            .ok_or_else(|| Error::GitHub(format!("unexpected response: {body}")))?
            .to_string();
        Ok(Account {
            id: format!("{login}@{host}"),
            login,
            host: host.into(),
            name: v["name"].as_str().map(Into::into),
            avatar_url: v["avatarUrl"].as_str().map(Into::into),
            scopes,
        })
    }

    pub async fn repo(&self, host: &str, token: &str, owner: &str, name: &str) -> Result<Candidate> {
        let data = self
            .graphql(
                host,
                token,
                "query($o: String!, $n: String!) { repository(owner: $o, name: $n) { id name url description owner { login } } }",
                json!({ "o": owner, "n": name }),
            )
            .await?;
        repo_candidate(&data["repository"]).ok_or_else(|| Error::GitHub(format!("{owner}/{name} not found")))
    }

    /// Repos the account owns, collaborates on, or can see through orgs; most recently pushed first.
    pub async fn viewer_repos(&self, host: &str, token: &str) -> Result<Vec<Candidate>> {
        let query = r#"query($after: String) { viewer { repositories(first: 100, after: $after,
            ownerAffiliations: [OWNER, COLLABORATOR, ORGANIZATION_MEMBER],
            orderBy: { field: PUSHED_AT, direction: DESC }) {
              pageInfo { hasNextPage endCursor }
              nodes { id name url description isArchived hasIssuesEnabled owner { login } } } } }"#;
        let mut out = Vec::new();
        let mut after = Value::Null;
        for _ in 0..3 {
            let data = self.graphql(host, token, query, json!({ "after": after })).await?;
            let conn = &data["viewer"]["repositories"];
            for n in conn["nodes"].as_array().into_iter().flatten() {
                if n["isArchived"] == true || n["hasIssuesEnabled"] == false {
                    continue;
                }
                out.extend(repo_candidate(n));
            }
            if conn["pageInfo"]["hasNextPage"] != true {
                break;
            }
            after = conn["pageInfo"]["endCursor"].clone();
        }
        Ok(out)
    }

    /// Projects owned by the account and its orgs. Org errors (e.g. SSO not
    /// authorized) are skipped rather than failing the whole list.
    pub async fn viewer_projects(&self, host: &str, token: &str) -> Result<Vec<Candidate>> {
        let query = r#"query { viewer {
            login
            projectsV2(first: 50, orderBy: { field: UPDATED_AT, direction: DESC }) {
              nodes { id number title url closed shortDescription } }
            organizations(first: 30) { nodes { login
              projectsV2(first: 30, orderBy: { field: UPDATED_AT, direction: DESC }) {
                nodes { id number title url closed shortDescription } } } } } }"#;
        let (data, errors) = self.graphql_partial(host, token, query, json!({})).await?;
        if data["viewer"].is_null() {
            return Err(Error::GitHub(explain_scopes(errors.join("; "))));
        }
        let viewer = &data["viewer"];
        let mut out = Vec::new();
        let owners = std::iter::once((viewer["login"].as_str(), &viewer["projectsV2"])).chain(
            viewer["organizations"]["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| (o["login"].as_str(), &o["projectsV2"])),
        );
        for (owner, conn) in owners {
            for p in conn["nodes"].as_array().into_iter().flatten() {
                if p["closed"] == true {
                    continue;
                }
                out.extend(project_candidate(p, owner.unwrap_or_default()));
            }
        }
        if out.is_empty() && !errors.is_empty() {
            return Err(Error::GitHub(explain_scopes(errors.join("; "))));
        }
        Ok(out)
    }

    pub async fn project_by_number(
        &self,
        host: &str,
        token: &str,
        owner: &str,
        number: i64,
    ) -> Result<Candidate> {
        let data = self
            .graphql(
                host,
                token,
                r#"query($o: String!, $n: Int!) { repositoryOwner(login: $o) {
                    ... on User { projectV2(number: $n) { id number title url shortDescription } }
                    ... on Organization { projectV2(number: $n) { id number title url shortDescription } } } }"#,
                json!({ "o": owner, "n": number }),
            )
            .await?;
        project_candidate(&data["repositoryOwner"]["projectV2"], owner)
            .ok_or_else(|| Error::GitHub(format!("project {owner}#{number} not found")))
    }

    // ---- sync -------------------------------------------------------------

    pub async fn repo_issues(&self, host: &str, token: &str, repo: &Repo) -> Result<Vec<Item>> {
        let query = format!(
            r#"query($o: String!, $n: String!, $after: String) {{ repository(owner: $o, name: $n) {{
                issues(states: OPEN, first: 100, after: $after, orderBy: {{ field: UPDATED_AT, direction: DESC }}) {{
                  pageInfo {{ hasNextPage endCursor }}
                  nodes {{ ...IssueFields }} }} }} }} {ISSUE_FIELDS}"#
        );
        let mut items = Vec::new();
        let mut after = Value::Null;
        for _ in 0..MAX_PAGES {
            let data = self
                .graphql(host, token, &query, json!({ "o": repo.owner, "n": repo.name, "after": after }))
                .await?;
            let conn = &data["repository"]["issues"];
            if conn.is_null() {
                return Err(Error::GitHub(format!("{} is not accessible", repo.full_name())));
            }
            for node in conn["nodes"].as_array().into_iter().flatten() {
                if let Some(mut it) = content_item(node, &repo.account_id) {
                    it.container_id = repo.id.clone();
                    items.push(it);
                }
            }
            if conn["pageInfo"]["hasNextPage"] != true {
                break;
            }
            after = conn["pageInfo"]["endCursor"].clone();
        }
        Ok(items)
    }

    pub async fn project_items(&self, host: &str, token: &str, project: &Project) -> Result<Vec<Item>> {
        let query = format!(
            r#"query($id: ID!, $after: String) {{ node(id: $id) {{ ... on ProjectV2 {{
                items(first: 100, after: $after) {{
                  pageInfo {{ hasNextPage endCursor }}
                  nodes {{
                    id isArchived
                    status: fieldValueByName(name: "Status") {{ ... on ProjectV2ItemFieldSingleSelectValue {{ name }} }}
                    fieldValues(first: 20) {{ nodes {{
                      ... on ProjectV2ItemFieldDateValue {{ date field {{ ... on ProjectV2FieldCommon {{ name }} }} }}
                    }} }}
                    content {{
                      __typename
                      ...IssueFields
                      ...PrFields
                      ... on DraftIssue {{ id title body updatedAt assignees(first: 10) {{ nodes {{ login }} }} }}
                    }} }} }} }} }} }} {ISSUE_FIELDS} {PR_FIELDS}"#
        );
        let mut items = Vec::new();
        let mut after = Value::Null;
        for _ in 0..MAX_PAGES {
            let data = self.graphql(host, token, &query, json!({ "id": project.id, "after": after })).await?;
            let conn = &data["node"]["items"];
            if conn.is_null() {
                return Err(Error::GitHub(format!("project \"{}\" is not accessible", project.title)));
            }
            for node in conn["nodes"].as_array().into_iter().flatten() {
                if node["isArchived"] == true {
                    continue;
                }
                let content = &node["content"];
                let mut item = if content["__typename"] == "DraftIssue" {
                    // Drafts have no URL or repo; address them by project item id so
                    // they can be archived when completed.
                    Item {
                        id: format!("ghpi:{}", node["id"].as_str().unwrap_or_default()),
                        kind: ItemKind::DraftIssue,
                        container_id: project.id.clone(),
                        container_name: project.title.clone(),
                        account_id: Some(project.account_id.clone()),
                        title: str_of(&content["title"]),
                        body: body_of(&content["body"]),
                        url: Some(project.url.clone()),
                        number: None,
                        completed: false,
                        due: None,
                        priority: 0,
                        labels: vec![],
                        assignees: logins(&content["assignees"]),
                        status: None,
                        project_ids: vec![],
                        updated_at: content["updatedAt"].as_str().map(Into::into),
                        parent_id: None,
                        sub_issues: None,
                        categories: vec![],
                    }
                } else {
                    match content_item(content, &project.account_id) {
                        Some(it) if !it.completed => it,
                        _ => continue,
                    }
                };
                item.status = node["status"]["name"].as_str().map(Into::into);
                // A milestone due date wins; otherwise use the project's date field.
                if item.due.is_none() {
                    item.due = project_due_date(&node["fieldValues"]["nodes"]);
                }
                item.project_ids = vec![project.id.clone()];
                items.push(item);
            }
            if conn["pageInfo"]["hasNextPage"] != true {
                break;
            }
            after = conn["pageInfo"]["endCursor"].clone();
        }
        Ok(items)
    }

    // ---- writes -----------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub async fn create_issue(
        &self,
        host: &str,
        token: &str,
        repo: &Repo,
        title: &str,
        body: Option<&str>,
        labels: &[String],
        assignees: &[String],
    ) -> Result<Item> {
        let v = self
            .rest(
                reqwest::Method::POST,
                host,
                token,
                &["repos", &repo.owner, &repo.name, "issues"],
                Some(json!({ "title": title, "body": body, "labels": labels, "assignees": assignees })),
            )
            .await?;
        let rest: RestIssue = serde_json::from_value(v)?;
        Ok(Item {
            id: format!("gh:{}", rest.node_id),
            kind: ItemKind::Issue,
            container_id: repo.id.clone(),
            container_name: repo.full_name(),
            account_id: Some(repo.account_id.clone()),
            title: rest.title,
            body: rest.body.and_then(|b| body_of(&Value::String(b))),
            url: Some(rest.html_url),
            number: Some(rest.number),
            completed: false,
            due: None,
            priority: 0,
            labels: rest.labels.into_iter().map(|l| Label { name: l.name, color: l.color }).collect(),
            assignees: rest.assignees.into_iter().map(|a| a.login).collect(),
            status: None,
            project_ids: vec![],
            updated_at: Some(rest.updated_at),
            parent_id: None,
            sub_issues: None,
            categories: vec![],
        })
    }

    pub async fn add_to_project(&self, host: &str, token: &str, project_id: &str, content_id: &str) -> Result<()> {
        self.graphql(
            host,
            token,
            "mutation($p: ID!, $c: ID!) { addProjectV2ItemById(input: { projectId: $p, contentId: $c }) { item { id } } }",
            json!({ "p": project_id, "c": content_id }),
        )
        .await?;
        Ok(())
    }

    pub async fn archive_project_item(&self, host: &str, token: &str, project_id: &str, item_id: &str) -> Result<()> {
        self.graphql(
            host,
            token,
            "mutation($p: ID!, $i: ID!) { archiveProjectV2Item(input: { projectId: $p, itemId: $i }) { item { id } } }",
            json!({ "p": project_id, "i": item_id }),
        )
        .await?;
        Ok(())
    }

    pub async fn unarchive_project_item(&self, host: &str, token: &str, project_id: &str, item_id: &str) -> Result<()> {
        self.graphql(
            host,
            token,
            "mutation($p: ID!, $i: ID!) { unarchiveProjectV2Item(input: { projectId: $p, itemId: $i }) { item { id } } }",
            json!({ "p": project_id, "i": item_id }),
        )
        .await?;
        Ok(())
    }

    pub async fn set_issue_closed(
        &self,
        host: &str,
        token: &str,
        owner: &str,
        name: &str,
        number: i64,
        closed: bool,
    ) -> Result<()> {
        let body = if closed {
            json!({ "state": "closed", "state_reason": "completed" })
        } else {
            json!({ "state": "open" })
        };
        let n = number.to_string();
        self.rest(reqwest::Method::PATCH, host, token, &["repos", owner, name, "issues", &n], Some(body))
            .await?;
        Ok(())
    }

    /// Closes an issue as "not planned" — used to undo creating one, since
    /// issues can't be deleted with a normal token.
    pub async fn close_not_planned(&self, host: &str, token: &str, owner: &str, name: &str, number: i64) -> Result<()> {
        let n = number.to_string();
        self.rest(
            reqwest::Method::PATCH,
            host,
            token,
            &["repos", owner, name, "issues", &n],
            Some(json!({ "state": "closed", "state_reason": "not_planned" })),
        )
        .await?;
        Ok(())
    }

    pub async fn add_label(&self, host: &str, token: &str, owner: &str, name: &str, number: i64, label: &str) -> Result<()> {
        let n = number.to_string();
        self.rest(
            reqwest::Method::POST,
            host,
            token,
            &["repos", owner, name, "issues", &n, "labels"],
            Some(json!({ "labels": [label] })),
        )
        .await?;
        Ok(())
    }

    pub async fn remove_label(&self, host: &str, token: &str, owner: &str, name: &str, number: i64, label: &str) -> Result<()> {
        let n = number.to_string();
        match self
            .rest(reqwest::Method::DELETE, host, token, &["repos", owner, name, "issues", &n, "labels", label], None)
            .await
        {
            // Already gone is fine.
            Err(Error::GitHub(m)) if m.starts_with("404") => Ok(()),
            r => r.map(|_| ()),
        }
    }

    // ---- editing ----------------------------------------------------------

    /// The full issue (or PR), without the body truncation applied to the cache.
    pub async fn issue(&self, host: &str, token: &str, owner: &str, name: &str, number: i64) -> Result<IssueDetails> {
        let n = number.to_string();
        let v = self
            .rest(reqwest::Method::GET, host, token, &["repos", owner, name, "issues", &n], None)
            .await?;
        Ok(serde_json::from_value::<RestIssue>(v)?.into())
    }

    pub async fn repo_labels(&self, host: &str, token: &str, owner: &str, name: &str) -> Result<Vec<Label>> {
        let v = self
            .rest_query(reqwest::Method::GET, host, token, &["repos", owner, name, "labels"], &[("per_page", "100")], None)
            .await?;
        let labels: Vec<RestLabel> = serde_json::from_value(v)?;
        Ok(labels.into_iter().map(|l| Label { name: l.name, color: l.color }).collect())
    }

    pub async fn repo_assignees(&self, host: &str, token: &str, owner: &str, name: &str) -> Result<Vec<String>> {
        let v = self
            .rest_query(
                reqwest::Method::GET,
                host,
                token,
                &["repos", owner, name, "assignees"],
                &[("per_page", "100")],
                None,
            )
            .await?;
        let users: Vec<RestUser> = serde_json::from_value(v)?;
        Ok(users.into_iter().map(|u| u.login).collect())
    }

    /// Patches an issue or PR. `fields` holds only what changed (title, body, labels, assignees).
    pub async fn update_issue(
        &self,
        host: &str,
        token: &str,
        owner: &str,
        name: &str,
        number: i64,
        fields: Value,
    ) -> Result<IssueDetails> {
        let n = number.to_string();
        let v = self
            .rest(reqwest::Method::PATCH, host, token, &["repos", owner, name, "issues", &n], Some(fields))
            .await?;
        Ok(serde_json::from_value::<RestIssue>(v)?.into())
    }

    /// Resolves a project item to its draft issue: `(draft id, title, body)`.
    pub async fn draft_issue(&self, host: &str, token: &str, project_item_id: &str) -> Result<(String, String, String)> {
        let data = self
            .graphql(
                host,
                token,
                "query($id: ID!) { node(id: $id) { ... on ProjectV2Item { content { ... on DraftIssue { id title body } } } } }",
                json!({ "id": project_item_id }),
            )
            .await?;
        let c = &data["node"]["content"];
        let id = c["id"].as_str().ok_or_else(|| Error::GitHub("draft issue not found".into()))?;
        Ok((id.into(), str_of(&c["title"]), str_of(&c["body"])))
    }

    pub async fn update_draft(&self, host: &str, token: &str, draft_id: &str, title: &str, body: &str) -> Result<()> {
        self.graphql(
            host,
            token,
            "mutation($id: ID!, $t: String!, $b: String!) { updateProjectV2DraftIssue(input: { draftIssueId: $id, title: $t, body: $b }) { draftIssue { id } } }",
            json!({ "id": draft_id, "t": title, "b": body }),
        )
        .await?;
        Ok(())
    }
}

/// An issue as returned by REST, with its untruncated body.
pub struct IssueDetails {
    pub title: String,
    pub body: Option<String>,
    pub labels: Vec<Label>,
    pub assignees: Vec<String>,
    pub updated_at: String,
}

impl From<RestIssue> for IssueDetails {
    fn from(r: RestIssue) -> Self {
        Self {
            title: r.title,
            body: r.body.filter(|b| !b.trim().is_empty()),
            labels: r.labels.into_iter().map(|l| Label { name: l.name, color: l.color }).collect(),
            assignees: r.assignees.into_iter().map(|a| a.login).collect(),
            updated_at: r.updated_at,
        }
    }
}

#[derive(Deserialize)]
struct RestIssue {
    node_id: String,
    number: i64,
    title: String,
    body: Option<String>,
    html_url: String,
    updated_at: String,
    #[serde(default)]
    labels: Vec<RestLabel>,
    #[serde(default)]
    assignees: Vec<RestUser>,
}

#[derive(Deserialize)]
struct RestLabel {
    name: String,
    #[serde(default)]
    color: String,
}

#[derive(Deserialize)]
struct RestUser {
    login: String,
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// Trims and caps a body for the cache.
pub fn cached_body(body: Option<&str>) -> Option<String> {
    body_of(&Value::String(body?.to_string()))
}

fn body_of(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    Some(match s.char_indices().nth(MAX_BODY) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    })
}

fn logins(conn: &Value) -> Vec<String> {
    conn["nodes"].as_array().into_iter().flatten().filter_map(|n| n["login"].as_str().map(Into::into)).collect()
}

/// Picks the project date field that means "when it's due": one named like
/// due/deadline/target/end/date, skipping start dates.
fn project_due_date(values: &Value) -> Option<String> {
    let dates: Vec<(String, String)> = values
        .as_array()?
        .iter()
        .filter_map(|v| Some((v["field"]["name"].as_str()?.to_lowercase(), v["date"].as_str()?.to_string())))
        .collect();
    let rank = |name: &str| {
        if name.contains("start") || name.contains("begin") {
            None
        } else if ["due", "deadline"].iter().any(|k| name.contains(k)) {
            Some(0)
        } else if ["target", "end", "finish"].iter().any(|k| name.contains(k)) {
            Some(1)
        } else if name.contains("date") {
            Some(2)
        } else {
            None
        }
    };
    dates
        .iter()
        .filter_map(|(name, date)| Some((rank(name)?, date)))
        .min_by_key(|(r, _)| *r)
        .map(|(_, d)| d.chars().take(10).collect())
}

fn repo_candidate(n: &Value) -> Option<Candidate> {
    Some(Candidate {
        id: n["id"].as_str()?.into(),
        owner: n["owner"]["login"].as_str()?.into(),
        name: n["name"].as_str()?.into(),
        number: None,
        url: str_of(&n["url"]),
        description: n["description"].as_str().map(Into::into),
    })
}

fn project_candidate(p: &Value, owner: &str) -> Option<Candidate> {
    Some(Candidate {
        id: p["id"].as_str()?.into(),
        owner: owner.into(),
        name: str_of(&p["title"]),
        number: p["number"].as_i64(),
        url: str_of(&p["url"]),
        description: p["shortDescription"].as_str().filter(|s| !s.is_empty()).map(Into::into),
    })
}

/// Converts an Issue or PullRequest GraphQL node into an `Item`.
fn content_item(n: &Value, account_id: &str) -> Option<Item> {
    let kind = match n["__typename"].as_str() {
        Some("PullRequest") => ItemKind::PullRequest,
        Some("Issue") | None => ItemKind::Issue,
        Some(_) => return None,
    };
    let id = n["id"].as_str()?;
    Some(Item {
        id: format!("gh:{id}"),
        kind,
        container_id: str_of(&n["repository"]["id"]),
        container_name: str_of(&n["repository"]["nameWithOwner"]),
        account_id: Some(account_id.into()),
        title: str_of(&n["title"]),
        body: body_of(&n["body"]),
        url: n["url"].as_str().map(Into::into),
        number: n["number"].as_i64(),
        completed: matches!(n["state"].as_str(), Some("CLOSED" | "MERGED")),
        due: n["milestone"]["dueOn"].as_str().map(|d| d.chars().take(10).collect()),
        priority: 0,
        labels: n["labels"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| Label { name: str_of(&l["name"]), color: str_of(&l["color"]) })
            .collect(),
        assignees: logins(&n["assignees"]),
        status: None,
        project_ids: vec![],
        updated_at: n["updatedAt"].as_str().map(Into::into),
        parent_id: n["parent"]["id"].as_str().map(|id| format!("gh:{id}")),
        sub_issues: n["subIssuesSummary"]["total"]
            .as_i64()
            .filter(|&t| t > 0)
            .map(|total| SubIssues { total, completed: n["subIssuesSummary"]["completed"].as_i64().unwrap_or(0) }),
        categories: vec![],
    })
}

/// Adds a hint when GitHub rejects a request for lack of the `project` scope.
fn explain_scopes(msg: String) -> String {
    if msg.contains("read:project") || msg.contains("'project'") || msg.contains("required scopes") {
        format!(
            "{msg}. Projects need the `project` (or `read:project`) token scope — \
             if you use the GitHub CLI, run `gh auth refresh -s project` and re-import."
        )
    } else {
        msg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_issue_and_pr_nodes() {
        let issue = json!({
            "__typename": "Issue", "id": "I_1", "number": 7, "title": "Fix it", "body": "",
            "url": "https://github.com/o/r/issues/7", "state": "OPEN", "updatedAt": "2026-10-01T00:00:00Z",
            "repository": { "id": "R_1", "nameWithOwner": "o/r" },
            "labels": { "nodes": [{ "name": "school", "color": "ff0000" }] },
            "assignees": { "nodes": [{ "login": "me" }] },
            "milestone": { "dueOn": "2026-11-01T07:00:00Z" },
            "parent": { "id": "I_0" },
            "subIssuesSummary": { "total": 3, "completed": 1 }
        });
        let it = content_item(&issue, "me@github.com").unwrap();
        assert_eq!(it.id, "gh:I_1");
        assert_eq!(it.kind, ItemKind::Issue);
        assert_eq!(it.body, None);
        assert_eq!(it.due.as_deref(), Some("2026-11-01"));
        assert_eq!(it.labels[0].name, "school");
        assert_eq!(it.repo_and_number(), Some(("o", "r", 7)));
        assert!(!it.completed);
        assert_eq!(it.parent_id.as_deref(), Some("gh:I_0"));
        assert_eq!(it.sub_issues, Some(SubIssues { total: 3, completed: 1 }));

        let pr = json!({ "__typename": "PullRequest", "id": "PR_1", "state": "MERGED",
                         "repository": { "id": "R_1", "nameWithOwner": "o/r" } });
        let it = content_item(&pr, "a").unwrap();
        assert_eq!(it.kind, ItemKind::PullRequest);
        assert!(it.completed);

        assert!(content_item(&json!({ "__typename": "DraftIssue", "id": "x" }), "a").is_none());
    }

    #[test]
    fn picks_due_like_project_date_fields() {
        let values = json!([
            { "date": "2026-10-01", "field": { "name": "Start date" } },
            { "date": "2026-10-20", "field": { "name": "Target date" } },
            {},
            { "date": "2026-10-15", "field": { "name": "Due" } }
        ]);
        assert_eq!(project_due_date(&values).as_deref(), Some("2026-10-15"));
        let only_start = json!([{ "date": "2026-10-01", "field": { "name": "Start" } }]);
        assert_eq!(project_due_date(&only_start), None);
        assert_eq!(project_due_date(&json!([])), None);
    }

    #[test]
    fn truncates_long_bodies_on_char_boundaries() {
        let long = "é".repeat(MAX_BODY + 10);
        let b = body_of(&Value::String(long)).unwrap();
        assert_eq!(b.chars().count(), MAX_BODY + 1);
        assert!(b.ends_with('…'));
    }

    #[test]
    fn endpoints_for_enterprise_hosts() {
        assert_eq!(graphql_url("github.com"), "https://api.github.com/graphql");
        assert_eq!(rest_url("ghe.example.com"), "https://ghe.example.com/api/v3");
    }
}
