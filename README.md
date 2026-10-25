# eDiscovery Search Request

A desktop app that helps staff ask for a Microsoft Purview eDiscovery search without
knowing any search syntax. They answer a few plain-language questions, and the app
saves a **`.kql` file** to email to the IT Director, who pastes it into
Purview → eDiscovery → Search → Query → KeyQL editor.

Staff never see the query. They see a plain-English summary of what they asked for.

Built in Rust with [egui/eframe](https://github.com/emilk/egui). It runs on Windows,
macOS and Linux.

## How it works

The app is a seven-step wizard with a step list on the left. Everything except the
request name is optional, and you can jump to any step at any time.

| Step | What staff do |
|---|---|
| 1. About this request | Name the request (required). Optionally add a reason and a deadline. |
| 2. Whose content | List the people (work email addresses) or shared mailboxes and sites to search, or tick "Search everyone". |
| 3. Words to look for | Add words or phrases as chips, then choose "any of these" or "all of these" and where they should appear. Optional: words to leave out, and words close together. |
| 4. Dates | Pick Any time, Last 30 days, Last 12 months, This year, Last year, or custom dates. Dates can be typed (MM/DD/YYYY) or picked from a calendar. |
| 5. Types of items | Tick cards for Email, Teams chats and meetings, or Files in OneDrive and SharePoint. More types are listed below. |
| 6. Narrow it down | Optional filters: sender and recipients, attachments, file types and names, authors, importance, size, and sensitive information. |
| 7. Check and save | Read a plain-English summary with **Change** links, see any problems with links to fix them, then **Save request file…**. |

After saving, a confirmation page says what to do next and offers three buttons:

- **Write the email** opens your mail program with the To, Subject and body filled in. You still attach the file yourself.
- **Copy email text** copies the request details: name, reason, deadline, whose content to search, and the summary.
- **Show the file in its folder** opens the folder the file was saved to.

The people and places to search go in the email, not the file. Purview chooses them
separately from the query, and KeyQL has no comments.

**Settings:** the IT Director's email address (used for the To line), text size
(Standard, Large or Extra large) and colors (match my computer, light or dark).

Answers are saved automatically and are still there when the app is reopened.

## Design basis

The layout follows published form-usability guidance:

- **NN/g** on wizards, required fields, placeholders and toggles vs. checkboxes
- **GOV.UK Design System**: one thing per page, check answers, error summary and error messages
- **USWDS**: date inputs and date ranges
- **WCAG 2.2**: contrast, not relying on color alone, and keyboard use

In practice that means:

- Labels and hints sit above the fields, and examples stay visible.
- Optional sections stay collapsed until needed.
- Problems show up once you've moved on from a step, both on the step itself and in a summary on the review page.
- Every problem has an icon and text, so color is never the only signal.
- Hint text has at least 7:1 contrast and field borders at least 3:1, in both themes.
- Anything still typed in a box is kept when you change page.

## The query

`src/query.rs` turns the answers into KeyQL. It follows the syntax on Microsoft Learn
("Keyword queries and search conditions for eDiscovery"):

- Operators are always written in upper case (`AND`, `OR`, `NOT`, `NEAR(n)`).
- Phrases are quoted.
- Dates are absolute (`sent:2026-01-01..2026-03-31`). Choices like "Last 30 days" are fixed on the day the file is saved, so the search can be repeated exactly.
- Several values in one condition are joined with `OR`, and conditions are joined with `AND`. Everything to leave out is gathered into one trailing `NOT (…)`.
- Item types map to `kind:` (email, microsoftteams/im, docs, meetings…). People map to `from:`, `recipients:` and `participants:`. Filters map to `HasAttachment:`, `FileExtension:`, `FileName:`, `author:`, `importance:`, `size>` / `size<` and `SensitiveType:"…"`.

The checks stop saving when there's nothing to search for, when only exclusions were
given, when the request has no name, or when a date can't be read. They warn about:

- names instead of email addresses
- a `*` at the start of a word
- dates in the future
- sensitive-information filters on email (they only work on OneDrive and SharePoint)
- queries over Purview's 4,000-character limit

## Build and run

Requires a Rust toolchain (<https://rustup.rs>).

```sh
cargo run --release              # start the app
cargo run --release -- --example # start on the review page with a sample request
cargo test                       # query generator and UI helper tests
```

### In a web browser

The same app also builds to WebAssembly and runs in a browser tab. Install the wasm
target and [trunk](https://trunkrs.dev) once, then serve it locally:

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
trunk serve --release            # http://127.0.0.1:8080  (add ?example for the sample request)
trunk build --release            # static files in dist/, servable from any web server
```

In the browser, **Save request file…** downloads the `.kql` file to the browser's
Downloads folder, and **Show the file in its folder** is hidden. Answers and settings
are kept in the browser's local storage.

On Windows the release build opens no console window. Saved answers live in
`~/.local/share/keyql-builder/` on Linux and `%APPDATA%\keyql-builder\` on Windows.

## Layout

- `src/query.rs` holds the request model, the KeyQL generator, the plain-English summary, the email text and the checks. It is pure and unit tested.
- `src/app.rs` holds the wizard UI, the chip and date inputs, saving and settings.
- `src/main.rs` sets up the window.
