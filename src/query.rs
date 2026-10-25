//! Search request model and KeyQL generator for Microsoft Purview eDiscovery.
//!
//! Staff fill in a `SearchRequest` through the wizard. `SearchRequest::build`
//! turns it into one KeyQL string, which is saved in a `.req` file for the
//! IT Director, plus plain-language notices. `describe` and `email` give
//! the plain-English versions shown on the review screen and sent with the
//! file. Everything here is pure and unit tested.
//!
//! KeyQL conventions followed (Microsoft Learn, "Keyword queries and search
//! conditions for eDiscovery"):
//! - Boolean operators are upper case and always written out.
//! - Values with spaces or special characters are double quoted.
//! - Dates are absolute `YYYY-MM-DD`; `a..b` covers both whole days.
//! - Only suffix wildcards (`budg*`) are supported.
//! - KeyQL has no comments, so request details never go into the query.

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Option tables
// ---------------------------------------------------------------------------

/// An item type the form offers.
pub struct ItemType {
    /// `kind:` values it maps to.
    pub kinds: &'static [&'static str],
    pub label: &'static str,
    pub description: &'static str,
    /// Shown up front; the rest sit under "More types".
    pub common: bool,
}

pub const ITEM_TYPES: &[ItemType] = &[
    ItemType {
        kinds: &["email"],
        label: "Email",
        description: "Messages in Outlook mailboxes",
        common: true,
    },
    ItemType {
        kinds: &["microsoftteams", "im"],
        label: "Teams chats and meetings",
        description: "Teams chats, channel posts, meetings and calls",
        common: true,
    },
    ItemType {
        kinds: &["docs"],
        label: "Files in OneDrive and SharePoint",
        description: "Word, Excel, PDF and other documents",
        common: true,
    },
    ItemType {
        kinds: &["meetings"],
        label: "Calendar items",
        description: "Outlook meetings and appointments",
        common: false,
    },
    ItemType {
        kinds: &["voicemail"],
        label: "Voicemail",
        description: "Voicemail saved in mailboxes",
        common: false,
    },
    ItemType {
        kinds: &["contacts"],
        label: "Contacts",
        description: "Outlook contacts",
        common: false,
    },
    ItemType {
        kinds: &["tasks"],
        label: "Tasks",
        description: "Outlook tasks",
        common: false,
    },
    ItemType {
        kinds: &["notes"],
        label: "Notes",
        description: "Outlook notes",
        common: false,
    },
    ItemType {
        kinds: &["faxes"],
        label: "Faxes",
        description: "Faxes saved in mailboxes",
        common: false,
    },
];

/// File types offered as check boxes: (extensions, label).
pub const FILE_TYPES: &[(&[&str], &str)] = &[
    (&["docx", "doc"], "Word"),
    (&["xlsx", "xls", "csv"], "Excel"),
    (&["pptx", "ppt"], "PowerPoint"),
    (&["pdf"], "PDF"),
    (&["msg", "eml"], "Saved emails"),
    (&["txt"], "Text files"),
    (&["jpg", "jpeg", "png", "gif"], "Pictures"),
    (&["mp4", "mov", "mp3", "wav"], "Audio and video"),
    (&["zip"], "Zip files"),
];

/// Built-in sensitive information types: (Purview name, friendly label).
pub const SENSITIVE_TYPES: &[(&str, &str)] = &[
    (
        "U.S. Social Security Number (SSN)",
        "Social Security numbers",
    ),
    ("Credit Card Number", "Credit card numbers"),
    ("U.S. Bank Account Number", "Bank account numbers"),
    ("ABA Routing Number", "Bank routing numbers"),
    ("U.S. Driver's License Number", "Driver's license numbers"),
    ("U.S. / U.K. Passport Number", "Passport numbers"),
    (
        "U.S. Individual Taxpayer Identification Number (ITIN)",
        "Taxpayer ID numbers (ITIN)",
    ),
];

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// What kind of request this is, in the words a Responsible Authority uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum RequestKind {
    #[default]
    NotSaid,
    PublicData,
    AboutThemselves,
    LegalHold,
    Personnel,
    Other,
}

impl RequestKind {
    pub const ALL: [RequestKind; 5] = [
        Self::PublicData,
        Self::AboutThemselves,
        Self::LegalHold,
        Self::Personnel,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::NotSaid => "Not said",
            Self::PublicData => "Data request from the public",
            Self::AboutThemselves => "Someone asking for data about themselves",
            Self::LegalHold => "Lawsuit or legal hold",
            Self::Personnel => "Personnel or internal investigation",
            Self::Other => "Something else",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::NotSaid => "",
            Self::PublicData => "A member of the public or the media wants to see government data.",
            Self::AboutThemselves => "A person wants to see the data kept about them.",
            Self::LegalHold => "An attorney needs records kept or collected for a case.",
            Self::Personnel => "HR or a manager needs records about an employee matter.",
            Self::Other => "Any other reason to search email, chats or files.",
        }
    }

    /// Used for the file name and email subject when the request has no name.
    fn short(self) -> &'static str {
        match self {
            Self::NotSaid | Self::Other => "Search request",
            Self::PublicData => "Data request",
            Self::AboutThemselves => "Data subject request",
            Self::LegalHold => "Legal hold",
            Self::Personnel => "Personnel matter",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct About {
    pub kind: RequestKind,
    /// Short name, also used for the file name. Optional: the kind and date stand in.
    pub name: String,
    /// What was asked for, pasted or in the user's words.
    pub reason: String,
    /// The Responsible Authority's own tracking number.
    pub reference: String,
    pub received: Option<NaiveDate>,
    pub needed_by: Option<NaiveDate>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Custodians {
    /// People whose mailbox / OneDrive the IT Director should search.
    pub people: Vec<String>,
    /// Shared mailboxes, Teams or SharePoint sites to search.
    pub locations: Vec<String>,
    /// Search everyone in the organisation instead.
    pub everyone: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum KeywordScope {
    #[default]
    Anywhere,
    SubjectTitle,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Keywords {
    /// Words or phrases. An entry with spaces is searched as an exact phrase.
    pub words: Vec<String>,
    /// Every entry must appear (otherwise any one is enough).
    pub match_all: bool,
    pub scope: KeywordScope,
    /// Leave out items containing any of these.
    pub none: Vec<String>,
    /// Pairs of words that must appear close together.
    pub near: Vec<Near>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Near {
    pub first: String,
    pub second: String,
    pub distance: u32,
}

impl Default for Near {
    fn default() -> Self {
        Self {
            first: String::new(),
            second: String::new(),
            distance: 10,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum DateField {
    #[default]
    Any,
    Sent,
    Received,
    Created,
    Modified,
}

impl DateField {
    pub const ALL: [Self; 5] = [
        Self::Any,
        Self::Sent,
        Self::Received,
        Self::Created,
        Self::Modified,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any date (sent, received or last changed)",
            Self::Sent => "The date an email was sent",
            Self::Received => "The date an email was received",
            Self::Created => "The date a file was created",
            Self::Modified => "The date a file was last changed",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Self::Any => "Dated",
            Self::Sent => "Sent",
            Self::Received => "Received",
            Self::Created => "Created",
            Self::Modified => "Last changed",
        }
    }

    fn properties(self) -> &'static [&'static str] {
        match self {
            Self::Any => &["sent", "received", "LastModifiedTime"],
            Self::Sent => &["sent"],
            Self::Received => &["received"],
            Self::Created => &["Created"],
            Self::Modified => &["LastModifiedTime"],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum DateMode {
    #[default]
    AnyTime,
    Last30Days,
    Last12Months,
    ThisYear,
    LastYear,
    Between,
    OnOrAfter,
    OnOrBefore,
}

impl DateMode {
    pub const ALL: [Self; 8] = [
        Self::AnyTime,
        Self::Last30Days,
        Self::Last12Months,
        Self::ThisYear,
        Self::LastYear,
        Self::Between,
        Self::OnOrAfter,
        Self::OnOrBefore,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::AnyTime => "Any time",
            Self::Last30Days => "Last 30 days",
            Self::Last12Months => "Last 12 months",
            Self::ThisYear => "This year",
            Self::LastYear => "Last year",
            Self::Between => "Between two dates",
            Self::OnOrAfter => "On or after a date",
            Self::OnOrBefore => "On or before a date",
        }
    }

    /// The fixed range a choice stands for, as of `today`. `None` = no limit
    /// on that side.
    pub fn range(
        self,
        start: NaiveDate,
        end: NaiveDate,
        today: NaiveDate,
    ) -> Option<(Option<NaiveDate>, Option<NaiveDate>)> {
        let ymd = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap_or(today);
        Some(match self {
            Self::AnyTime => return None,
            Self::Last30Days => (Some(today - Duration::days(30)), Some(today)),
            Self::Last12Months => (
                Some(
                    today
                        .with_year(today.year() - 1)
                        .unwrap_or(today - Duration::days(365)),
                ),
                Some(today),
            ),
            Self::ThisYear => (Some(ymd(today.year(), 1, 1)), Some(today)),
            Self::LastYear => (
                Some(ymd(today.year() - 1, 1, 1)),
                Some(ymd(today.year() - 1, 12, 31)),
            ),
            Self::Between => {
                let (a, b) = if start <= end {
                    (start, end)
                } else {
                    (end, start)
                };
                (Some(a), Some(b))
            }
            Self::OnOrAfter => (Some(start), None),
            Self::OnOrBefore => (None, Some(end)),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Dates {
    pub mode: DateMode,
    pub field: DateField,
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl Dates {
    pub fn new(today: NaiveDate) -> Self {
        Self {
            mode: DateMode::AnyTime,
            field: DateField::Any,
            start: NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap_or(today),
            end: today,
        }
    }
}

impl Default for Dates {
    fn default() -> Self {
        Self::new(chrono::Local::now().date_naive())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Attachments {
    #[default]
    Any,
    With,
    Without,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum SizeOp {
    #[default]
    Any,
    LargerThan,
    SmallerThan,
}

/// Optional filters ("Narrow it down").
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    pub sent_by: Vec<String>,
    pub sent_to: Vec<String>,
    pub leave_out_people: Vec<String>,
    pub attachments: Attachments,
    pub attachment_names: Vec<String>,
    pub high_importance: bool,
    /// Labels from `FILE_TYPES`.
    pub file_types: Vec<String>,
    pub other_extensions: Vec<String>,
    pub file_names: Vec<String>,
    pub authors: Vec<String>,
    pub size_op: SizeOp,
    pub size_mb: f64,
    /// Purview names from `SENSITIVE_TYPES`.
    pub sensitive: Vec<String>,
}

/// Everything a staff member fills in for one search request.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchRequest {
    pub about: About,
    pub custodians: Custodians,
    pub keywords: Keywords,
    pub dates: Dates,
    /// Labels from `ITEM_TYPES`. Empty means every type.
    pub item_types: Vec<String>,
    pub filters: Filters,
    /// The office's optional holidays, copied in from the app's settings (not part of the request).
    #[serde(skip)]
    pub holidays: OptionalHolidays,
}

// ---------------------------------------------------------------------------
// Minnesota deadline for a data subject's request
// ---------------------------------------------------------------------------

/// Minn. Stat. § 13.04, subd. 3: comply immediately if possible, or within ten days of the request,
/// excluding Saturdays, Sundays and legal holidays.
pub const SUBJECT_DAYS: u32 = 10;
pub const SUBJECT_CITATION: &str = "Minn. Stat. § 13.04, subd. 3";

/// Holidays Minn. Stat. § 645.44, subd. 5 lets local governments choose to observe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OptionalHolidays {
    pub indigenous_peoples_day: bool,
    pub day_after_thanksgiving: bool,
}

fn nth_weekday(year: i32, month: u32, day: Weekday, n: u8) -> Option<NaiveDate> {
    NaiveDate::from_weekday_of_month_opt(year, month, day, n)
}

fn last_weekday(year: i32, month: u32, day: Weekday) -> Option<NaiveDate> {
    (1..=5u8)
        .rev()
        .find_map(|n| nth_weekday(year, month, day, n))
}

/// Legal holidays under Minn. Stat. § 645.44, subd. 5 for one year, with the fixed-date ones
/// moved to Friday or Monday when they fall on a weekend.
pub fn mn_holidays(year: i32, opt: OptionalHolidays) -> Vec<NaiveDate> {
    let fixed = |m: u32, d: u32| NaiveDate::from_ymd_opt(year, m, d);
    let mut out = Vec::new();
    // New Year's Day, Juneteenth, Independence Day, Veterans Day and Christmas: Sunday -> Monday,
    // Saturday -> Friday (the Friday may fall in the year before, for New Year's Day).
    for d in [
        fixed(1, 1),
        fixed(6, 19),
        fixed(7, 4),
        fixed(11, 11),
        fixed(12, 25),
    ]
    .into_iter()
    .flatten()
    {
        out.push(match d.weekday() {
            Weekday::Sat => d - Duration::days(1),
            Weekday::Sun => d + Duration::days(1),
            _ => d,
        });
    }
    let thanksgiving = nth_weekday(year, 11, Weekday::Thu, 4);
    out.extend(
        [
            nth_weekday(year, 1, Weekday::Mon, 3), // Martin Luther King's Birthday
            nth_weekday(year, 2, Weekday::Mon, 3), // Washington's and Lincoln's Birthday
            last_weekday(year, 5, Weekday::Mon),   // Memorial Day
            nth_weekday(year, 9, Weekday::Mon, 1), // Labor Day
            thanksgiving,
        ]
        .into_iter()
        .flatten(),
    );
    if opt.indigenous_peoples_day {
        out.extend(nth_weekday(year, 10, Weekday::Mon, 2));
    }
    if opt.day_after_thanksgiving {
        out.extend(thanksgiving.map(|t| t + Duration::days(1)));
    }
    out.sort();
    out
}

pub fn is_business_day(d: NaiveDate, opt: OptionalHolidays) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
        // An observed New Year's Day can fall in the year before, so check next year's list too.
        && ![d.year(), d.year() + 1]
            .iter()
            .any(|&y| mn_holidays(y, opt).contains(&d))
}

/// The date `n` business days after `start` (day one is the next business day).
pub fn add_business_days(start: NaiveDate, n: u32, opt: OptionalHolidays) -> NaiveDate {
    let mut d = start;
    let mut left = n;
    while left > 0 {
        d += Duration::days(1);
        if is_business_day(d, opt) {
            left -= 1;
        }
    }
    d
}

/// The last day to respond to a person asking for data about themselves.
pub fn subject_deadline(received: NaiveDate, opt: OptionalHolidays) -> NaiveDate {
    add_business_days(received, SUBJECT_DAYS, opt)
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Level {
    /// Must be fixed before the file can be saved.
    Error,
    /// Probably a mistake; saving is still allowed.
    Warning,
    /// Good to know.
    Info,
}

/// The wizard step a notice belongs to, so the UI can link straight to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash, Serialize, Deserialize)]
pub enum Step {
    #[default]
    About,
    Whose,
    Words,
    When,
    Types,
    Narrow,
    Review,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub level: Level,
    pub step: Step,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct Built {
    pub query: String,
    pub notices: Vec<Notice>,
}

impl Built {
    pub fn can_save(&self) -> bool {
        !self.query.is_empty() && !self.notices.iter().any(|n| n.level == Level::Error)
    }

    pub fn for_step(&self, step: Step) -> impl Iterator<Item = &Notice> {
        self.notices.iter().filter(move |n| n.step == step)
    }
}

/// Purview's limit for SharePoint/OneDrive search queries.
pub const LENGTH_LIMIT: usize = 4000;

struct Ctx<'a> {
    notices: &'a mut Vec<Notice>,
}

impl Ctx<'_> {
    fn push(&mut self, level: Level, step: Step, message: impl Into<String>) {
        let message = message.into();
        if !self.notices.iter().any(|n| n.message == message) {
            self.notices.push(Notice {
                level,
                step,
                message,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

const RESERVED: &[&str] = &["AND", "OR", "NOT", "NEAR", "ONEAR", "XRANK", "WORDS"];

/// Tidy one entry as typed: drops double quotes and collapses whitespace.
pub fn clean_entry(raw: &str) -> String {
    raw.replace('"', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rough check for a person entry: a full address or a bare domain.
pub fn looks_like_address(entry: &str) -> bool {
    let e = entry.trim();
    match e.split_once('@') {
        Some((user, domain)) => !user.is_empty() && domain.contains('.') && !domain.ends_with('.'),
        None => !e.contains(' ') && e.contains('.') && !e.starts_with('.') && !e.ends_with('.'),
    }
}

/// Search details picked out of a pasted request, offered to the user as suggestions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub emails: Vec<String>,
    /// Text in double quotes ("..." or “...”).
    pub phrases: Vec<String>,
    /// Every date written out in full, oldest first.
    pub dates: Vec<NaiveDate>,
}

impl Found {
    pub fn is_empty(&self) -> bool {
        self.emails.is_empty() && self.phrases.is_empty() && self.dates.is_empty()
    }
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

fn month_number(word: &str) -> Option<u32> {
    let w = word.trim_end_matches('.').to_lowercase();
    if w.len() < 3 {
        return None;
    }
    let w = if w == "sept" { "sep".to_owned() } else { w };
    MONTHS
        .iter()
        .position(|m| *m == w || (w.len() == 3 && m.starts_with(&w)))
        .map(|i| i as u32 + 1)
}

fn numeric_date(token: &str) -> Option<NaiveDate> {
    ["%m/%d/%Y", "%m-%d-%Y", "%Y-%m-%d"]
        .iter()
        .find_map(|f| NaiveDate::parse_from_str(token, f).ok())
        .filter(|d| (1990..=2100).contains(&d.year()))
}

/// Pick out email addresses, quoted phrases and dates ("03/15/2026", "2026-03-15",
/// "March 15, 2026", "Mar 15 2026") from text such as a pasted data request.
pub fn find_details(text: &str) -> Found {
    let mut found = Found::default();

    let edge = |c: char| {
        !(c.is_alphanumeric() || c == '@' || c == '/' || c == '-' || c == '_' || c == '.')
    };
    let words: Vec<&str> = text
        .split(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    ',' | ';' | '<' | '>' | '(' | ')' | '[' | ']' | '"' | '“' | '”'
                )
        })
        .map(|w| w.trim_matches(edge).trim_end_matches('.'))
        .filter(|w| !w.is_empty())
        .collect();

    for w in &words {
        if w.contains('@') && looks_like_address(w) {
            let e = w.to_lowercase();
            if !found.emails.contains(&e) {
                found.emails.push(e);
            }
        } else if let Some(d) = numeric_date(w) {
            found.dates.push(d);
        }
    }
    // "March 15, 2026" / "Mar 15 2026"
    for win in words.windows(3) {
        let Some(m) = month_number(win[0]) else {
            continue;
        };
        let day = win[1].trim_end_matches(|c: char| c.is_alphabetic());
        let (Ok(day), Ok(year)) = (day.parse::<u32>(), win[2].parse::<i32>()) else {
            continue;
        };
        if let Some(d) =
            NaiveDate::from_ymd_opt(year, m, day).filter(|d| (1990..=2100).contains(&d.year()))
        {
            found.dates.push(d);
        }
    }
    found.dates.sort();
    found.dates.dedup();

    let mut rest = text;
    while let Some(open) = rest.find(['"', '“']) {
        let after = &rest[open + rest[open..].chars().next().map_or(1, char::len_utf8)..];
        let Some(close) = after.find(['"', '”']) else {
            break;
        };
        let p = clean_entry(&after[..close]);
        let fits = !p.is_empty() && p.chars().count() <= 60 && !p.contains('@');
        if fits && !found.phrases.iter().any(|x| x.eq_ignore_ascii_case(&p)) {
            found.phrases.push(p);
        }
        rest = &after[close + after[close..].chars().next().map_or(1, char::len_utf8)..];
    }
    found
}

fn term(raw: &str, step: Step, cx: &mut Ctx) -> Option<String> {
    let cleaned = clean_entry(raw);
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.starts_with('*') || cleaned.starts_with('?') {
        cx.push(
            Level::Warning,
            step,
            format!(
                "\"{cleaned}\" starts with *. A * only works at the end of a word \
                 (for example budg* finds budget and budgets)."
            ),
        );
    }
    let needs_quotes = cleaned
        .chars()
        .any(|c| c.is_whitespace() || "():<>=,;[]{}\\^~".contains(c))
        || cleaned.starts_with('-')
        || cleaned.starts_with('+')
        || RESERVED.contains(&cleaned.as_str());
    Some(if needs_quotes {
        format!("\"{cleaned}\"")
    } else {
        cleaned
    })
}

fn group(parts: &[String], op: &str) -> Option<String> {
    match parts.len() {
        0 => None,
        1 => Some(parts[0].clone()),
        _ => Some(format!("({})", parts.join(&format!(" {op} ")))),
    }
}

fn terms(values: &[String], prop: Option<&str>, step: Step, cx: &mut Ctx) -> Vec<String> {
    values
        .iter()
        .filter_map(|v| term(v, step, cx))
        .map(|v| match prop {
            Some(p) => format!("{p}:{v}"),
            None => v,
        })
        .collect()
}

fn any_of(values: &[String], prop: &str, step: Step, cx: &mut Ctx) -> Option<String> {
    group(&terms(values, Some(prop), step, cx), "OR")
}

fn check_people(values: &[String], step: Step, cx: &mut Ctx) {
    for v in values {
        let v = clean_entry(v);
        if !v.is_empty() && !looks_like_address(&v) {
            cx.push(
                Level::Warning,
                step,
                format!(
                    "\"{v}\" isn't a full email address. Names can match the wrong person, so use \
                     addresses like name@example.gov where you can."
                ),
            );
        }
    }
}

fn iso(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// The heading for one part of the summary in the email, the records copy and the request file.
pub fn section_title(step: Step) -> Option<&'static str> {
    Some(match step {
        Step::About => "Request",
        Step::Whose => "Whose content to search",
        Step::Words => "Words",
        Step::When => "Dates",
        Step::Types => "Types of items",
        Step::Narrow => "Other filters",
        Step::Review => return None,
    })
}

/// "March 5, 2026"
pub fn long_date(d: NaiveDate) -> String {
    d.format("%B %-d, %Y").to_string()
}

/// "Thursday, March 5, 2026"
pub fn weekday_date(d: NaiveDate) -> String {
    d.format("%A, %B %-d, %Y").to_string()
}

fn non_empty(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|v| clean_entry(v))
        .filter(|v| !v.is_empty())
        .collect()
}

/// `"a"`, `"a" or "b"`, `"a", "b" or "c"`.
fn quoted_list(values: &[String], conj: &str) -> String {
    let q: Vec<String> = values.iter().map(|v| format!("\"{v}\"")).collect();
    plain_list(&q, conj)
}

fn plain_list<S: AsRef<str>>(values: &[S], conj: &str) -> String {
    let v: Vec<&str> = values.iter().map(AsRef::as_ref).collect();
    match v.len() {
        0 => String::new(),
        1 => v[0].to_owned(),
        n => format!("{} {conj} {}", v[..n - 1].join(", "), v[n - 1]),
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

impl SearchRequest {
    pub fn build(&self) -> Built {
        self.build_at(chrono::Local::now().date_naive())
    }

    /// Build with an explicit "today", which fixes preset date ranges.
    pub fn build_at(&self, today: NaiveDate) -> Built {
        let mut notices = Vec::new();
        let mut cx = Ctx {
            notices: &mut notices,
        };
        let mut include: Vec<String> = Vec::new();
        let mut exclude: Vec<String> = Vec::new();

        // The Minnesota deadline for someone asking for data about themselves.
        let a = &self.about;
        if a.kind == RequestKind::AboutThemselves {
            match a.received {
                None => cx.push(
                    Level::Info,
                    Step::About,
                    "Add the date the request was received to work out the 10-business-day deadline.",
                ),
                Some(r) => {
                    let due = subject_deadline(r, self.holidays);
                    if due < today {
                        cx.push(
                            Level::Warning,
                            Step::About,
                            format!(
                                "The deadline to respond was {}. Respond as soon as you can.",
                                weekday_date(due)
                            ),
                        );
                    } else if a.needed_by.is_some_and(|n| n > due) {
                        cx.push(
                            Level::Warning,
                            Step::About,
                            format!(
                                "The results are asked for after the deadline to respond ({}).",
                                weekday_date(due)
                            ),
                        );
                    }
                }
            }
        }

        // Whose content (not part of the query; checked for the email).
        let c = &self.custodians;
        if !c.everyone && non_empty(&c.people).is_empty() && non_empty(&c.locations).is_empty() {
            cx.push(
                Level::Warning,
                Step::Whose,
                "You haven't said whose content to search. Add people or places, or tick \"Search everyone\".",
            );
        }
        if !c.everyone {
            // Names are fine here: the IT Director looks the person up when choosing mailboxes.
            for v in non_empty(&c.people)
                .into_iter()
                .filter(|v| !looks_like_address(v))
            {
                cx.push(
                    Level::Info,
                    Step::Whose,
                    format!(
                        "\"{v}\" is a name. That's fine: the IT Director will find their mailbox. \
                         An email address avoids mix-ups if two people share a name."
                    ),
                );
            }
        }

        // Words
        let k = &self.keywords;
        let scope = match k.scope {
            KeywordScope::Anywhere => None,
            KeywordScope::SubjectTitle => Some("SubjectTitle"),
        };
        let words = terms(&k.words, scope, Step::Words, &mut cx);
        if k.match_all {
            include.extend(words);
        } else {
            include.extend(group(&words, "OR"));
        }
        exclude.extend(terms(&k.none, scope, Step::Words, &mut cx));
        for p in &k.near {
            match (term(&p.first, Step::Words, &mut cx), term(&p.second, Step::Words, &mut cx)) {
                (Some(a), Some(b)) => include.push(format!("({a} NEAR({}) {b})", p.distance.max(1))),
                (None, None) => {}
                _ => cx.push(
                    Level::Error,
                    Step::Words,
                    "A \"words close together\" row is missing one of its words. Fill it in or remove the row.",
                ),
            }
        }

        // Dates
        let d = &self.dates;
        if d.mode == DateMode::Between && d.start > d.end {
            cx.push(
                Level::Info,
                Step::When,
                "The start date was after the end date, so they were swapped.",
            );
        }
        if let Some((from, to)) = d.mode.range(d.start, d.end, today) {
            let restriction = match (from, to) {
                (Some(a), Some(b)) => format!(":{}..{}", iso(a), iso(b)),
                (Some(a), None) => format!(">={}", iso(a)),
                (None, Some(b)) => format!("<={}", iso(b)),
                (None, None) => unreachable!("every limited date choice has at least one side"),
            };
            let parts: Vec<String> = d
                .field
                .properties()
                .iter()
                .map(|p| format!("{p}{restriction}"))
                .collect();
            include.extend(group(&parts, "OR"));
            if let Some(a) = from
                && a > today
            {
                cx.push(
                    Level::Warning,
                    Step::When,
                    "The dates are in the future, so nothing will be found.",
                );
            }
        }

        // Types
        let kinds: Vec<String> = ITEM_TYPES
            .iter()
            .filter(|t| self.item_types.iter().any(|l| l == t.label))
            .flat_map(|t| t.kinds.iter().map(|k| format!("kind:{k}")))
            .collect();
        include.extend(group(&kinds, "OR"));

        // Narrow it down
        let f = &self.filters;
        check_people(&f.sent_by, Step::Narrow, &mut cx);
        check_people(&f.sent_to, Step::Narrow, &mut cx);
        check_people(&f.leave_out_people, Step::Narrow, &mut cx);
        include.extend(any_of(&f.sent_by, "from", Step::Narrow, &mut cx));
        include.extend(any_of(&f.sent_to, "recipients", Step::Narrow, &mut cx));
        exclude.extend(any_of(
            &f.leave_out_people,
            "participants",
            Step::Narrow,
            &mut cx,
        ));
        match f.attachments {
            Attachments::Any => {}
            Attachments::With => include.push("HasAttachment:true".into()),
            Attachments::Without => include.push("HasAttachment:false".into()),
        }
        include.extend(any_of(
            &f.attachment_names,
            "AttachmentNames",
            Step::Narrow,
            &mut cx,
        ));
        if f.high_importance {
            include.push("importance:high".into());
        }
        let mut exts: Vec<String> = FILE_TYPES
            .iter()
            .filter(|(_, label)| f.file_types.iter().any(|l| l == label))
            .flat_map(|(e, _)| e.iter().map(|s| (*s).to_owned()))
            .collect();
        for e in &f.other_extensions {
            let e = e
                .trim()
                .trim_start_matches('*')
                .trim_start_matches('.')
                .to_lowercase();
            if !e.is_empty() && !exts.contains(&e) {
                exts.push(e);
            }
        }
        include.extend(any_of(&exts, "FileExtension", Step::Narrow, &mut cx));
        include.extend(any_of(&f.file_names, "FileName", Step::Narrow, &mut cx));
        include.extend(any_of(&f.authors, "author", Step::Narrow, &mut cx));
        let bytes = (f.size_mb.max(0.0) * 1024.0 * 1024.0).round() as i64;
        match f.size_op {
            SizeOp::Any => {}
            SizeOp::LargerThan => include.push(format!("size>{bytes}")),
            SizeOp::SmallerThan => include.push(format!("size<{bytes}")),
        }
        let sits: Vec<String> = non_empty(&f.sensitive)
            .into_iter()
            .map(|s| format!("SensitiveType:\"{s}\""))
            .collect();
        if !sits.is_empty() {
            let files_only = self.item_types.is_empty()
                || self
                    .item_types
                    .iter()
                    .all(|t| t == "Files in OneDrive and SharePoint");
            cx.push(
                if files_only {
                    Level::Info
                } else {
                    Level::Warning
                },
                Step::Narrow,
                "Sensitive information can only be found in OneDrive and SharePoint files. \
                 Emails and Teams messages won't match this filter.",
            );
        }
        include.extend(group(&sits, "OR"));

        // Assemble
        let negative = group(&exclude, "OR");
        let query = match (include.len(), negative) {
            (0, None) => String::new(),
            (_, None) => include.join(" AND "),
            (0, Some(n)) => {
                cx.push(
                    Level::Error,
                    Step::Words,
                    "You've only said what to leave out. Add at least one thing to look for, such as \
                     words, a date range or a type of item.",
                );
                format!("NOT {n}")
            }
            (1, Some(n)) => format!("{} NOT {n}", include[0]),
            (_, Some(n)) => format!("({}) NOT {n}", include.join(" AND ")),
        };

        if query.is_empty() {
            cx.push(
                Level::Error,
                Step::Words,
                "There's nothing to search for yet. Add some words, a date range or a type of item.",
            );
        } else {
            if non_empty(&k.words).is_empty() && k.near.is_empty() {
                cx.push(
                    Level::Info,
                    Step::Words,
                    "No words were added, so everything that matches your other choices will be found.",
                );
            }
            if d.mode == DateMode::AnyTime {
                cx.push(
                    Level::Info,
                    Step::When,
                    "No dates were chosen, so every date will be searched. A date range makes results smaller and faster.",
                );
            }
            if query.len() > LENGTH_LIMIT {
                cx.push(
                    Level::Warning,
                    Step::Review,
                    "This request is very long. Purview may refuse it for OneDrive and SharePoint; \
                     try fewer words or split it into two requests.",
                );
            }
        }

        notices.sort_by_key(|n| n.level);
        Built { query, notices }
    }

    /// Plain-English lines describing what will be searched for, grouped by step.
    pub fn describe(&self, today: NaiveDate) -> Vec<(Step, Vec<String>)> {
        let mut out = Vec::new();

        let a = &self.about;
        let mut about = Vec::new();
        if a.kind != RequestKind::NotSaid {
            about.push(format!("Type: {}", a.kind.label()));
        }
        about.push(format!("Name: {}", self.display_name(today)));
        if !a.reference.trim().is_empty() {
            about.push(format!("Reference number: {}", a.reference.trim()));
        }
        if !a.reason.trim().is_empty() {
            about.push(format!("What was asked for: {}", a.reason.trim()));
        }
        if let Some(r) = a.received {
            about.push(format!("Received {}", long_date(r)));
            if a.kind == RequestKind::AboutThemselves {
                about.push(format!(
                    "Minnesota deadline to respond: {} ({SUBJECT_DAYS} business days, {SUBJECT_CITATION})",
                    weekday_date(subject_deadline(r, self.holidays))
                ));
            }
        }
        if let Some(n) = a.needed_by {
            about.push(format!("Needed by {}", long_date(n)));
        }
        out.push((Step::About, about));

        let c = &self.custodians;
        let mut whose = Vec::new();
        if c.everyone {
            whose.push("Everyone in the organization".to_owned());
        } else {
            let p = non_empty(&c.people);
            if !p.is_empty() {
                whose.push(format!("People: {}", plain_list(&p, "and")));
            }
            let l = non_empty(&c.locations);
            if !l.is_empty() {
                whose.push(format!("Other places: {}", plain_list(&l, "and")));
            }
            if whose.is_empty() {
                whose.push("Not given".to_owned());
            }
        }
        out.push((Step::Whose, whose));

        let k = &self.keywords;
        let place = match k.scope {
            KeywordScope::Anywhere => "",
            KeywordScope::SubjectTitle => " in the subject line or file title",
        };
        let mut words = Vec::new();
        let w = non_empty(&k.words);
        match (w.len(), k.match_all) {
            (0, _) => {}
            (1, _) => words.push(format!("Contains {}{place}", quoted_list(&w, "and"))),
            (_, true) => words.push(format!("Contains all of {}{place}", quoted_list(&w, "and"))),
            (_, false) => words.push(format!("Contains any of {}{place}", quoted_list(&w, "or"))),
        }
        for p in &k.near {
            let (x, y) = (clean_entry(&p.first), clean_entry(&p.second));
            if !x.is_empty() && !y.is_empty() {
                words.push(format!(
                    "Has \"{x}\" within {} words of \"{y}\"",
                    p.distance.max(1)
                ));
            }
        }
        let none = non_empty(&k.none);
        if !none.is_empty() {
            words.push(format!(
                "Leaves out anything containing {}",
                quoted_list(&none, "or")
            ));
        }
        if words.is_empty() {
            words.push("No words (everything that matches the other choices)".to_owned());
        }
        out.push((Step::Words, words));

        let d = &self.dates;
        let when = match d.mode.range(d.start, d.end, today) {
            None => "Any time".to_owned(),
            Some((Some(x), Some(y))) => format!(
                "{} between {} and {}",
                d.field.verb(),
                long_date(x),
                long_date(y)
            ),
            Some((Some(x), None)) => format!("{} on or after {}", d.field.verb(), long_date(x)),
            Some((None, Some(y))) => format!("{} on or before {}", d.field.verb(), long_date(y)),
            Some((None, None)) => "Any time".to_owned(),
        };
        out.push((Step::When, vec![when]));

        let types: Vec<&str> = ITEM_TYPES
            .iter()
            .filter(|t| self.item_types.iter().any(|l| l == t.label))
            .map(|t| t.label)
            .collect();
        out.push((
            Step::Types,
            vec![if types.is_empty() {
                "All types of items".to_owned()
            } else {
                plain_list(&types, "and")
            }],
        ));

        let f = &self.filters;
        let mut narrow = Vec::new();
        for (values, label) in [(&f.sent_by, "Sent by"), (&f.sent_to, "Sent to")] {
            let v = non_empty(values);
            if !v.is_empty() {
                narrow.push(format!("{label} {}", plain_list(&v, "or")));
            }
        }
        match f.attachments {
            Attachments::Any => {}
            Attachments::With => narrow.push("Has attachments".into()),
            Attachments::Without => narrow.push("Has no attachments".into()),
        }
        let v = non_empty(&f.attachment_names);
        if !v.is_empty() {
            narrow.push(format!("Has an attachment named {}", quoted_list(&v, "or")));
        }
        if f.high_importance {
            narrow.push("Marked high importance".into());
        }
        let mut ft: Vec<String> = f.file_types.clone();
        ft.extend(
            non_empty(&f.other_extensions)
                .iter()
                .map(|e| format!(".{}", e.trim_start_matches('.'))),
        );
        if !ft.is_empty() {
            narrow.push(format!("File type is {}", plain_list(&ft, "or")));
        }
        for (values, label) in [
            (&f.file_names, "File name is"),
            (&f.authors, "File written by"),
        ] {
            let v = non_empty(values);
            if !v.is_empty() {
                narrow.push(format!("{label} {}", quoted_list(&v, "or")));
            }
        }
        match f.size_op {
            SizeOp::Any => {}
            SizeOp::LargerThan => narrow.push(format!("Larger than {} MB", f.size_mb)),
            SizeOp::SmallerThan => narrow.push(format!("Smaller than {} MB", f.size_mb)),
        }
        let s: Vec<&str> = f
            .sensitive
            .iter()
            .map(|n| {
                SENSITIVE_TYPES
                    .iter()
                    .find(|(p, _)| p == n)
                    .map_or(n.as_str(), |(_, l)| *l)
            })
            .collect();
        if !s.is_empty() {
            narrow.push(format!("Contains {}", plain_list(&s, "or").to_lowercase()));
        }
        let ex = non_empty(&f.leave_out_people);
        if !ex.is_empty() {
            narrow.push(format!(
                "Leaves out anything sent by or to {}",
                plain_list(&ex, "or")
            ));
        }
        if narrow.is_empty() {
            narrow.push("No extra filters".to_owned());
        }
        out.push((Step::Narrow, narrow));
        out
    }

    /// File name for the saved request, e.g. `Smith records request 2026-10-23.req`.
    /// The request's name, or "<kind> <date>" when none was given.
    pub fn display_name(&self, today: NaiveDate) -> String {
        let name = self.about.name.trim();
        if name.is_empty() {
            format!("{} {}", self.about.kind.short(), long_date(today))
        } else {
            name.to_owned()
        }
    }

    pub fn file_name(&self, today: NaiveDate) -> String {
        let safe: String = self
            .about
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                    c
                } else {
                    ' '
                }
            })
            .collect();
        let safe = safe.split_whitespace().collect::<Vec<_>>().join(" ");
        let base = if safe.is_empty() {
            self.about.kind.short().to_owned()
        } else {
            safe
        };
        format!("{base} {}.req", iso(today))
    }

    /// Subject and body for the email that goes with the file.
    /// The email to the IT Director, who runs the attached file with the eDiscovery Search Runner.
    pub fn email(&self, file_name: &str, today: NaiveDate) -> (String, String) {
        let subject = format!("eDiscovery search request: {}", self.display_name(today));
        let mut body = format!(
            "Hello,\n\nPlease run the Purview search in the attached request file ({file_name}).\n\
             Open it with the eDiscovery Search Runner to check the details and run the search.\n"
        );
        for (step, lines) in self.describe(today) {
            let Some(heading) = section_title(step) else {
                continue;
            };
            body.push_str(&format!("\n{heading}:\n"));
            for l in lines {
                body.push_str(&format!("  - {l}\n"));
            }
        }
        body.push_str("\nThank you.\n");
        (subject, body)
    }

    /// A plain-text record of the request for the Responsible Authority's files: what was asked
    /// for, the plain-English summary and the exact search that went to the IT Director.
    pub fn record(&self, file_name: &str, query: &str, today: NaiveDate) -> String {
        let mut out = format!(
            "eDiscovery search request: {}\nSaved {}\nRequest file: {file_name}\n",
            self.display_name(today),
            long_date(today)
        );
        for (step, lines) in self.describe(today) {
            let Some(heading) = section_title(step) else {
                continue;
            };
            out.push_str(&format!("\n{heading}:\n"));
            for l in lines {
                out.push_str(&format!("  - {l}\n"));
            }
        }
        out.push_str(&format!(
            "\nThe search, as sent in the request file (Purview KeyQL):\n{query}\n"
        ));
        out
    }

    /// A filled-in sample request.
    pub fn example(today: NaiveDate) -> Self {
        Self {
            about: About {
                kind: RequestKind::PublicData,
                name: "Example - Highway 56 contract".into(),
                reason:
                    "All emails and documents this year between Jane Doe and Pat Lee about the \
                         \"Highway 56\" contract, with attachments. No newsletters."
                        .into(),
                reference: "DR-2026-041".into(),
                received: Some(today - Duration::days(2)),
                needed_by: Some(today + Duration::days(10)),
            },
            custodians: Custodians {
                people: vec!["jane.doe@example.gov".into(), "pat.lee@example.gov".into()],
                ..Default::default()
            },
            keywords: Keywords {
                words: vec!["Highway 56".into(), "contract".into()],
                match_all: true,
                none: vec!["newsletter".into()],
                ..Default::default()
            },
            dates: Dates {
                mode: DateMode::ThisYear,
                ..Dates::new(today)
            },
            item_types: vec!["Email".into(), "Files in OneDrive and SharePoint".into()],
            filters: Filters {
                attachments: Attachments::With,
                ..Default::default()
            },
            holidays: OptionalHolidays::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    const TODAY: (i32, u32, u32) = (2026, 10, 23);

    fn today() -> NaiveDate {
        d(TODAY.0, TODAY.1, TODAY.2)
    }

    fn named() -> SearchRequest {
        let mut s = SearchRequest::default();
        s.about.name = "Test".into();
        s.custodians.everyone = true;
        s.dates = Dates::new(today());
        s
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    fn errors(b: &Built) -> Vec<&str> {
        b.notices
            .iter()
            .filter(|n| n.level == Level::Error)
            .map(|n| n.message.as_str())
            .collect()
    }

    #[test]
    fn empty_request_cannot_be_saved() {
        let b = SearchRequest::default().build_at(today());
        assert_eq!(b.query, "");
        assert!(!b.can_save());
        // Only "nothing to search for": the name is optional.
        assert_eq!(errors(&b).len(), 1, "{:?}", b.notices);
        assert!(
            b.notices
                .iter()
                .any(|n| n.step == Step::Whose && n.level == Level::Warning)
        );
    }

    #[test]
    fn words_any_and_all() {
        let mut s = named();
        s.keywords.words = v(&["budget", "fiscal year", "\"county board\""]);
        assert_eq!(
            s.build_at(today()).query,
            r#"(budget OR "fiscal year" OR "county board")"#
        );
        s.keywords.match_all = true;
        s.keywords.none = v(&["newsletter", "spam"]);
        let b = s.build_at(today());
        assert_eq!(
            b.query,
            r#"(budget AND "fiscal year" AND "county board") NOT (newsletter OR spam)"#
        );
        assert!(b.can_save());
    }

    #[test]
    fn subject_scope_and_reserved_words() {
        let mut s = named();
        s.keywords.words = v(&["NEAR", "merger"]);
        s.keywords.match_all = true;
        s.keywords.scope = KeywordScope::SubjectTitle;
        assert_eq!(
            s.build_at(today()).query,
            r#"SubjectTitle:"NEAR" AND SubjectTitle:merger"#
        );
    }

    #[test]
    fn leading_wildcard_warns() {
        let mut s = named();
        s.keywords.words = v(&["*port"]);
        let b = s.build_at(today());
        assert!(
            b.notices
                .iter()
                .any(|n| n.level == Level::Warning && n.step == Step::Words)
        );
    }

    #[test]
    fn near_needs_both_words() {
        let mut s = named();
        s.keywords.near.push(Near {
            first: "contract".into(),
            second: "breach".into(),
            distance: 5,
        });
        assert_eq!(s.build_at(today()).query, "(contract NEAR(5) breach)");
        s.keywords.near.push(Near {
            first: "late".into(),
            ..Default::default()
        });
        assert!(!s.build_at(today()).can_save());
    }

    #[test]
    fn only_exclusions_is_an_error() {
        let mut s = named();
        s.keywords.none = v(&["spam"]);
        let b = s.build_at(today());
        assert_eq!(b.query, "NOT spam");
        assert!(!b.can_save());
    }

    #[test]
    fn date_choices_become_fixed_dates() {
        let mut s = named();
        let q = |s: &SearchRequest| s.build_at(today()).query;
        s.dates.field = DateField::Sent;
        s.dates.mode = DateMode::Last30Days;
        assert_eq!(q(&s), "sent:2026-09-23..2026-10-23");
        s.dates.mode = DateMode::Last12Months;
        assert_eq!(q(&s), "sent:2025-10-23..2026-10-23");
        s.dates.mode = DateMode::ThisYear;
        assert_eq!(q(&s), "sent:2026-01-01..2026-10-23");
        s.dates.mode = DateMode::LastYear;
        assert_eq!(q(&s), "sent:2025-01-01..2025-12-31");
        s.dates.mode = DateMode::Between;
        s.dates.start = d(2026, 3, 31);
        s.dates.end = d(2026, 1, 1);
        let b = s.build_at(today());
        assert_eq!(b.query, "sent:2026-01-01..2026-03-31");
        assert!(b.notices.iter().any(|n| n.message.contains("swapped")));
        s.dates.field = DateField::Any;
        s.dates.mode = DateMode::OnOrAfter;
        s.dates.start = d(2026, 10, 1);
        assert_eq!(
            q(&s),
            "(sent>=2026-10-01 OR received>=2026-10-01 OR LastModifiedTime>=2026-10-01)"
        );
        s.dates.field = DateField::Created;
        s.dates.mode = DateMode::OnOrBefore;
        assert_eq!(q(&s), "Created<=2026-01-01");
        s.dates.mode = DateMode::OnOrAfter;
        s.dates.start = d(2027, 1, 1);
        assert!(
            s.build_at(today())
                .notices
                .iter()
                .any(|n| n.message.contains("future"))
        );
    }

    #[test]
    fn types_and_people_filters() {
        let mut s = named();
        s.item_types = v(&["Email", "Teams chats and meetings"]);
        s.filters.sent_by = v(&["alice@example.gov", "Bob Smith"]);
        s.filters.leave_out_people = v(&["noreply@example.gov"]);
        let b = s.build_at(today());
        assert_eq!(
            b.query,
            r#"((kind:email OR kind:microsoftteams OR kind:im) AND (from:alice@example.gov OR from:"Bob Smith")) NOT participants:noreply@example.gov"#
        );
        assert!(
            b.notices
                .iter()
                .any(|n| n.level == Level::Warning && n.message.contains("Bob Smith"))
        );
    }

    #[test]
    fn address_check() {
        assert!(looks_like_address("jane@example.gov"));
        assert!(looks_like_address("example.gov"));
        assert!(!looks_like_address("Jane Doe"));
        assert!(!looks_like_address("jane@"));
        assert!(!looks_like_address("jane@localhost"));
    }

    #[test]
    fn missing_custodians_warns_but_saves() {
        let mut s = named();
        s.custodians.everyone = false;
        s.keywords.words = v(&["budget"]);
        let b = s.build_at(today());
        assert!(b.can_save());
        assert!(
            b.notices
                .iter()
                .any(|n| n.step == Step::Whose && n.level == Level::Warning)
        );
    }

    #[test]
    fn filters() {
        let mut s = named();
        s.filters.attachments = Attachments::With;
        s.filters.high_importance = true;
        s.filters.file_types = v(&["PDF", "Word"]);
        s.filters.other_extensions = v(&[".DWG", "pdf"]);
        s.filters.size_op = SizeOp::LargerThan;
        s.filters.size_mb = 2.0;
        s.filters.sensitive = v(&["Credit Card Number"]);
        let b = s.build_at(today());
        assert_eq!(
            b.query,
            "HasAttachment:true AND importance:high \
             AND (FileExtension:docx OR FileExtension:doc OR FileExtension:pdf OR FileExtension:dwg) \
             AND size>2097152 AND SensitiveType:\"Credit Card Number\""
        );
        assert!(
            b.notices
                .iter()
                .any(|n| n.level == Level::Info && n.message.contains("OneDrive"))
        );
        s.item_types = v(&["Email"]);
        assert!(
            s.build_at(today())
                .notices
                .iter()
                .any(|n| n.level == Level::Warning && n.message.contains("OneDrive"))
        );
    }

    #[test]
    fn embedded_quotes_are_removed() {
        let mut s = named();
        s.filters.attachment_names = v(&[r#"my "final" report.docx"#]);
        assert_eq!(
            s.build_at(today()).query,
            r#"AttachmentNames:"my final report.docx""#
        );
    }

    #[test]
    fn example_builds_describes_and_emails() {
        let s = SearchRequest::example(today());
        let b = s.build_at(today());
        assert_eq!(
            b.query,
            "(\"Highway 56\" AND contract \
             AND (sent:2026-01-01..2026-10-23 OR received:2026-01-01..2026-10-23 OR LastModifiedTime:2026-01-01..2026-10-23) \
             AND (kind:email OR kind:docs) AND HasAttachment:true) NOT newsletter"
        );
        assert!(b.can_save(), "{:?}", b.notices);
        let described = s.describe(today());
        assert_eq!(
            described[2].1,
            vec![
                "Contains all of \"Highway 56\" and \"contract\"",
                "Leaves out anything containing \"newsletter\"",
            ]
        );
        assert_eq!(
            described[3].1,
            vec!["Dated between January 1, 2026 and October 23, 2026"]
        );
        let (subject, body) = s.email("x.req", today());
        assert_eq!(
            subject,
            "eDiscovery search request: Example - Highway 56 contract"
        );
        assert!(body.contains(
            "Whose content to search:\n  - People: jane.doe@example.gov and pat.lee@example.gov"
        ));
        assert!(body.contains("(x.req)"));
    }

    #[test]
    fn finds_details_in_a_pasted_request() {
        let text = "Please send all emails between Jane Doe (jane.doe@example.gov) and \
                    <pat.lee@Example.gov> about the \u{201c}Highway 56\u{201d} project or \"bid tabulation\" \
                    from January 5, 2026 to 3/31/2026. Also Sept. 2 2025, and 2026-02-30 (not a date).";
        let f = find_details(text);
        assert_eq!(
            f.emails,
            vec!["jane.doe@example.gov", "pat.lee@example.gov"]
        );
        assert_eq!(f.phrases, vec!["Highway 56", "bid tabulation"]);
        assert_eq!(f.dates, vec![d(2025, 9, 2), d(2026, 1, 5), d(2026, 3, 31)]);
        assert!(find_details("nothing to see here").is_empty());
        assert_eq!(find_details("on May 1st, 2026").dates, vec![d(2026, 5, 1)]);
    }

    #[test]
    fn unnamed_requests_use_their_kind() {
        let mut s = named();
        s.about.name.clear();
        s.about.kind = RequestKind::PublicData;
        s.keywords.words = v(&["budget"]);
        assert_eq!(s.file_name(today()), "Data request 2026-10-23.req");
        assert_eq!(s.display_name(today()), "Data request October 23, 2026");
        assert!(s.build_at(today()).can_save());
        let (subject, _) = s.email("x.req", today());
        assert_eq!(
            subject,
            "eDiscovery search request: Data request October 23, 2026"
        );
    }

    #[test]
    fn names_of_people_to_search_are_fine() {
        let mut s = named();
        s.custodians.everyone = false;
        s.custodians.people = v(&["Jane Doe", "pat.lee@example.gov"]);
        s.keywords.words = v(&["budget"]);
        let b = s.build_at(today());
        assert!(
            b.notices.iter().all(|n| n.level == Level::Info),
            "{:?}",
            b.notices
        );
        assert_eq!(b.for_step(Step::Whose).count(), 1, "{:?}", b.notices);
    }

    #[test]
    fn record_has_summary_and_query() {
        let s = SearchRequest::example(today());
        let b = s.build_at(today());
        let r = s.record("x.req", &b.query, today());
        assert!(r.starts_with("eDiscovery search request: Example - Highway 56 contract\nSaved October 23, 2026\nRequest file: x.req\n"));
        assert!(r.contains("  - Type: Data request from the public\n"));
        assert!(r.contains("  - Reference number: DR-2026-041\n"));
        assert!(r.ends_with(&format!("(Purview KeyQL):\n{}\n", b.query)));
    }

    #[test]
    fn minnesota_holidays() {
        let none = OptionalHolidays::default();
        // 2026: Jan 1 Thu, MLK Jan 19, Presidents Feb 16, Memorial May 25, Juneteenth Fri Jun 19,
        // July 4 is a Saturday -> Fri Jul 3, Labor Sep 7, Veterans Wed Nov 11, Thanksgiving Nov 26, Christmas Fri.
        assert_eq!(
            mn_holidays(2026, none),
            vec![
                d(2026, 1, 1),
                d(2026, 1, 19),
                d(2026, 2, 16),
                d(2026, 5, 25),
                d(2026, 6, 19),
                d(2026, 7, 3),
                d(2026, 9, 7),
                d(2026, 11, 11),
                d(2026, 11, 26),
                d(2026, 12, 25),
            ]
        );
        let all = OptionalHolidays {
            indigenous_peoples_day: true,
            day_after_thanksgiving: true,
        };
        let h = mn_holidays(2026, all);
        assert!(h.contains(&d(2026, 10, 12)) && h.contains(&d(2026, 11, 27)));
        // Sunday holidays move to Monday: Christmas 2022, Juneteenth 2022.
        assert!(mn_holidays(2022, none).contains(&d(2022, 12, 26)));
        assert!(mn_holidays(2022, none).contains(&d(2022, 6, 20)));
        // New Year's Day 2028 is a Saturday: observed Friday, December 31, 2027.
        assert!(!is_business_day(d(2027, 12, 31), none));
    }

    #[test]
    fn ten_business_days() {
        let none = OptionalHolidays::default();
        // Received Friday Oct 23, 2026: Oct 26-30 and Nov 2-6 -> Friday Nov 6.
        assert_eq!(subject_deadline(d(2026, 10, 23), none), d(2026, 11, 6));
        // Received Monday Nov 9: Veterans Day (Wed 11) and Thanksgiving (Thu 26) are skipped.
        // 10, 12, 13, 16, 17, 18, 19, 20, 23, 24 -> Tuesday Nov 24.
        assert_eq!(subject_deadline(d(2026, 11, 9), none), d(2026, 11, 24));
        // Received Thu Nov 19: 20, 23, 24, 25, (26 off), 27, 30, Dec 1, 2, 3, 4 -> Fri Dec 4.
        assert_eq!(subject_deadline(d(2026, 11, 19), none), d(2026, 12, 4));
        // With the day after Thanksgiving off as well, one day later: Mon Dec 7.
        let fri = OptionalHolidays {
            day_after_thanksgiving: true,
            ..none
        };
        assert_eq!(subject_deadline(d(2026, 11, 19), fri), d(2026, 12, 7));
        // A request that comes in on a Saturday still counts from the next business day.
        assert_eq!(subject_deadline(d(2026, 10, 24), none), d(2026, 11, 6));
    }

    #[test]
    fn deadline_checks() {
        let mut s = named();
        s.keywords.words = v(&["budget"]);
        s.about.kind = RequestKind::AboutThemselves;
        let about = |s: &SearchRequest| -> Vec<(Level, String)> {
            s.build_at(today())
                .for_step(Step::About)
                .map(|n| (n.level, n.message.clone()))
                .collect()
        };
        assert_eq!(about(&s)[0].0, Level::Info); // no date received yet
        s.about.received = Some(d(2026, 10, 1)); // due Thu Oct 15, before "today" (Oct 23)
        assert_eq!(
            about(&s),
            vec![(Level::Warning, "The deadline to respond was Thursday, October 15, 2026. Respond as soon as you can.".to_owned())]
        );
        s.about.received = Some(d(2026, 10, 23)); // due Fri Nov 6
        s.about.needed_by = Some(d(2026, 11, 9));
        assert_eq!(about(&s)[0].0, Level::Warning);
        s.about.needed_by = Some(d(2026, 11, 4));
        assert!(about(&s).is_empty());
        assert!(s.build_at(today()).can_save());
    }

    #[test]
    fn deadline_in_summary_only_for_subject_requests() {
        let mut s = named();
        s.about.received = Some(d(2026, 10, 23));
        let line = "Minnesota deadline to respond: Friday, November 6, 2026 (10 business days, Minn. Stat. § 13.04, subd. 3)";
        s.about.kind = RequestKind::PublicData;
        assert!(!s.describe(today())[0].1.iter().any(|l| l == line));
        s.about.kind = RequestKind::AboutThemselves;
        assert!(
            s.describe(today())[0].1.iter().any(|l| l == line),
            "{:?}",
            s.describe(today())
        );
    }

    #[test]
    fn file_name_is_safe() {
        let mut s = named();
        s.about.name = "Smith / Jones: records?".into();
        assert_eq!(s.file_name(today()), "Smith Jones records 2026-10-23.req");
        s.about.name = "  ".into();
        assert_eq!(s.file_name(today()), "Search request 2026-10-23.req");
    }
}
