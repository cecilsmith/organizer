//! Apple Reminders and calendar events through EventKit, called directly
//! from Rust (no AppleScript). Calendars include every account added to macOS
//! (iCloud, Google, Exchange/Microsoft 365, CalDAV).
//!
//! `EKEventStore` and the objects it vends aren't `Send`, so one dedicated
//! thread owns the store and runs jobs sent to it. Each job gets a oneshot
//! sender for its result; async EventKit calls (access requests, fetches) carry
//! that sender into their completion block so the thread never blocks on them.

use tokio::sync::oneshot;

use crate::error::{Error, Result};
use crate::model::{CalendarEvent, EventPatch, Item, ReminderList};

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

/// Parses an event time: `YYYY-MM-DDTHH:MM` or a bare date (midnight).
fn parse_event_time(s: &str) -> Result<chrono::NaiveDateTime> {
    let (date, time) = parse_due(s)?;
    Ok(date.and_time(time.unwrap_or_default()))
}

/// Start and end instants for an event. All-day events run from midnight of
/// the first day to the last second of the last day, as EventKit stores them.
fn event_bounds(start: &str, end: &str, all_day: bool) -> Result<(chrono::NaiveDateTime, chrono::NaiveDateTime)> {
    let s = parse_event_time(start)?;
    let e = parse_event_time(end)?;
    if all_day {
        let first = s.date();
        let last = e.date().max(first);
        return Ok((first.and_hms_opt(0, 0, 0).unwrap(), last.and_hms_opt(23, 59, 59).unwrap()));
    }
    if e <= s {
        return Err(Error::Reminders("the event has to end after it starts".into()));
    }
    Ok((s, e))
}

/// Formats EventKit's start/end for the UI (see `CalendarEvent`).
fn format_event_times(start: chrono::NaiveDateTime, end: chrono::NaiveDateTime, all_day: bool) -> (String, String) {
    if all_day {
        // EventKit may end an all-day event at 23:59:59 or at the next midnight.
        let last = if end.time() == chrono::NaiveTime::MIN && end > start {
            (end - chrono::Duration::seconds(1)).date()
        } else {
            end.date()
        };
        return (start.date().to_string(), last.max(start.date()).to_string());
    }
    let f = |d: chrono::NaiveDateTime| d.format("%Y-%m-%dT%H:%M").to_string();
    (f(start), f(end))
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
        EKAlarm, EKAuthorizationStatus, EKCalendar, EKEntityType, EKEvent, EKEventStore,
        EKEventStoreChangedNotification, EKReminder, EKSpan,
    };
    use objc2_foundation::{
        NSArray, NSCalendar, NSDate, NSDateComponentUndefined, NSDateComponents, NSError,
        NSNotification, NSNotificationCenter, NSObjectProtocol, NSString,
    };

    use super::*;
    use crate::model::{ItemKind, Provider};

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
            status_for(EKEntityType::Reminder)
        }

        pub fn calendar_access_status(&self) -> &'static str {
            status_for(EKEntityType::Event)
        }

        pub async fn request_access(&self) -> Result<bool> {
            self.request(EKEntityType::Reminder).await
        }

        pub async fn request_calendar_access(&self) -> Result<bool> {
            self.request(EKEntityType::Event).await
        }

        async fn request(&self, kind: EKEntityType) -> Result<bool> {
            self.exec(move |s, reply| {
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
                unsafe {
                    if kind == EKEntityType::Event {
                        s.ek.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&block))
                    } else {
                        s.ek.requestFullAccessToRemindersWithCompletion(RcBlock::as_ptr(&block))
                    }
                };
            })
            .await
        }

        pub async fn lists(&self) -> Result<Vec<ReminderList>> {
            self.calendars(EKEntityType::Reminder).await
        }

        /// Event calendars from every account macOS knows about.
        pub async fn event_calendars(&self) -> Result<Vec<ReminderList>> {
            self.calendars(EKEntityType::Event).await
        }

        async fn calendars(&self, kind: EKEntityType) -> Result<Vec<ReminderList>> {
            self.exec(move |s, reply| {
                let cals = unsafe { s.ek.calendarsForEntityType(kind) };
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

        // ---- events ---------------------------------------------------------

        /// Every occurrence in the given calendars between two dates.
        pub async fn events(
            &self,
            calendar_ids: Vec<String>,
            from: chrono::NaiveDate,
            to: chrono::NaiveDate,
        ) -> Result<Vec<CalendarEvent>> {
            if calendar_ids.is_empty() {
                return Ok(vec![]);
            }
            self.exec(move |s, reply| {
                let cals: Vec<Retained<EKCalendar>> = calendar_ids
                    .iter()
                    .filter_map(|id| unsafe { s.ek.calendarWithIdentifier(&NSString::from_str(id)) })
                    .collect();
                if cals.is_empty() {
                    let _ = reply.send(Ok(vec![]));
                    return;
                }
                let cals = NSArray::from_retained_slice(&cals);
                let start = ns_date(from.and_hms_opt(0, 0, 0).unwrap());
                let end = ns_date(to.and_hms_opt(23, 59, 59).unwrap());
                let events = unsafe {
                    let pred = s.ek.predicateForEventsWithStartDate_endDate_calendars(&start, &end, Some(&cals));
                    s.ek.eventsMatchingPredicate(&pred)
                };
                let _ = reply.send(Ok(events.iter().filter_map(|e| to_event(&e)).collect()));
            })
            .await
        }

        #[allow(clippy::too_many_arguments)]
        pub async fn create_event(
            &self,
            calendar_id: String,
            title: String,
            start: String,
            end: String,
            all_day: bool,
            location: Option<String>,
            notes: Option<String>,
        ) -> Result<CalendarEvent> {
            let (start, end) = event_bounds(&start, &end, all_day)?;
            self.exec(move |s, reply| {
                let res = (|| unsafe {
                    let cal = s
                        .ek
                        .calendarWithIdentifier(&NSString::from_str(&calendar_id))
                        .ok_or_else(|| Error::Reminders("that calendar no longer exists".into()))?;
                    let e = EKEvent::eventWithEventStore(&s.ek);
                    e.setCalendar(Some(&cal));
                    e.setTitle(Some(&NSString::from_str(&title)));
                    e.setAllDay(all_day);
                    e.setStartDate(Some(&ns_date(start)));
                    e.setEndDate(Some(&ns_date(end)));
                    if let Some(l) = location.as_deref().filter(|l| !l.trim().is_empty()) {
                        e.setLocation(Some(&NSString::from_str(l)));
                    }
                    if let Some(n) = notes.as_deref().filter(|n| !n.trim().is_empty()) {
                        e.setNotes(Some(&NSString::from_str(n)));
                    }
                    s.ek.saveEvent_span_commit_error(&e, EKSpan::ThisEvent, true).map_err(ns_err)?;
                    to_event(&e).ok_or_else(|| Error::Reminders("the event was saved but couldn't be read back".into()))
                })();
                let _ = reply.send(res);
            })
            .await
        }

        /// Edits one occurrence (for repeating events, only that occurrence).
        pub async fn update_event(&self, event_id: String, occurrence: i64, patch: EventPatch) -> Result<CalendarEvent> {
            self.exec(move |s, reply| {
                let res = find_event(s, &event_id, occurrence).and_then(|e| unsafe {
                    if let Some(id) = &patch.calendar_id {
                        let cal = s
                            .ek
                            .calendarWithIdentifier(&NSString::from_str(id))
                            .ok_or_else(|| Error::Reminders("that calendar no longer exists".into()))?;
                        e.setCalendar(Some(&cal));
                    }
                    if let Some(t) = &patch.title {
                        e.setTitle(Some(&NSString::from_str(t)));
                    }
                    if let Some(n) = &patch.notes {
                        e.setNotes(Some(n.as_str()).filter(|n| !n.trim().is_empty()).map(NSString::from_str).as_deref());
                    }
                    if let Some(l) = &patch.location {
                        e.setLocation(Some(l.as_str()).filter(|l| !l.trim().is_empty()).map(NSString::from_str).as_deref());
                    }
                    if patch.start.is_some() || patch.end.is_some() || patch.all_day.is_some() {
                        let current = to_event(&e).ok_or_else(|| Error::Reminders("couldn't read the event".into()))?;
                        let all_day = patch.all_day.unwrap_or(current.all_day);
                        let start = patch.start.clone().unwrap_or(current.start);
                        let end = patch.end.clone().unwrap_or(current.end);
                        let (start, end) = event_bounds(&start, &end, all_day)?;
                        e.setAllDay(all_day);
                        e.setStartDate(Some(&ns_date(start)));
                        e.setEndDate(Some(&ns_date(end)));
                    }
                    s.ek.saveEvent_span_commit_error(&e, EKSpan::ThisEvent, true).map_err(ns_err)?;
                    to_event(&e).ok_or_else(|| Error::Reminders("couldn't read the event".into()))
                });
                let _ = reply.send(res);
            })
            .await
        }

        /// Deletes one occurrence. Only used to undo creating an event.
        pub async fn remove_event(&self, event_id: String, occurrence: i64) -> Result<()> {
            self.exec(move |s, reply| {
                let res = find_event(s, &event_id, occurrence).and_then(|e| {
                    unsafe { s.ek.removeEvent_span_commit_error(&e, EKSpan::ThisEvent, true) }.map_err(ns_err)
                });
                let _ = reply.send(res);
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

    fn status_for(kind: EKEntityType) -> &'static str {
        match unsafe { EKEventStore::authorizationStatusForEntityType(kind) } {
            EKAuthorizationStatus::FullAccess => "granted",
            EKAuthorizationStatus::WriteOnly => "writeOnly",
            EKAuthorizationStatus::Denied => "denied",
            EKAuthorizationStatus::Restricted => "restricted",
            _ => "notDetermined",
        }
    }

    fn ns_date(dt: chrono::NaiveDateTime) -> Retained<NSDate> {
        use chrono::TimeZone;
        // `earliest` resolves times that occur twice when clocks fall back.
        let local = chrono::Local
            .from_local_datetime(&dt)
            .earliest()
            .unwrap_or_else(|| chrono::Local.from_utc_datetime(&dt));
        NSDate::dateWithTimeIntervalSince1970(local.timestamp() as f64)
    }

    fn local(d: &NSDate) -> chrono::NaiveDateTime {
        let secs = d.timeIntervalSince1970();
        chrono::DateTime::from_timestamp(secs as i64, 0)
            .unwrap_or_default()
            .with_timezone(&chrono::Local)
            .naive_local()
    }

    fn to_event(e: &EKEvent) -> Option<CalendarEvent> {
        unsafe {
            let event_id = e.eventIdentifier()?.to_string();
            let start = e.startDate();
            let occurrence = e.occurrenceDate().unwrap_or_else(|| start.clone()).timeIntervalSince1970() as i64;
            let all_day = e.isAllDay();
            let (start, end) = format_event_times(local(&start), local(&e.endDate()), all_day);
            let cal = e.calendar();
            let recurring = e.hasRecurrenceRules();
            Some(CalendarEvent {
                // One-off events keep the same id when moved; occurrences of a
                // repeating event are told apart by their original start.
                id: if recurring { format!("ev:{event_id}@{occurrence}") } else { format!("ev:{event_id}") },
                event_id,
                occurrence,
                calendar_id: cal.as_ref().map(|c| c.calendarIdentifier().to_string()).unwrap_or_default(),
                calendar_name: cal.as_ref().map(|c| c.title().to_string()).unwrap_or_default(),
                color: cal.as_ref().and_then(|c| c.CGColor()).and_then(|c| hex_color(&c)),
                title: e.title().to_string(),
                notes: e.notes().map(|n| n.to_string()).filter(|n| !n.trim().is_empty()),
                location: e.location().map(|l| l.to_string()).filter(|l| !l.trim().is_empty()),
                url: e.URL().and_then(|u| u.absoluteString()).map(|u| u.to_string()),
                start,
                end,
                all_day,
                recurring,
                writable: cal.as_ref().is_some_and(|c| c.allowsContentModifications()),
                categories: vec![],
            })
        }
    }

    /// Finds one occurrence of an event. `eventWithIdentifier` only returns the
    /// first occurrence of a repeating event, so search around the date.
    fn find_event(s: &Store, event_id: &str, occurrence: i64) -> Result<Retained<EKEvent>> {
        let not_found = || Error::Reminders("event not found — it may have been deleted".into());
        let first = unsafe { s.ek.eventWithIdentifier(&NSString::from_str(event_id)) }.ok_or_else(not_found)?;
        if !unsafe { first.hasRecurrenceRules() } {
            return Ok(first);
        }
        let at = |secs: i64| NSDate::dateWithTimeIntervalSince1970(secs as f64);
        let found = unsafe {
            let cals = first.calendar().map(|c| NSArray::from_retained_slice(&[c]));
            let pred = s.ek.predicateForEventsWithStartDate_endDate_calendars(
                &at(occurrence - 86_400),
                &at(occurrence + 2 * 86_400),
                cals.as_deref(),
            );
            s.ek.eventsMatchingPredicate(&pred).iter().find(|e| {
                e.eventIdentifier().is_some_and(|id| id.to_string() == event_id)
                    && e.occurrenceDate().is_some_and(|d| (d.timeIntervalSince1970() as i64 - occurrence).abs() <= 1)
            })
        };
        found.ok_or_else(not_found)
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
                provider: Provider::Github,
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
                start: None,
                project_dates: None,
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
        pub fn calendar_access_status(&self) -> &'static str {
            "unsupported"
        }
        pub async fn request_calendar_access(&self) -> Result<bool> {
            unsupported()
        }
        pub async fn lists(&self) -> Result<Vec<ReminderList>> {
            unsupported()
        }
        pub async fn event_calendars(&self) -> Result<Vec<ReminderList>> {
            unsupported()
        }
        pub async fn events(&self, _: Vec<String>, _: chrono::NaiveDate, _: chrono::NaiveDate) -> Result<Vec<CalendarEvent>> {
            unsupported()
        }
        #[allow(clippy::too_many_arguments)]
        pub async fn create_event(
            &self,
            _: String,
            _: String,
            _: String,
            _: String,
            _: bool,
            _: Option<String>,
            _: Option<String>,
        ) -> Result<CalendarEvent> {
            unsupported()
        }
        pub async fn update_event(&self, _: String, _: i64, _: EventPatch) -> Result<CalendarEvent> {
            unsupported()
        }
        pub async fn remove_event(&self, _: String, _: i64) -> Result<()> {
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
    use super::{event_bounds, format_event_times, parse_due, parse_event_time};

    #[test]
    fn parses_due_dates() {
        let (d, t) = parse_due("2026-10-12").unwrap();
        assert_eq!(d.to_string(), "2026-10-12");
        assert!(t.is_none());
        let (_, t) = parse_due("2026-10-12T09:30").unwrap();
        assert_eq!(t.unwrap().to_string(), "09:30:00");
        assert!(parse_due("tomorrow").is_err());
    }

    #[test]
    fn all_day_events_span_whole_days() {
        let (s, e) = event_bounds("2026-10-12", "2026-10-13", true).unwrap();
        assert_eq!(s.to_string(), "2026-10-12 00:00:00");
        assert_eq!(e.to_string(), "2026-10-13 23:59:59");
        // Formatting back gives the inclusive last day, whichever way EventKit ends it.
        assert_eq!(format_event_times(s, e, true), ("2026-10-12".into(), "2026-10-13".into()));
        let next_midnight = parse_event_time("2026-10-14").unwrap();
        assert_eq!(format_event_times(s, next_midnight, true).1, "2026-10-13");
    }

    #[test]
    fn timed_events_must_end_after_they_start() {
        assert!(event_bounds("2026-10-12T10:00", "2026-10-12T09:00", false).is_err());
        let (s, e) = event_bounds("2026-10-12T10:00", "2026-10-12T11:30", false).unwrap();
        assert_eq!(format_event_times(s, e, false), ("2026-10-12T10:00".into(), "2026-10-12T11:30".into()));
    }
}
