# Purview KeyQL Builder

A desktop form for building Microsoft Purview eDiscovery **KeyQL** queries without
memorising the syntax. You fill in text boxes, check boxes, dropdowns and date
pickers, and the app writes the query live, ready to paste into
Purview → eDiscovery → Search → Query → KeyQL editor.

Built in Rust with [egui/eframe](https://github.com/emilk/egui).

## What you can build

| Section | Controls | Example output |
|---|---|---|
| Keywords | all / exact phrases / any / none of these words, plus where to look (anywhere, subject/title, subject, title) | `budget AND "fiscal year" AND (draft OR final) … NOT newsletter` |
| Words near each other | two words, distance, "in this order" | `(payment NEAR(5) late)`, `(jury ONEAR(3) verdict)` |
| Item types | check boxes for every `kind:` value | `(kind:email OR kind:microsoftteams)` |
| People | from / to / cc / bcc / participants / recipients / author / modified by; include or exclude | `(from:jane@contoso.com OR from:"John Smith")` |
| Dates | sent, received, created, modified or any item date; between / on / before / after / last N days / relative (this week, last month…) with calendar pickers | `sent:2026-01-01..2026-03-31`, `Created:"last month"` |
| Email details | has attachments, importance, read state, attachment names, categories | `HasAttachment:true AND importance:high` |
| Documents & files | file-type check boxes, other extensions, file names, site/folder URL | `(FileExtension:docx OR FileExtension:pdf)` |
| Size | smaller / larger / between, in bytes–GB | `size>2097152` |
| Sensitive information | built-in or custom SIT name, minimum count, confidence | `SensitiveType:"Credit Card Number\|3..\|85..100"` |
| Advanced conditions | any property × operator (`:` `=` `<>` `<` `<=` `>` `>=` `..`), joined AND / OR / AND NOT, read top to bottom | `((subject:invoice OR AttachmentNames:invoice*) NOT from:billing@contoso.com)` |
| Custom KeyQL | free text, ANDed on as-is | |

Sections are combined with `AND`; every exclusion is gathered into one trailing `NOT (…)`.

The **Checks** panel flags problems as you type: unbalanced quotes or parentheses,
lower-case `and`/`or`/`not` (Purview treats those as search words), leading
wildcards (Purview only supports suffix `*`), queries that only exclude things,
reversed ranges, and very long queries.

**Saved queries** keeps named forms between sessions, and **Example** fills in a
sample. Everything you enter is stored locally by eframe (`~/.local/share/keyql-builder/`
on Linux, `%APPDATA%\keyql-builder\` on Windows).

## Build and run

Requires a Rust toolchain (<https://rustup.rs>).

```sh
cargo run --release              # start the app
cargo run --release -- --example # start with the sample query loaded
cargo test                       # query-generator unit tests
```

Builds natively on Linux, Windows and macOS. On Windows, the release build
opens no console window.

## Layout

- `src/query.rs` holds the query model and KeyQL generator. It is pure and unit tested.
- `src/app.rs` holds the egui form, output panel and saved queries.
- `src/main.rs` sets up the window.

## Caveats

KeyQL support differs between Purview data sources. In particular, `SensitiveType`
only matches content that was classified at index time, and some properties apply
only to email or only to documents. Check results in Purview's statistics or
sample view before relying on a query.
