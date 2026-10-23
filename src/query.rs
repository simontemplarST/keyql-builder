//! Query model and KeyQL generator for Microsoft Purview eDiscovery.
//!
//! The model mirrors the form in the UI. `QuerySpec::build` turns it into a
//! single KeyQL string plus a list of notices. Everything here is pure so it
//! can be unit tested without a GUI.
//!
//! Conventions followed (Purview KeyQL / KQL):
//! - Boolean operators are upper case (`AND`, `OR`, `NOT`, `NEAR(n)`, `ONEAR(n)`).
//! - Values with spaces or special characters are double quoted.
//! - Dates are `YYYY-MM-DD`; ranges use `..`; comparisons use `<`, `<=`, `>`, `>=`.
//! - Only suffix wildcards (`budg*`) are supported by Purview.

use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Option tables shown in the UI
// ---------------------------------------------------------------------------

/// `kind:` values supported by Purview, with friendly labels.
pub const KINDS: &[(&str, &str)] = &[
    ("email", "Email"),
    ("docs", "Documents (SharePoint / OneDrive)"),
    ("microsoftteams", "Microsoft Teams messages"),
    ("im", "Instant messages / chats"),
    ("meetings", "Meetings & appointments"),
    ("contacts", "Contacts"),
    ("tasks", "Tasks"),
    ("notes", "Notes"),
    ("voicemail", "Voicemail"),
    ("faxes", "Faxes"),
    ("journals", "Journals"),
    ("posts", "Posts (public folders)"),
    ("rssfeeds", "RSS feeds"),
    ("externaldata", "Third-party / external data"),
];

/// Common file extensions offered as check boxes.
pub const EXTENSIONS: &[&str] = &[
    "docx", "doc", "xlsx", "xls", "pptx", "ppt", "pdf", "msg", "eml", "txt", "csv", "one", "vsdx",
    "zip", "jpg", "png", "mp4",
];

/// Common built-in sensitive information types.
pub const SENSITIVE_TYPES: &[&str] = &[
    "Credit Card Number",
    "U.S. Social Security Number (SSN)",
    "U.S. Individual Taxpayer Identification Number (ITIN)",
    "U.S. Bank Account Number",
    "ABA Routing Number",
    "U.S. Driver's License Number",
    "U.S. / U.K. Passport Number",
    "International Banking Account Number (IBAN)",
    "SWIFT Code",
    "All Full Names",
    "All Physical Addresses",
    "All Medical Terms And Conditions",
];

/// Properties offered in the "Advanced conditions" builder: (name, hint).
pub const PROPERTIES: &[(&str, &str)] = &[
    ("subject", "Email subject"),
    ("SubjectTitle", "Email subject or document title"),
    ("from", "Sender"),
    ("to", "To recipients"),
    ("cc", "CC recipients"),
    ("bcc", "BCC recipients"),
    ("participants", "From/To/CC/BCC (any)"),
    ("recipients", "To/CC/BCC (any)"),
    ("sent", "Date sent (YYYY-MM-DD)"),
    ("received", "Date received (YYYY-MM-DD)"),
    ("kind", "Item type, e.g. email"),
    ("HasAttachment", "true / false"),
    ("AttachmentNames", "Attachment file name"),
    ("importance", "high / medium / low"),
    ("IsRead", "true / false"),
    ("category", "Outlook category"),
    ("ItemClass", "e.g. IPM.Note"),
    ("size", "Size in bytes"),
    ("author", "Document author"),
    ("title", "Document title"),
    ("FileName", "Document file name"),
    ("FileExtension", "e.g. docx"),
    ("ModifiedBy", "Last modified by"),
    ("Created", "Date created (YYYY-MM-DD)"),
    ("LastModifiedTime", "Date last modified (YYYY-MM-DD)"),
    ("path", "Site or folder URL"),
    ("site", "Site URL"),
    ("ContentType", "SharePoint content type"),
    ("DetectedLanguage", "e.g. en"),
    ("SensitiveType", "Sensitive info type name"),
];

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Tri {
    #[default]
    Any,
    Yes,
    No,
}

impl Tri {
    pub const ALL: [Self; 3] = [Self::Any, Self::Yes, Self::No];

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Doesn't matter",
            Self::Yes => "Yes",
            Self::No => "No",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum KeywordScope {
    #[default]
    Anywhere,
    SubjectTitle,
    Subject,
    Title,
}

impl KeywordScope {
    pub const ALL: [Self; 4] = [
        Self::Anywhere,
        Self::SubjectTitle,
        Self::Subject,
        Self::Title,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Anywhere => "Anywhere (body, subject, metadata)",
            Self::SubjectTitle => "Email subject or document title",
            Self::Subject => "Email subject only",
            Self::Title => "Document title only",
        }
    }

    fn property(self) -> Option<&'static str> {
        match self {
            Self::Anywhere => None,
            Self::SubjectTitle => Some("SubjectTitle"),
            Self::Subject => Some("subject"),
            Self::Title => Some("title"),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Keywords {
    /// Every word must appear.
    pub all_words: String,
    /// Exact phrases separated by `;` or new lines; all must appear.
    pub phrases: String,
    /// At least one of these must appear.
    pub any_words: String,
    /// None of these may appear.
    pub none_words: String,
    pub scope: KeywordScope,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Proximity {
    pub first: String,
    pub second: String,
    pub distance: u32,
    /// `ONEAR` (first must precede second) instead of `NEAR`.
    pub ordered: bool,
}

impl Default for Proximity {
    fn default() -> Self {
        Self {
            first: String::new(),
            second: String::new(),
            distance: 8,
            ordered: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum PersonField {
    #[default]
    Participants,
    From,
    To,
    Cc,
    Bcc,
    Recipients,
    Author,
    ModifiedBy,
}

impl PersonField {
    pub const ALL: [Self; 8] = [
        Self::Participants,
        Self::From,
        Self::To,
        Self::Cc,
        Self::Bcc,
        Self::Recipients,
        Self::Author,
        Self::ModifiedBy,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Participants => "Any participant (from/to/cc/bcc)",
            Self::From => "From (sender)",
            Self::To => "To",
            Self::Cc => "CC",
            Self::Bcc => "BCC",
            Self::Recipients => "Any recipient (to/cc/bcc)",
            Self::Author => "Document author",
            Self::ModifiedBy => "Document modified by",
        }
    }

    pub fn property(self) -> &'static str {
        match self {
            Self::Participants => "participants",
            Self::From => "from",
            Self::To => "to",
            Self::Cc => "cc",
            Self::Bcc => "bcc",
            Self::Recipients => "recipients",
            Self::Author => "author",
            Self::ModifiedBy => "ModifiedBy",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PersonCond {
    pub field: PersonField,
    /// Addresses, names or domains separated by commas, semicolons or new lines.
    pub values: String,
    pub exclude: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum DateField {
    #[default]
    AnyItemDate,
    Sent,
    Received,
    Created,
    LastModified,
}

impl DateField {
    pub const ALL: [Self; 5] = [
        Self::AnyItemDate,
        Self::Sent,
        Self::Received,
        Self::Created,
        Self::LastModified,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::AnyItemDate => "Email sent/received or document modified",
            Self::Sent => "Email sent",
            Self::Received => "Email received",
            Self::Created => "Document created",
            Self::LastModified => "Document last modified",
        }
    }

    fn properties(self) -> &'static [&'static str] {
        match self {
            Self::AnyItemDate => &["sent", "received", "LastModifiedTime"],
            Self::Sent => &["sent"],
            Self::Received => &["received"],
            Self::Created => &["Created"],
            Self::LastModified => &["LastModifiedTime"],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum DateOp {
    #[default]
    Between,
    On,
    Before,
    OnOrBefore,
    After,
    OnOrAfter,
    LastNDays,
    Relative,
}

impl DateOp {
    pub const ALL: [Self; 8] = [
        Self::Between,
        Self::On,
        Self::Before,
        Self::OnOrBefore,
        Self::After,
        Self::OnOrAfter,
        Self::LastNDays,
        Self::Relative,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Between => "is between",
            Self::On => "is on",
            Self::Before => "is before",
            Self::OnOrBefore => "is on or before",
            Self::After => "is after",
            Self::OnOrAfter => "is on or after",
            Self::LastNDays => "is within the last N days",
            Self::Relative => "is in a relative period",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum RelativeDate {
    Today,
    Yesterday,
    #[default]
    ThisWeek,
    ThisMonth,
    LastMonth,
    ThisYear,
    LastYear,
}

impl RelativeDate {
    pub const ALL: [Self; 7] = [
        Self::Today,
        Self::Yesterday,
        Self::ThisWeek,
        Self::ThisMonth,
        Self::LastMonth,
        Self::ThisYear,
        Self::LastYear,
    ];

    fn keyword(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Yesterday => "yesterday",
            Self::ThisWeek => "\"this week\"",
            Self::ThisMonth => "\"this month\"",
            Self::LastMonth => "\"last month\"",
            Self::ThisYear => "\"this year\"",
            Self::LastYear => "\"last year\"",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Yesterday => "Yesterday",
            Self::ThisWeek => "This week",
            Self::ThisMonth => "This month",
            Self::LastMonth => "Last month",
            Self::ThisYear => "This year",
            Self::LastYear => "Last year",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DateCond {
    pub field: DateField,
    pub op: DateOp,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub days: u32,
    pub relative: RelativeDate,
}

impl DateCond {
    pub fn new(today: NaiveDate) -> Self {
        Self {
            field: DateField::default(),
            op: DateOp::default(),
            start: today - Duration::days(30),
            end: today,
            days: 30,
            relative: RelativeDate::default(),
        }
    }
}

impl Default for DateCond {
    fn default() -> Self {
        Self::new(chrono::Local::now().date_naive())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Importance {
    #[default]
    Any,
    High,
    Normal,
    Low,
}

impl Importance {
    pub const ALL: [Self; 4] = [Self::Any, Self::High, Self::Normal, Self::Low];

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any",
            Self::High => "High",
            Self::Normal => "Normal",
            Self::Low => "Low",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailOpts {
    pub has_attachment: Tri,
    pub importance: Importance,
    pub is_read: Tri,
    /// Attachment names, comma/semicolon/new-line separated (any of).
    pub attachment_names: String,
    /// Outlook categories (any of).
    pub categories: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DocOpts {
    /// Extensions picked from `EXTENSIONS`.
    pub extensions: Vec<String>,
    /// Extra extensions typed by the user.
    pub custom_extensions: String,
    /// File names (any of). Trailing `*` allowed.
    pub file_names: String,
    /// Site / folder URLs (any of), emitted as `path:`.
    pub paths: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum SizeOp {
    #[default]
    Any,
    LessThan,
    GreaterThan,
    Between,
}

impl SizeOp {
    pub const ALL: [Self; 4] = [Self::Any, Self::LessThan, Self::GreaterThan, Self::Between];

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any size",
            Self::LessThan => "Smaller than",
            Self::GreaterThan => "Larger than",
            Self::Between => "Between",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum SizeUnit {
    Bytes,
    #[default]
    KB,
    MB,
    GB,
}

impl SizeUnit {
    pub const ALL: [Self; 4] = [Self::Bytes, Self::KB, Self::MB, Self::GB];

    pub fn label(self) -> &'static str {
        match self {
            Self::Bytes => "bytes",
            Self::KB => "KB",
            Self::MB => "MB",
            Self::GB => "GB",
        }
    }

    fn factor(self) -> f64 {
        match self {
            Self::Bytes => 1.0,
            Self::KB => 1024.0,
            Self::MB => 1024.0 * 1024.0,
            Self::GB => 1024.0 * 1024.0 * 1024.0,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SizeCond {
    pub op: SizeOp,
    pub a: f64,
    pub b: f64,
    pub unit: SizeUnit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Confidence {
    #[default]
    Any,
    Low,
    Medium,
    High,
}

impl Confidence {
    pub const ALL: [Self; 4] = [Self::Any, Self::Low, Self::Medium, Self::High];

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any confidence",
            Self::Low => "Low or higher (65+)",
            Self::Medium => "Medium or higher (75+)",
            Self::High => "High (85+)",
        }
    }

    fn min(self) -> Option<u32> {
        match self {
            Self::Any => None,
            Self::Low => Some(65),
            Self::Medium => Some(75),
            Self::High => Some(85),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SensitiveCond {
    pub name: String,
    /// Minimum number of instances (1 = any).
    pub min_count: u32,
    pub confidence: Confidence,
}

impl Default for SensitiveCond {
    fn default() -> Self {
        Self {
            name: SENSITIVE_TYPES[0].to_owned(),
            min_count: 1,
            confidence: Confidence::Any,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Join {
    #[default]
    And,
    Or,
    AndNot,
}

impl Join {
    pub const ALL: [Self; 3] = [Self::And, Self::Or, Self::AndNot];

    pub fn label(self) -> &'static str {
        match self {
            Self::And => "AND",
            Self::Or => "OR",
            Self::AndNot => "AND NOT",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum CondOp {
    #[default]
    Contains,
    Equals,
    NotEquals,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    Range,
}

impl CondOp {
    pub const ALL: [Self; 8] = [
        Self::Contains,
        Self::Equals,
        Self::NotEquals,
        Self::Less,
        Self::LessEq,
        Self::Greater,
        Self::GreaterEq,
        Self::Range,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Contains => ": contains / matches",
            Self::Equals => "= equals",
            Self::NotEquals => "<> not equal",
            Self::Less => "< less than",
            Self::LessEq => "<= at most",
            Self::Greater => "> greater than",
            Self::GreaterEq => ">= at least",
            Self::Range => ".. range",
        }
    }

    fn token(self) -> &'static str {
        match self {
            Self::Contains | Self::Range => ":",
            Self::Equals => "=",
            Self::NotEquals => "<>",
            Self::Less => "<",
            Self::LessEq => "<=",
            Self::Greater => ">",
            Self::GreaterEq => ">=",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Condition {
    pub join: Join,
    pub property: String,
    pub op: CondOp,
    pub value: String,
    pub value2: String,
}

impl Default for Condition {
    fn default() -> Self {
        Self {
            join: Join::And,
            property: "subject".to_owned(),
            op: CondOp::Contains,
            value: String::new(),
            value2: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QuerySpec {
    pub keywords: Keywords,
    pub proximity: Vec<Proximity>,
    pub kinds: Vec<String>,
    pub people: Vec<PersonCond>,
    pub dates: Vec<DateCond>,
    pub email: EmailOpts,
    pub docs: DocOpts,
    pub size: SizeCond,
    pub sensitive: Vec<SensitiveCond>,
    pub advanced: Vec<Condition>,
    /// Hand-written KeyQL ANDed onto the generated query.
    pub raw: String,
}

impl QuerySpec {
    /// A filled-in sample that shows off most of the form.
    pub fn example(today: NaiveDate) -> Self {
        use chrono::Datelike;
        let year = today.year();
        Self {
            keywords: Keywords {
                all_words: "contract".into(),
                phrases: "purchase agreement".into(),
                any_words: "draft revised final".into(),
                none_words: "newsletter".into(),
                scope: KeywordScope::Anywhere,
            },
            proximity: vec![Proximity {
                first: "payment".into(),
                second: "late".into(),
                distance: 5,
                ordered: false,
            }],
            kinds: vec!["email".into(), "docs".into()],
            people: vec![PersonCond {
                field: PersonField::Participants,
                values: "jane.doe@contoso.com; fabrikam.com".into(),
                exclude: false,
            }],
            dates: vec![DateCond {
                field: DateField::AnyItemDate,
                op: DateOp::Between,
                start: NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or(today),
                end: today,
                ..DateCond::new(today)
            }],
            email: EmailOpts {
                has_attachment: Tri::Yes,
                ..Default::default()
            },
            docs: DocOpts {
                extensions: vec!["docx".into(), "pdf".into()],
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub level: Level,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct Built {
    pub query: String,
    pub notices: Vec<Notice>,
}

/// Soft limit for query length. Purview rejects very long queries; around
/// 4,000 characters is a commonly cited ceiling.
pub const LENGTH_WARNING: usize = 4000;

struct Ctx<'a> {
    notices: &'a mut Vec<Notice>,
}

impl Ctx<'_> {
    fn push(&mut self, level: Level, message: impl Into<String>) {
        let message = message.into();
        if !self.notices.iter().any(|n| n.message == message) {
            self.notices.push(Notice { level, message });
        }
    }
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

const RESERVED: &[&str] = &["AND", "OR", "NOT", "NEAR", "ONEAR", "XRANK", "WORDS"];

/// Format one free-text term or property value, quoting when needed.
/// Returns `None` for empty input.
fn term(raw: &str, cx: &mut Ctx) -> Option<String> {
    let mut s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let was_quoted = s.len() >= 2 && s.starts_with('"') && s.ends_with('"');
    if was_quoted {
        s = &s[1..s.len() - 1];
    }
    if s.contains('"') {
        cx.push(
            Level::Warning,
            "Double quotes inside a value were removed (KeyQL cannot escape them).",
        );
    }
    let cleaned = s
        .replace('"', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.starts_with('*') || cleaned.starts_with('?') {
        cx.push(
            Level::Warning,
            format!("'{cleaned}': Purview only supports wildcards at the end of a word (suffix), not the start."),
        );
    }
    let needs_quotes = was_quoted
        || cleaned
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

/// Split free text into terms on whitespace, keeping "quoted phrases" whole.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for c in text.chars() {
        match c {
            '"' => {
                cur.push(c);
                if in_quotes {
                    out.push(std::mem::take(&mut cur));
                }
                in_quotes = !in_quotes;
            }
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        if in_quotes {
            cur.push('"');
        }
        out.push(cur);
    }
    out.retain(|t| !t.trim_matches('"').trim().is_empty());
    out
}

/// Split the exact-phrase field on semicolons and new lines (commas may be part of a phrase).
pub fn split_phrases(text: &str) -> Vec<String> {
    text.split([';', '\n'])
        .map(|s| s.trim().trim_matches('"').trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Split a list field on commas, semicolons and new lines.
pub fn split_list(text: &str) -> Vec<String> {
    text.split([',', ';', '\n'])
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

fn group(parts: &[String], op: &str) -> Option<String> {
    match parts.len() {
        0 => None,
        1 => Some(parts[0].clone()),
        _ => Some(format!("({})", parts.join(&format!(" {op} ")))),
    }
}

fn scoped(prop: Option<&str>, value: &str) -> String {
    match prop {
        Some(p) => format!("{p}:{value}"),
        None => value.to_owned(),
    }
}

fn prop_any(prop: &str, values: &[String], cx: &mut Ctx) -> Option<String> {
    let parts: Vec<String> = values
        .iter()
        .filter_map(|v| term(v, cx))
        .map(|v| format!("{prop}:{v}"))
        .collect();
    group(&parts, "OR")
}

fn terms(text: &str, prop: Option<&str>, cx: &mut Ctx) -> Vec<String> {
    tokenize(text)
        .iter()
        .filter_map(|t| term(t, cx))
        .map(|t| scoped(prop, &t))
        .collect()
}

fn date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn number(n: f64) -> String {
    format!("{}", n.max(0.0).round() as i64)
}

/// Rough check that a hand-written KeyQL fragment is well formed.
fn check_raw(raw: &str, label: &str, cx: &mut Ctx) {
    if raw.chars().filter(|&c| c == '"').count() % 2 != 0 {
        cx.push(Level::Error, format!("{label}: unbalanced double quote."));
    }
    let mut depth = 0i32;
    let mut in_q = false;
    for c in raw.chars() {
        match c {
            '"' => in_q = !in_q,
            '(' if !in_q => depth += 1,
            ')' if !in_q => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        cx.push(Level::Error, format!("{label}: unbalanced parentheses."));
    }
    for w in raw.split_whitespace() {
        if matches!(w.to_ascii_lowercase().as_str(), "and" | "or" | "not")
            && w != w.to_ascii_uppercase()
        {
            cx.push(
                Level::Warning,
                format!("{label}: '{w}' is not upper case, so Purview searches for it as a word instead of using it as an operator."),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

impl QuerySpec {
    /// Build using today's local date for relative calculations.
    pub fn build(&self) -> Built {
        self.build_at(chrono::Local::now().date_naive())
    }

    /// Build with an explicit "today" (for tests and reproducibility).
    pub fn build_at(&self, today: NaiveDate) -> Built {
        let mut notices = Vec::new();
        let mut cx = Ctx {
            notices: &mut notices,
        };
        let mut include: Vec<String> = Vec::new();
        let mut exclude: Vec<String> = Vec::new();

        self.build_keywords(&mut include, &mut exclude, &mut cx);
        self.build_proximity(&mut include, &mut cx);

        let kinds: Vec<String> = self.kinds.iter().map(|k| format!("kind:{k}")).collect();
        include.extend(group(&kinds, "OR"));

        self.build_people(&mut include, &mut exclude, &mut cx);
        self.build_dates(&mut include, today, &mut cx);
        self.build_email(&mut include, &mut cx);
        self.build_docs(&mut include, &mut cx);
        self.build_size(&mut include, &mut cx);
        self.build_sensitive(&mut include, &mut cx);
        self.build_advanced(&mut include, &mut cx);

        let raw = self.raw.trim();
        if !raw.is_empty() {
            check_raw(raw, "Custom KeyQL", &mut cx);
            include.push(format!("({raw})"));
        }

        let negative = group(&exclude, "OR");
        let query = match (include.len(), negative) {
            (0, None) => String::new(),
            (_, None) => include.join(" AND "),
            (0, Some(n)) => {
                cx.push(
                    Level::Error,
                    "The query only excludes things. Purview needs at least one positive condition before NOT (for example an item type or a date range).",
                );
                format!("NOT {n}")
            }
            (1, Some(n)) => format!("{} NOT {n}", include[0]),
            (_, Some(n)) => format!("({}) NOT {n}", include.join(" AND ")),
        };

        if query.is_empty() {
            cx.push(
                Level::Info,
                "Fill in any section on the left to start building a query.",
            );
        } else if query.len() > LENGTH_WARNING {
            cx.push(
                Level::Warning,
                format!(
                    "The query is {} characters long; Purview may reject very long queries. Consider splitting it into several searches.",
                    query.len()
                ),
            );
        }

        notices.sort_by_key(|n| n.level);
        Built { query, notices }
    }

    fn build_keywords(&self, include: &mut Vec<String>, exclude: &mut Vec<String>, cx: &mut Ctx) {
        let k = &self.keywords;
        let prop = k.scope.property();

        include.extend(terms(&k.all_words, prop, cx));
        for phrase in split_phrases(&k.phrases) {
            if let Some(t) = term(&format!("\"{phrase}\""), cx) {
                include.push(scoped(prop, &t));
            }
        }
        include.extend(group(&terms(&k.any_words, prop, cx), "OR"));
        exclude.extend(terms(&k.none_words, prop, cx));
    }

    fn build_proximity(&self, include: &mut Vec<String>, cx: &mut Ctx) {
        for p in &self.proximity {
            match (term(&p.first, cx), term(&p.second, cx)) {
                (Some(a), Some(b)) => {
                    let op = if p.ordered { "ONEAR" } else { "NEAR" };
                    include.push(format!("({a} {op}({}) {b})", p.distance.max(1)));
                }
                (None, None) => {}
                _ => cx.push(
                    Level::Warning,
                    "A proximity search needs both words; it was skipped.",
                ),
            }
        }
    }

    fn build_people(&self, include: &mut Vec<String>, exclude: &mut Vec<String>, cx: &mut Ctx) {
        for p in &self.people {
            if let Some(expr) = prop_any(p.field.property(), &split_list(&p.values), cx) {
                if p.exclude {
                    exclude.push(expr)
                } else {
                    include.push(expr)
                }
            }
        }
    }

    fn build_dates(&self, include: &mut Vec<String>, today: NaiveDate, cx: &mut Ctx) {
        for d in &self.dates {
            let (start, end) = if d.op == DateOp::Between && d.start > d.end {
                cx.push(
                    Level::Info,
                    "A date range had its start after its end; the dates were swapped.",
                );
                (d.end, d.start)
            } else {
                (d.start, d.end)
            };
            let restriction = match d.op {
                DateOp::Between => format!(":{}..{}", date(start), date(end)),
                DateOp::On => format!(":{}", date(start)),
                DateOp::Before => format!("<{}", date(start)),
                DateOp::OnOrBefore => format!("<={}", date(start)),
                DateOp::After => format!(">{}", date(start)),
                DateOp::OnOrAfter => format!(">={}", date(start)),
                DateOp::LastNDays => {
                    cx.push(
                        Level::Info,
                        "\"Last N days\" is turned into a fixed start date. Rebuild the query if you reuse it on a later day.",
                    );
                    format!(">={}", date(today - Duration::days(i64::from(d.days))))
                }
                DateOp::Relative => format!(":{}", d.relative.keyword()),
            };
            let parts: Vec<String> = d
                .field
                .properties()
                .iter()
                .map(|p| format!("{p}{restriction}"))
                .collect();
            include.extend(group(&parts, "OR"));
        }
    }

    fn build_email(&self, include: &mut Vec<String>, cx: &mut Ctx) {
        let e = &self.email;
        match e.has_attachment {
            Tri::Any => {}
            Tri::Yes => include.push("HasAttachment:true".into()),
            Tri::No => include.push("HasAttachment:false".into()),
        }
        match e.importance {
            Importance::Any => {}
            Importance::High => include.push("importance:high".into()),
            Importance::Normal => include.push("importance:medium".into()),
            Importance::Low => include.push("importance:low".into()),
        }
        match e.is_read {
            Tri::Any => {}
            Tri::Yes => include.push("IsRead:true".into()),
            Tri::No => include.push("IsRead:false".into()),
        }
        include.extend(prop_any(
            "AttachmentNames",
            &split_list(&e.attachment_names),
            cx,
        ));
        include.extend(prop_any("category", &split_list(&e.categories), cx));
    }

    fn build_docs(&self, include: &mut Vec<String>, cx: &mut Ctx) {
        let d = &self.docs;
        let mut exts: Vec<String> = d.extensions.clone();
        for e in split_list(&d.custom_extensions.replace(' ', ",")) {
            let e = e
                .trim_start_matches("*.")
                .trim_start_matches('.')
                .to_lowercase();
            if !e.is_empty() && !exts.contains(&e) {
                exts.push(e);
            }
        }
        include.extend(prop_any("FileExtension", &exts, cx));
        include.extend(prop_any("FileName", &split_list(&d.file_names), cx));
        include.extend(prop_any("path", &split_list(&d.paths), cx));
    }

    fn build_size(&self, include: &mut Vec<String>, cx: &mut Ctx) {
        let s = &self.size;
        let f = s.unit.factor();
        let (a, b) = (s.a * f, s.b * f);
        match s.op {
            SizeOp::Any => {}
            SizeOp::LessThan => include.push(format!("size<{}", number(a))),
            SizeOp::GreaterThan => include.push(format!("size>{}", number(a))),
            SizeOp::Between => {
                if a > b {
                    cx.push(
                        Level::Info,
                        "The size range was reversed; the values were swapped.",
                    );
                }
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                include.push(format!("size:{}..{}", number(lo), number(hi)));
            }
        }
    }

    fn build_sensitive(&self, include: &mut Vec<String>, cx: &mut Ctx) {
        let mut parts = Vec::new();
        for s in &self.sensitive {
            let name = s.name.trim().replace('"', "");
            if name.is_empty() {
                continue;
            }
            let value = if s.min_count <= 1 && s.confidence == Confidence::Any {
                name
            } else {
                let conf = s.confidence.min().unwrap_or(1);
                format!("{name}|{}..|{conf}..100", s.min_count.max(1))
            };
            parts.push(format!("SensitiveType:\"{value}\""));
        }
        if !parts.is_empty() {
            cx.push(
                Level::Info,
                "SensitiveType only matches items that Purview classified when it indexed them (mainly Exchange email and SharePoint/OneDrive files).",
            );
        }
        include.extend(group(&parts, "OR"));
    }

    fn build_advanced(&self, include: &mut Vec<String>, cx: &mut Ctx) {
        let mut expr: Option<String> = None;
        for c in &self.advanced {
            let prop = c.property.trim();
            if prop.is_empty() {
                continue;
            }
            if prop
                .chars()
                .any(|ch| ch.is_whitespace() || "():\"<>=".contains(ch))
            {
                cx.push(
                    Level::Error,
                    format!("Property name '{prop}' contains spaces or symbols; it was skipped."),
                );
                continue;
            }
            let Some(v) = term(&c.value, cx) else {
                continue;
            };
            let restriction = if c.op == CondOp::Range {
                let Some(v2) = term(&c.value2, cx) else {
                    cx.push(
                        Level::Warning,
                        format!("The range on '{prop}' needs two values; it was skipped."),
                    );
                    continue;
                };
                format!("{prop}:{v}..{v2}")
            } else {
                format!("{prop}{}{v}", c.op.token())
            };
            expr = Some(match expr {
                None => {
                    if c.join == Join::AndNot {
                        cx.push(
                            Level::Warning,
                            "The first advanced condition can't start with AND NOT; it was treated as AND.",
                        );
                    }
                    restriction
                }
                Some(prev) => {
                    // Wrap what came before so the conditions read strictly top to bottom.
                    let prev = if prev.contains(' ') {
                        format!("({prev})")
                    } else {
                        prev
                    };
                    match c.join {
                        Join::And => format!("{prev} AND {restriction}"),
                        Join::Or => format!("{prev} OR {restriction}"),
                        Join::AndNot => format!("{prev} NOT {restriction}"),
                    }
                }
            });
        }
        if let Some(e) = expr {
            include.push(if e.contains(' ') { format!("({e})") } else { e });
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

    fn build(spec: &QuerySpec) -> Built {
        spec.build_at(d(2026, 10, 23))
    }

    #[test]
    fn empty_spec_gives_empty_query() {
        let b = build(&QuerySpec::default());
        assert_eq!(b.query, "");
        assert!(b.notices.iter().all(|n| n.level == Level::Info));
    }

    #[test]
    fn tokenize_keeps_phrases() {
        assert_eq!(
            tokenize(r#"budget "fiscal year" 2026"#),
            vec!["budget", "\"fiscal year\"", "2026"]
        );
        assert_eq!(tokenize(r#"open "quote"#), vec!["open", "\"quote\""]);
        assert_eq!(tokenize(r#"a "" b"#), vec!["a", "b"]);
    }

    #[test]
    fn keywords_all_any_none() {
        let mut s = QuerySpec::default();
        s.keywords.all_words = "budget audit".into();
        s.keywords.phrases = "fiscal year; \"county board\"".into();
        s.keywords.any_words = "draft final".into();
        s.keywords.none_words = "newsletter spam".into();
        assert_eq!(
            build(&s).query,
            r#"(budget AND audit AND "fiscal year" AND "county board" AND (draft OR final)) NOT (newsletter OR spam)"#
        );
    }

    #[test]
    fn keyword_scope_and_reserved_words() {
        let mut s = QuerySpec::default();
        s.keywords.all_words = "NEAR merger".into();
        s.keywords.scope = KeywordScope::Subject;
        assert_eq!(build(&s).query, r#"subject:"NEAR" AND subject:merger"#);
    }

    #[test]
    fn prefix_wildcard_warns() {
        let mut s = QuerySpec::default();
        s.keywords.all_words = "*port budg*".into();
        let b = build(&s);
        assert_eq!(b.query, "*port AND budg*");
        assert!(
            b.notices
                .iter()
                .any(|n| n.level == Level::Warning && n.message.contains("suffix"))
        );
    }

    #[test]
    fn proximity() {
        let mut s = QuerySpec::default();
        s.proximity.push(Proximity {
            first: "contract".into(),
            second: "breach".into(),
            distance: 5,
            ordered: false,
        });
        s.proximity.push(Proximity {
            first: "jury".into(),
            second: "verdict".into(),
            distance: 3,
            ordered: true,
        });
        assert_eq!(
            build(&s).query,
            "(contract NEAR(5) breach) AND (jury ONEAR(3) verdict)"
        );
    }

    #[test]
    fn kinds_and_people() {
        let mut s = QuerySpec::default();
        s.kinds = vec!["email".into(), "microsoftteams".into()];
        s.people.push(PersonCond {
            field: PersonField::From,
            values: "alice@contoso.com; Bob Smith".into(),
            exclude: false,
        });
        s.people.push(PersonCond {
            field: PersonField::Participants,
            values: "noreply@contoso.com".into(),
            exclude: true,
        });
        assert_eq!(
            build(&s).query,
            r#"((kind:email OR kind:microsoftteams) AND (from:alice@contoso.com OR from:"Bob Smith")) NOT participants:noreply@contoso.com"#
        );
    }

    #[test]
    fn dates() {
        let mut s = QuerySpec::default();
        s.dates.push(DateCond {
            field: DateField::Sent,
            op: DateOp::Between,
            start: d(2026, 3, 31),
            end: d(2026, 1, 1),
            ..DateCond::new(d(2026, 10, 23))
        });
        let b = build(&s);
        assert_eq!(b.query, "sent:2026-01-01..2026-03-31");
        assert!(b.notices.iter().any(|n| n.message.contains("swapped")));

        s.dates[0].field = DateField::AnyItemDate;
        s.dates[0].op = DateOp::LastNDays;
        s.dates[0].days = 7;
        assert_eq!(
            build(&s).query,
            "(sent>=2026-10-16 OR received>=2026-10-16 OR LastModifiedTime>=2026-10-16)"
        );

        s.dates[0].field = DateField::Created;
        s.dates[0].op = DateOp::Relative;
        s.dates[0].relative = RelativeDate::LastMonth;
        assert_eq!(build(&s).query, r#"Created:"last month""#);
    }

    #[test]
    fn email_docs_size() {
        let mut s = QuerySpec::default();
        s.email.has_attachment = Tri::Yes;
        s.email.importance = Importance::Normal;
        s.docs.extensions = vec!["pdf".into()];
        s.docs.custom_extensions = ".DWG, *.xlsm pdf".into();
        s.size = SizeCond {
            op: SizeOp::GreaterThan,
            a: 2.0,
            b: 0.0,
            unit: SizeUnit::MB,
        };
        assert_eq!(
            build(&s).query,
            "HasAttachment:true AND importance:medium AND (FileExtension:pdf OR FileExtension:dwg OR FileExtension:xlsm) AND size>2097152"
        );
    }

    #[test]
    fn sensitive_types() {
        let mut s = QuerySpec::default();
        s.sensitive.push(SensitiveCond {
            name: "Credit Card Number".into(),
            min_count: 1,
            confidence: Confidence::Any,
        });
        s.sensitive.push(SensitiveCond {
            name: "U.S. Social Security Number (SSN)".into(),
            min_count: 5,
            confidence: Confidence::High,
        });
        assert_eq!(
            build(&s).query,
            r#"(SensitiveType:"Credit Card Number" OR SensitiveType:"U.S. Social Security Number (SSN)|5..|85..100")"#
        );
    }

    #[test]
    fn advanced_conditions_fold_top_to_bottom() {
        let mut s = QuerySpec::default();
        s.advanced.push(Condition {
            property: "subject".into(),
            value: "invoice".into(),
            ..Default::default()
        });
        s.advanced.push(Condition {
            join: Join::Or,
            property: "AttachmentNames".into(),
            value: "invoice*".into(),
            ..Default::default()
        });
        s.advanced.push(Condition {
            join: Join::AndNot,
            property: "from".into(),
            value: "billing@contoso.com".into(),
            ..Default::default()
        });
        s.advanced.push(Condition {
            join: Join::And,
            property: "size".into(),
            op: CondOp::Range,
            value: "1000".into(),
            value2: "5000".into(),
        });
        assert_eq!(
            build(&s).query,
            "(((subject:invoice OR AttachmentNames:invoice*) NOT from:billing@contoso.com) AND size:1000..5000)"
        );
    }

    #[test]
    fn exclusions_only_is_an_error() {
        let mut s = QuerySpec::default();
        s.keywords.none_words = "spam".into();
        let b = build(&s);
        assert_eq!(b.query, "NOT spam");
        assert_eq!(b.notices[0].level, Level::Error);
    }

    #[test]
    fn raw_is_checked() {
        let mut s = QuerySpec::default();
        s.kinds = vec!["email".into()];
        s.raw = "(budget or \"audit".into();
        let b = build(&s);
        assert_eq!(b.query, "kind:email AND ((budget or \"audit)");
        assert_eq!(
            b.notices.iter().filter(|n| n.level == Level::Error).count(),
            2
        );
        assert!(b.notices.iter().any(|n| n.message.contains("upper case")));
    }

    #[test]
    fn example_is_clean() {
        let b = build(&QuerySpec::example(d(2026, 10, 23)));
        assert_eq!(
            b.query,
            "(contract AND \"purchase agreement\" AND (draft OR revised OR final) AND (payment NEAR(5) late) \
             AND (kind:email OR kind:docs) AND (participants:jane.doe@contoso.com OR participants:fabrikam.com) \
             AND (sent:2026-01-01..2026-10-23 OR received:2026-01-01..2026-10-23 OR LastModifiedTime:2026-01-01..2026-10-23) \
             AND HasAttachment:true AND (FileExtension:docx OR FileExtension:pdf)) NOT newsletter"
        );
        assert!(b.notices.iter().all(|n| n.level == Level::Info));
    }

    #[test]
    fn embedded_quotes_are_stripped() {
        let mut s = QuerySpec::default();
        s.email.attachment_names = r#"my "final" report.docx"#.into();
        let b = build(&s);
        assert_eq!(b.query, r#"AttachmentNames:"my final report.docx""#);
        assert!(b.notices.iter().any(|n| n.level == Level::Warning));
    }
}
