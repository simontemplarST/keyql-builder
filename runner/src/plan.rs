//! Turning a request into what Purview needs: names for the case and search, and the mailboxes
//! and sites to search. The person asking may have typed names ("Jane Doe") or descriptions
//! ("Highway Department shared mailbox"); those can't be searched until the admin gives the real
//! address, so they start out unresolved.

use keyql_builder::query;
use keyql_builder::reqfile::Contents;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// A mailbox (and its OneDrive), by SMTP address.
    Mailbox,
    /// A SharePoint or OneDrive site, by URL.
    Site,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceEntry {
    /// What the request said, kept for the audit log.
    pub asked: String,
    /// Person (whose content) or place (shared mailbox, Team, site).
    pub is_person: bool,
    pub kind: SourceKind,
    /// The address or URL to search; empty until the admin fills it in.
    pub value: String,
    /// Left out on purpose by the admin.
    pub skip: bool,
}

impl SourceEntry {
    pub fn problem(&self) -> Option<String> {
        if self.skip {
            return None;
        }
        let v = self.value.trim();
        match self.kind {
            _ if v.is_empty() => Some(format!(
                "\"{}\" needs an email address or site URL.",
                self.asked
            )),
            SourceKind::Mailbox if !is_email(v) => Some(format!("\"{v}\" isn't an email address.")),
            SourceKind::Site if !is_site_url(v) => Some(format!(
                "\"{v}\" isn't a site address (it should start with https://)."
            )),
            _ => None,
        }
    }
}

pub fn is_email(s: &str) -> bool {
    s.contains('@') && query::looks_like_address(s) && !s.contains(char::is_whitespace)
}

pub fn is_site_url(s: &str) -> bool {
    s.starts_with("https://") && s.len() > "https://x.y".len() && !s.contains(char::is_whitespace)
}

/// Where to search, as worked out from the request and then corrected by the admin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePlan {
    /// "Search everyone in the organization": every mailbox and site in the tenant.
    pub everyone: bool,
    pub entries: Vec<SourceEntry>,
}

impl SourcePlan {
    pub fn from_request(c: &Contents) -> Self {
        let cu = &c.request.custodians;
        let mut entries = Vec::new();
        for p in cu.people.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
            entries.push(guess(p, true));
        }
        for l in cu
            .locations
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
        {
            entries.push(guess(l, false));
        }
        Self {
            everyone: cu.everyone,
            entries,
        }
    }

    /// Problems that stop the search from running, in plain words.
    pub fn problems(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .entries
            .iter()
            .filter_map(SourceEntry::problem)
            .collect();
        if !self.everyone && self.active().next().is_none() {
            out.push(
                "Nothing to search: add a mailbox or site, or choose to search everyone.".into(),
            );
        }
        out
    }

    /// The entries that will be added to the search.
    pub fn active(&self) -> impl Iterator<Item = &SourceEntry> {
        self.entries.iter().filter(|e| !e.skip)
    }

    /// Purview's `dataSourceScopes` for the search.
    pub fn scopes(&self) -> Option<&'static str> {
        self.everyone.then_some("allTenantMailboxes,allTenantSites")
    }
}

fn guess(asked: &str, is_person: bool) -> SourceEntry {
    let (kind, value) = if is_email(asked) {
        (SourceKind::Mailbox, asked.to_lowercase())
    } else if is_site_url(asked) {
        (SourceKind::Site, asked.to_owned())
    } else if asked.contains(".sharepoint.com/") {
        (
            SourceKind::Site,
            format!("https://{}", asked.trim_start_matches("http://")),
        )
    } else {
        (SourceKind::Mailbox, String::new())
    };
    SourceEntry {
        asked: asked.to_owned(),
        is_person,
        kind,
        value,
        skip: false,
    }
}

/// Names used in Purview, built from the request.
pub struct Names {
    /// For a new case: "DR-2026-041 Example - Highway 56 contract".
    pub case: String,
    /// The request's reference number, stored as the case's external ID.
    pub external_id: String,
    /// The search: the request name and the date it was saved, unique enough within a case.
    pub search: String,
    /// What was asked for, for the search's description.
    pub description: String,
}

/// Purview limits display names; keep well within it.
const MAX_NAME: usize = 120;

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

pub fn names(c: &Contents) -> Names {
    let name = c.name();
    let reference = c.request.about.reference.trim().to_owned();
    let case = if reference.is_empty() {
        name.clone()
    } else {
        format!("{reference} {name}")
    };
    let mut description = c.request.about.reason.trim().to_owned();
    if description.is_empty() {
        description = format!(
            "{} request saved {}",
            c.request.about.kind.label(),
            query::long_date(c.saved_on)
        );
    }
    Names {
        case: clip(&case, MAX_NAME),
        external_id: clip(&reference, 64),
        search: clip(
            &format!("{name} ({})", c.saved_on.format("%Y-%m-%d")),
            MAX_NAME,
        ),
        description: clip(&description, 1000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use keyql_builder::query::SearchRequest;

    fn contents(people: &[&str], places: &[&str], everyone: bool) -> Contents {
        let today = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        let mut r = SearchRequest::example(today);
        r.custodians.people = people.iter().map(|s| s.to_string()).collect();
        r.custodians.locations = places.iter().map(|s| s.to_string()).collect();
        r.custodians.everyone = everyone;
        Contents::new(&r, today)
    }

    #[test]
    fn sources_are_guessed_and_names_left_for_the_admin() {
        let c = contents(
            &["Jane.Doe@Example.gov", "Pat Lee"],
            &[
                "https://contoso.sharepoint.com/sites/Highway",
                "Highway Department shared mailbox",
                "roads@example.gov",
            ],
            false,
        );
        let mut p = SourcePlan::from_request(&c);
        let v: Vec<(&str, SourceKind, &str)> = p
            .entries
            .iter()
            .map(|e| (e.asked.as_str(), e.kind, e.value.as_str()))
            .collect();
        assert_eq!(
            v,
            vec![
                (
                    "Jane.Doe@Example.gov",
                    SourceKind::Mailbox,
                    "jane.doe@example.gov"
                ),
                ("Pat Lee", SourceKind::Mailbox, ""),
                (
                    "https://contoso.sharepoint.com/sites/Highway",
                    SourceKind::Site,
                    "https://contoso.sharepoint.com/sites/Highway"
                ),
                ("Highway Department shared mailbox", SourceKind::Mailbox, ""),
                (
                    "roads@example.gov",
                    SourceKind::Mailbox,
                    "roads@example.gov"
                ),
            ]
        );
        assert_eq!(p.problems().len(), 2);
        p.entries[1].value = "pat.lee@example.gov".into();
        p.entries[3].skip = true;
        assert!(p.problems().is_empty());
        assert_eq!(p.active().count(), 4);
        assert_eq!(p.scopes(), None);
        p.entries[2].kind = SourceKind::Mailbox;
        assert_eq!(p.problems().len(), 1);
    }

    #[test]
    fn everyone_and_empty() {
        let p = SourcePlan::from_request(&contents(&[], &[], true));
        assert!(p.problems().is_empty());
        assert_eq!(p.scopes(), Some("allTenantMailboxes,allTenantSites"));
        let p = SourcePlan::from_request(&contents(&[], &[], false));
        assert_eq!(p.problems().len(), 1);
    }

    #[test]
    fn purview_names() {
        let n = names(&contents(&["a@b.gov"], &[], false));
        assert_eq!(n.case, "DR-2026-041 Example - Highway 56 contract");
        assert_eq!(n.external_id, "DR-2026-041");
        assert_eq!(n.search, "Example - Highway 56 contract (2026-10-23)");
        assert!(
            n.description
                .starts_with("All emails and documents this year")
        );
        assert_eq!(clip("abcdef", 4), "abc…");
    }
}
