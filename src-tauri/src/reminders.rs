//! Apple Reminders through EventKit, called directly from Rust (no AppleScript).
//!
//! `EKEventStore` and the objects it vends aren't `Send`, so one dedicated
//! thread owns the store and runs jobs sent to it. Each job gets a oneshot
//! sender for its result; async EventKit calls (access requests, fetches) carry
//! that sender into their completion block so the thread never blocks on them.

use tokio::sync::oneshot;

use crate::error::{Error, Result};
use crate::model::{Item, ReminderList};

pub struct NewReminder {
    pub list_id: String,
    pub title: String,
    pub notes: Option<String>,
    pub due: Option<String>,
    pub priority: i64,
}

/// Fields to change on an existing reminder; `None` leaves a field alone.
#[derive(Default)]
pub struct ReminderPatch {
    pub title: Option<String>,
    /// `Some(None)` clears the notes.
    pub notes: Option<Option<String>>,
    /// `Some(None)` removes the due date (and its alerts).
    pub due: Option<Option<String>>,
    pub priority: Option<i64>,
    pub list_id: Option<String>,
}

#[cfg(target_os = "macos")]
pub use mac::Reminders;

#[cfg(not(target_os = "macos"))]
pub use other::Reminders;

/// Parses `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM` into (date, optional time).
fn parse_due(s: &str) -> Result<(chrono::NaiveDate, Option<chrono::NaiveTime>)> {
    use chrono::{NaiveDate, NaiveDateTime};
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok((d, None));
    }
    for fmt in ["%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok((dt.date(), Some(dt.time())));
        }
    }
    Err(Error::Reminders(format!("couldn't understand due date \"{s}\"")))
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ptr::NonNull;
    use std::sync::Mutex;
    use std::sync::mpsc;

    use block2::RcBlock;
    use chrono::{Datelike, Timelike};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool, ProtocolObject};
    use objc2_core_graphics::CGColor;
    use objc2_event_kit::{
        EKAlarm, EKAuthorizationStatus, EKCalendar, EKEntityType, EKEventStore,
        EKEventStoreChangedNotification, EKReminder,
    };
    use objc2_foundation::{
        NSArray, NSCalendar, NSDate, NSDateComponentUndefined, NSDateComponents, NSError,
        NSNotification, NSNotificationCenter, NSObjectProtocol, NSString,
    };

    use super::*;
    use crate::model::ItemKind;

    type Job = Box<dyn FnOnce(&Store) + Send>;

    struct Store {
        ek: Retained<EKEventStore>,
        _observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    }

    pub struct Reminders {
        jobs: mpsc::Sender<Job>,
    }

    impl Reminders {
        /// Starts the EventKit thread. `on_change` fires whenever the Reminders
        /// database changes (including edits made in the Reminders app).
        pub fn start(on_change: impl Fn() + Send + Sync + 'static) -> Self {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("eventkit".into())
                .spawn(move || {
                    let ek = unsafe { EKEventStore::new() };
                    let block = RcBlock::new(move |_: NonNull<NSNotification>| on_change());
                    let observer = unsafe {
                        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                            Some(EKEventStoreChangedNotification),
                            Some(&*ek as &AnyObject),
                            None,
                            &block,
                        )
                    };
                    let store = Store { ek, _observer: observer };
                    while let Ok(job) = rx.recv() {
                        // A panic in one job shouldn't take down Reminders for the session.
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&store)));
                    }
                })
                .expect("failed to spawn EventKit thread");
            Self { jobs: tx }
        }

        async fn exec<T: Send + 'static>(
            &self,
            f: impl FnOnce(&Store, oneshot::Sender<Result<T>>) + Send + 'static,
        ) -> Result<T> {
            let (tx, rx) = oneshot::channel();
            self.jobs
                .send(Box::new(move |s| f(s, tx)))
                .map_err(|_| Error::Reminders("EventKit thread stopped".into()))?;
            rx.await.map_err(|_| Error::Reminders("EventKit request was dropped".into()))?
        }

        pub fn access_status(&self) -> &'static str {
            let st = unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Reminder) };
            match st {
                EKAuthorizationStatus::FullAccess => "granted",
                EKAuthorizationStatus::WriteOnly => "writeOnly",
                EKAuthorizationStatus::Denied => "denied",
                EKAuthorizationStatus::Restricted => "restricted",
                _ => "notDetermined",
            }
        }

        pub async fn request_access(&self) -> Result<bool> {
            self.exec(|s, reply| {
                let reply = Mutex::new(Some(reply));
                let ek = s.ek.clone();
                let block = RcBlock::new(move |granted: Bool, err: *mut NSError| {
                    let res = if granted.as_bool() {
                        // Calendars only appear in a store after access is granted.
                        unsafe { ek.reset() };
                        Ok(true)
                    } else if let Some(err) = unsafe { err.as_ref() } {
                        Err(Error::Reminders(err.localizedDescription().to_string()))
                    } else {
                        Ok(false)
                    };
                    if let Some(r) = reply.lock().unwrap().take() {
                        let _ = r.send(res);
                    }
                });
                unsafe { s.ek.requestFullAccessToRemindersWithCompletion(RcBlock::as_ptr(&block)) };
            })
            .await
        }

        pub async fn lists(&self) -> Result<Vec<ReminderList>> {
            self.exec(|s, reply| {
                let cals = unsafe { s.ek.calendarsForEntityType(EKEntityType::Reminder) };
                let lists = cals
                    .iter()
                    .map(|c| unsafe {
                        ReminderList {
                            id: c.calendarIdentifier().to_string(),
                            title: c.title().to_string(),
                            color: c.CGColor().and_then(|c| hex_color(&c)),
                            source_title: c.source().map(|s| s.title().to_string()),
                            writable: c.allowsContentModifications(),
                            enabled: true,
                            default_category: None,
                        }
                    })
                    .collect();
                let _ = reply.send(Ok(lists));
            })
            .await
        }

        /// All incomplete reminders in the given lists.
        pub async fn incomplete(&self, list_ids: Vec<String>) -> Result<Vec<Item>> {
            if list_ids.is_empty() {
                // An empty filter would mean "all lists" to EventKit.
                return Ok(vec![]);
            }
            self.exec(move |s, reply| {
                let cals: Vec<Retained<EKCalendar>> = list_ids
                    .iter()
                    .filter_map(|id| unsafe { s.ek.calendarWithIdentifier(&NSString::from_str(id)) })
                    .collect();
                if cals.is_empty() {
                    let _ = reply.send(Ok(vec![]));
                    return;
                }
                let cals = NSArray::from_retained_slice(&cals);
                let pred = unsafe {
                    s.ek.predicateForIncompleteRemindersWithDueDateStarting_ending_calendars(
                        None,
                        None,
                        Some(&cals),
                    )
                };
                let reply = Mutex::new(Some(reply));
                let block = RcBlock::new(move |arr: *mut NSArray<EKReminder>| {
                    let items = unsafe { arr.as_ref() }
                        .map(|a| a.iter().map(|r| to_item(&r)).collect())
                        .unwrap_or_default();
                    if let Some(r) = reply.lock().unwrap().take() {
                        let _ = r.send(Ok(items));
                    }
                });
                unsafe { s.ek.fetchRemindersMatchingPredicate_completion(&pred, &block) };
            })
            .await
        }

        pub async fn create(&self, new: NewReminder) -> Result<Item> {
            let due = new.due.as_deref().map(parse_due).transpose()?;
            self.exec(move |s, reply| {
                let res = (|| unsafe {
                    let cal = s
                        .ek
                        .calendarWithIdentifier(&NSString::from_str(&new.list_id))
                        .ok_or_else(|| Error::Reminders("that list no longer exists".into()))?;
                    let r = EKReminder::reminderWithEventStore(&s.ek);
                    r.setCalendar(Some(&cal));
                    r.setTitle(Some(&NSString::from_str(&new.title)));
                    if let Some(n) = new.notes.as_deref().filter(|n| !n.trim().is_empty()) {
                        r.setNotes(Some(&NSString::from_str(n)));
                    }
                    r.setPriority(new.priority.clamp(0, 9) as usize);
                    if due.is_some() {
                        set_due(&r, due);
                    }
                    s.ek.saveReminder_commit_error(&r, true).map_err(ns_err)?;
                    Ok(to_item(&r))
                })();
                let _ = reply.send(res);
            })
            .await
        }

        pub async fn update(&self, id: String, patch: ReminderPatch) -> Result<Item> {
            let due = match &patch.due {
                Some(Some(d)) => Some(Some(parse_due(d)?)),
                Some(None) => Some(None),
                None => None,
            };
            self.exec(move |s, reply| {
                let res = find(s, &id).and_then(|r| unsafe {
                    if let Some(list_id) = &patch.list_id {
                        let cal = s
                            .ek
                            .calendarWithIdentifier(&NSString::from_str(list_id))
                            .ok_or_else(|| Error::Reminders("that list no longer exists".into()))?;
                        r.setCalendar(Some(&cal));
                    }
                    if let Some(t) = &patch.title {
                        r.setTitle(Some(&NSString::from_str(t)));
                    }
                    if let Some(n) = &patch.notes {
                        r.setNotes(n.as_deref().filter(|n| !n.trim().is_empty()).map(NSString::from_str).as_deref());
                    }
                    if let Some(p) = patch.priority {
                        r.setPriority(p.clamp(0, 9) as usize);
                    }
                    if let Some(d) = due {
                        // The old alerts belong to the old due date.
                        r.setAlarms(None);
                        set_due(&r, d);
                    }
                    s.ek.saveReminder_commit_error(&r, true).map_err(ns_err)?;
                    Ok(to_item(&r))
                });
                let _ = reply.send(res);
            })
            .await
        }

        pub async fn set_completed(&self, id: String, completed: bool) -> Result<()> {
            self.modify(id, move |r| unsafe { r.setCompleted(completed) }).await
        }

        /// Deletes a reminder. Only used to undo creating one.
        pub async fn remove(&self, id: String) -> Result<()> {
            self.exec(move |s, reply| {
                let res = find(s, &id)
                    .and_then(|r| unsafe { s.ek.removeReminder_commit_error(&r, true) }.map_err(ns_err));
                let _ = reply.send(res);
            })
            .await
        }

        pub async fn move_to_list(&self, id: String, list_id: String) -> Result<()> {
            self.exec(move |s, reply| {
                let res = unsafe {
                    match s.ek.calendarWithIdentifier(&NSString::from_str(&list_id)) {
                        Some(cal) => find(s, &id).and_then(|r| {
                            r.setCalendar(Some(&cal));
                            s.ek.saveReminder_commit_error(&r, true).map_err(ns_err)
                        }),
                        None => Err(Error::Reminders("that list no longer exists".into())),
                    }
                };
                let _ = reply.send(res);
            })
            .await
        }

        pub async fn set_notes(&self, id: String, notes: Option<String>) -> Result<()> {
            self.modify(id, move |r| unsafe {
                r.setNotes(notes.as_deref().map(NSString::from_str).as_deref())
            })
            .await
        }

        async fn modify(&self, id: String, f: impl FnOnce(&EKReminder) + Send + 'static) -> Result<()> {
            self.exec(move |s, reply| {
                let res = find(s, &id).and_then(|r| {
                    f(&r);
                    unsafe { s.ek.saveReminder_commit_error(&r, true) }.map_err(ns_err)
                });
                let _ = reply.send(res);
            })
            .await
        }
    }

    /// Sets (or clears) the due date. Timed reminders only notify if they carry
    /// an alarm, so one is added at the due time.
    unsafe fn set_due(r: &EKReminder, due: Option<(chrono::NaiveDate, Option<chrono::NaiveTime>)>) {
        let Some((date, time)) = due else {
            unsafe { r.setDueDateComponents(None) };
            return;
        };
        let comps = NSDateComponents::new();
        comps.setYear(date.year() as isize);
        comps.setMonth(date.month() as isize);
        comps.setDay(date.day() as isize);
        if let Some(t) = time {
            comps.setHour(t.hour() as isize);
            comps.setMinute(t.minute() as isize);
            if let Some(at) = NSCalendar::currentCalendar().dateFromComponents(&comps) {
                unsafe { r.addAlarm(&EKAlarm::alarmWithAbsoluteDate(&at)) };
            }
        }
        unsafe { r.setDueDateComponents(Some(&comps)) };
    }

    fn find(s: &Store, id: &str) -> Result<Retained<EKReminder>> {
        let item = unsafe { s.ek.calendarItemWithIdentifier(&NSString::from_str(id)) }
            .ok_or_else(|| Error::Reminders("reminder not found — it may have been deleted".into()))?;
        item.downcast::<EKReminder>().map_err(|_| Error::Reminders("not a reminder".into()))
    }

    fn ns_err(e: Retained<NSError>) -> Error {
        Error::Reminders(e.localizedDescription().to_string())
    }

    fn to_item(r: &EKReminder) -> Item {
        unsafe {
            let (container_id, container_name) = r
                .calendar()
                .map(|c| (c.calendarIdentifier().to_string(), c.title().to_string()))
                .unwrap_or_default();
            Item {
                id: format!("rem:{}", r.calendarItemIdentifier()),
                kind: ItemKind::Reminder,
                container_id,
                container_name,
                account_id: None,
                title: r.title().to_string(),
                body: r.notes().map(|n| n.to_string()).filter(|n| !n.trim().is_empty()),
                url: None,
                number: None,
                completed: r.isCompleted(),
                due: r.dueDateComponents().and_then(|c| format_components(&c)),
                priority: r.priority() as i64,
                labels: vec![],
                assignees: vec![],
                status: None,
                project_ids: vec![],
                updated_at: r.lastModifiedDate().and_then(|d| rfc3339(&d)),
                parent_id: None,
                sub_issues: None,
                categories: vec![],
            }
        }
    }

    fn format_components(c: &NSDateComponents) -> Option<String> {
        let (y, m, d) = (c.year(), c.month(), c.day());
        if [y, m, d].contains(&NSDateComponentUndefined) {
            return None;
        }
        let date = format!("{y:04}-{m:02}-{d:02}");
        let (h, min) = (c.hour(), c.minute());
        if h == NSDateComponentUndefined {
            return Some(date);
        }
        let min = if min == NSDateComponentUndefined { 0 } else { min };
        Some(format!("{date}T{h:02}:{min:02}"))
    }

    fn rfc3339(d: &NSDate) -> Option<String> {
        let secs = d.timeIntervalSince1970();
        chrono::DateTime::from_timestamp(secs as i64, 0).map(|t| t.to_rfc3339())
    }

    fn hex_color(c: &CGColor) -> Option<String> {
        let n = CGColor::number_of_components(Some(c));
        let ptr = CGColor::components(Some(c));
        if ptr.is_null() || n < 3 {
            return None;
        }
        let comps = unsafe { std::slice::from_raw_parts(ptr, n) };
        let to = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some(format!("#{:02x}{:02x}{:02x}", to(comps[0]), to(comps[1]), to(comps[2])))
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use super::*;

    pub struct Reminders;

    fn unsupported<T>() -> Result<T> {
        Err(Error::Reminders("Apple Reminders is only available on macOS".into()))
    }

    impl Reminders {
        pub fn start(_: impl Fn() + Send + Sync + 'static) -> Self {
            Self
        }
        pub fn access_status(&self) -> &'static str {
            "unsupported"
        }
        pub async fn request_access(&self) -> Result<bool> {
            unsupported()
        }
        pub async fn lists(&self) -> Result<Vec<ReminderList>> {
            unsupported()
        }
        pub async fn incomplete(&self, _: Vec<String>) -> Result<Vec<Item>> {
            unsupported()
        }
        pub async fn create(&self, _: NewReminder) -> Result<Item> {
            unsupported()
        }
        pub async fn set_completed(&self, _: String, _: bool) -> Result<()> {
            unsupported()
        }
        pub async fn update(&self, _: String, _: ReminderPatch) -> Result<Item> {
            unsupported()
        }
        pub async fn remove(&self, _: String) -> Result<()> {
            unsupported()
        }
        pub async fn move_to_list(&self, _: String, _: String) -> Result<()> {
            unsupported()
        }
        pub async fn set_notes(&self, _: String, _: Option<String>) -> Result<()> {
            unsupported()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_due;

    #[test]
    fn parses_due_dates() {
        let (d, t) = parse_due("2026-10-12").unwrap();
        assert_eq!(d.to_string(), "2026-10-12");
        assert!(t.is_none());
        let (_, t) = parse_due("2026-10-12T09:30").unwrap();
        assert_eq!(t.unwrap().to_string(), "09:30:00");
        assert!(parse_due("tomorrow").is_err());
    }
}
