//! Decides which category tabs an item belongs to.
//!
//! Precedence: explicit tags (GitHub labels, `#hashtags` in reminders) win. If an
//! item has none, it inherits the default category of where it lives (Reminders
//! list, repo, or project). Items with neither land in the Inbox.

use std::collections::HashMap;

use crate::model::{Category, Item, ItemKind};

pub struct Rules {
    /// Lowercased label/name -> category id.
    tags: HashMap<String, String>,
    /// Container id (list, repo or project) -> default category id.
    defaults: HashMap<String, String>,
}

impl Rules {
    pub fn new<'a>(
        categories: &[Category],
        defaults: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
    ) -> Self {
        let mut tags = HashMap::new();
        for c in categories {
            tags.insert(c.name.to_lowercase(), c.id.clone());
            // The label wins if it collides with another category's name.
            tags.insert(c.label.to_lowercase(), c.id.clone());
        }
        let defaults = defaults
            .into_iter()
            .filter_map(|(id, cat)| Some((id.to_owned(), cat?.to_owned())))
            .collect();
        Self { tags, defaults }
    }

    pub fn categorize(&self, item: &Item) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        fn push(out: &mut Vec<String>, id: &String) {
            if !out.contains(id) {
                out.push(id.clone());
            }
        }

        match item.kind {
            ItemKind::Reminder => {
                let text = [Some(item.title.as_str()), item.body.as_deref()];
                for tag in text.into_iter().flatten().flat_map(hashtags) {
                    if let Some(id) = self.tags.get(&tag) {
                        push(&mut out, id);
                    }
                }
            }
            _ => {
                for label in &item.labels {
                    if let Some(id) = self.tags.get(&label.name.to_lowercase()) {
                        push(&mut out, id);
                    }
                }
            }
        }
        if !out.is_empty() {
            return out;
        }

        let containers = std::iter::once(&item.container_id).chain(item.project_ids.iter());
        for c in containers {
            if let Some(id) = self.defaults.get(c) {
                push(&mut out, id);
            }
        }
        out
    }
}

/// Lowercased `#tags` in free text. A tag is `#` followed by word characters,
/// `-` or `:`, and must start the text or follow whitespace (so URLs with
/// fragments don't count).
pub fn hashtags(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split_whitespace().filter_map(|word| {
        let tag = word.strip_prefix('#')?;
        let tag: String = tag
            .chars()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ':'))
            .collect();
        (!tag.is_empty()).then(|| tag.to_lowercase())
    })
}

pub fn default_categories() -> Vec<Category> {
    [
        ("personal", "Personal", "#4f8cff"),
        ("business", "Business", "#f2994a"),
        ("school", "School", "#9b6dff"),
        ("fun", "Fun", "#27ae60"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (id, name, color))| Category {
        id: id.into(),
        name: name.into(),
        color: color.into(),
        label: id.into(),
        position: i as i64,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Label;

    fn item(kind: ItemKind, container: &str) -> Item {
        Item {
            id: "x".into(),
            kind,
            container_id: container.into(),
            container_name: "c".into(),
            account_id: None,
            title: "Do the thing".into(),
            body: None,
            url: None,
            number: None,
            completed: false,
            due: None,
            priority: 0,
            labels: vec![],
            assignees: vec![],
            status: None,
            project_ids: vec![],
            updated_at: None,
            parent_id: None,
            sub_issues: None,
            categories: vec![],
        }
    }

    fn rules() -> Rules {
        let mut cats = default_categories();
        cats[1].label = "area:work".into();
        Rules::new(
            &cats,
            [
                ("list-school", Some("school")),
                ("repo-biz", Some("business")),
                ("proj-fun", Some("fun")),
                ("list-none", None),
            ],
        )
    }

    #[test]
    fn github_labels_match_label_or_name_case_insensitively() {
        let mut i = item(ItemKind::Issue, "repo-biz");
        i.labels = vec![
            Label { name: "School".into(), color: "".into() },
            Label { name: "AREA:WORK".into(), color: "".into() },
            Label { name: "bug".into(), color: "".into() },
        ];
        assert_eq!(rules().categorize(&i), vec!["school", "business"]);
    }

    #[test]
    fn labels_override_repo_default() {
        let mut i = item(ItemKind::Issue, "repo-biz");
        i.labels = vec![Label { name: "fun".into(), color: "".into() }];
        assert_eq!(rules().categorize(&i), vec!["fun"]);
    }

    #[test]
    fn falls_back_to_repo_then_project_defaults() {
        let mut i = item(ItemKind::Issue, "repo-biz");
        i.project_ids = vec!["proj-fun".into()];
        assert_eq!(rules().categorize(&i), vec!["business", "fun"]);

        let d = item(ItemKind::DraftIssue, "proj-fun");
        assert_eq!(rules().categorize(&d), vec!["fun"]);
    }

    #[test]
    fn reminder_hashtags_override_list_default() {
        let mut r = item(ItemKind::Reminder, "list-school");
        assert_eq!(rules().categorize(&r), vec!["school"]);
        r.title = "Buy tickets #Fun".into();
        r.body = Some("see https://x.com/#personal and #personal".into());
        assert_eq!(rules().categorize(&r), vec!["fun", "personal"]);
    }

    #[test]
    fn uncategorized_is_empty() {
        assert!(rules().categorize(&item(ItemKind::Reminder, "list-none")).is_empty());
        assert!(rules().categorize(&item(ItemKind::Issue, "unknown")).is_empty());
    }

    #[test]
    fn hashtag_parsing() {
        let tags: Vec<_> = hashtags("#a, b#c #area:work! # #x-y_z").collect();
        assert_eq!(tags, vec!["a", "area:work", "x-y_z"]);
    }
}
