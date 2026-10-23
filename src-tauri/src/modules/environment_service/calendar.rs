//! Environment readers share the explicitly imported calendar, including offline use.
use chrono::NaiveDate;
use super::types::CalendarView;
pub fn read(date:NaiveDate)->CalendarView {
    crate::modules::care::global().map(|s|s.calendar.lock().unwrap().view(date)).unwrap_or_else(||CalendarView{local_date:date.to_string(),region:String::new(),dataset_version:"not-imported".into(),coverage:"not_imported".into(),holidays:vec![]})
}
