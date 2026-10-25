<div align="center">

# eDiscovery Search Request

**Ask for a Microsoft Purview eDiscovery search without knowing any search syntax.**

Staff answer a few plain-language questions. The app writes the KeyQL query for them and saves it as a
`.kql` file to email to whoever runs Purview searches.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Rust 2024](https://img.shields.io/badge/rust-2024%20edition-orange.svg)
![Windows | macOS | Linux | Web](https://img.shields.io/badge/runs%20on-Windows%20%7C%20macOS%20%7C%20Linux%20%7C%20browser-informational.svg)
![No uploads](https://img.shields.io/badge/data-never%20leaves%20your%20device-success.svg)

<img src="docs/screenshots/step-words.png" alt="Step 3 of the wizard: words and phrases as chips, any/all choice, and where the words should appear" width="860">

</div>

---

## Contents

- [Why](#why)
- [What it does](#what-it-does)
- [A tour](#a-tour)
- [Privacy: nothing leaves your device](#privacy-nothing-leaves-your-device)
- [The file it makes](#the-file-it-makes)
- [How answers become KeyQL](#how-answers-become-keyql)
- [Checks](#checks)
- [Design basis](#design-basis)
- [Getting started](#getting-started)
- [Hosting the web version](#hosting-the-web-version)
- [Project layout](#project-layout)
- [License](#license)

## Why

Purview's search box expects KeyQL: `participants:`, `kind:`, date ranges, upper-case operators,
quoted phrases. People who need a search (a records officer answering a data-practices request, HR,
a department head) usually don't know it. The person who does know it ends up translating emails like
"everything Jane sent about the Highway 56 contract this year, but not the newsletters" by hand.

This app does that translation. Staff never see the query. They see a plain-English summary of
what they asked for, and whoever runs the search gets an exact, repeatable query.

## What it does

- **A seven-step wizard** with one question per page and a step list you can jump around in.
- **Only one required answer** (a name for the request). Everything else is optional and explained in plain words.
- **A check-your-answers page** that summarizes the request in plain English, with **Change** links.
- **Problems are explained, not just flagged.** Each problem has a link that takes you to the step to fix it.
- **Saves a `.kql` file** named after the request and the date, ready to attach to an email.
- **Writes the email for you.** It opens your mail program with the To, Subject and body filled in, or copies the text.
- **Remembers your answers** if you close the app, so a half-finished request isn't lost.
- **Text size and color settings:** Standard, Large or Extra large text, and light, dark or match-my-computer colors.
- **Runs anywhere:** a native app for Windows, macOS and Linux, and the same app in a browser tab via WebAssembly.

## A tour

| | |
|---|---|
| <img src="docs/screenshots/step-about.png" alt="Step 1: About this request" width="420"> | <img src="docs/screenshots/step-whose.png" alt="Step 2: Whose content" width="420"> |
| **1. About this request.** Name the request. Optionally give a reason and a deadline. | **2. Whose content.** Work email addresses of the people to search, or shared mailboxes and sites, or "Search everyone". |
| <img src="docs/screenshots/step-words.png" alt="Step 3: Words to look for" width="420"> | <img src="docs/screenshots/step-dates.png" alt="Step 4: Dates" width="420"> |
| **3. Words to look for.** Words and phrases as chips, "any" or "all", where they should appear, words to leave out, and words close together. | **4. Dates.** Any time, Last 30 days, Last 12 months, This year, Last year, or your own dates (typed as MM/DD/YYYY or picked from a calendar). |
| <img src="docs/screenshots/step-types.png" alt="Step 5: Types of items" width="420"> | <img src="docs/screenshots/step-narrow.png" alt="Step 6: Narrow it down" width="420"> |
| **5. Types of items.** Email, Teams chats and meetings, or files in OneDrive and SharePoint. More types are listed below. | **6. Narrow it down.** Sender and recipients, attachments, file types and names, authors, importance, size, sensitive information. |
| <img src="docs/screenshots/review.png" alt="Step 7: Check your request" width="420"> | <img src="docs/screenshots/review-problems.png" alt="Check page with problems to fix" width="420"> |
| **7. Check and save.** A plain-English summary of the whole request, with Change links. | **Problems block saving until they're fixed**, and warnings ask for a second look. Every one links to its step. |
| <img src="docs/screenshots/saved.png" alt="Saved: what to do next" width="420"> | <img src="docs/screenshots/step-words-dark.png" alt="Dark colors" width="420"> |
| **Saved.** What to do next, plus **Write the email** and **Copy email text**. | **Dark colors**, or match the computer's setting. |

## Privacy: nothing leaves your device

eDiscovery requests are sensitive: who is being searched, and for what. The app is built so that none of it
goes anywhere:

- **No networking code.** The app makes no network requests at all. In the browser it downloads its own files once and that's it.
- **Locked down in the browser.** The page's Content-Security-Policy only allows it to load its own files (`connect-src 'self'`), so even a future change couldn't quietly send data elsewhere.
- **The file is made on your device.** On the desktop it's written where you choose. In the browser it's created in the tab and downloaded.
- **Answers stay local.** They're saved in the app's own data folder on the desktop, or in that one browser's local storage on the web.
- **Email stays yours.** "Write the email" only opens a `mailto:` link in your own mail program, and you attach the file yourself.

The web version shows this while it loads:

<p align="center">
  <img src="docs/screenshots/loading-light.png" alt="Loading screen with download progress and the privacy promises" width="420">
  <img src="docs/screenshots/loading-dark.png" alt="The same loading screen in dark colors" width="420">
</p>

## The file it makes

The sample request (`--example` on the desktop, `?example` in the browser) produces
[`docs/example.kql`](docs/example.kql):

```text
("Highway 56" AND contract AND (sent:2026-01-01..2026-10-25 OR received:2026-01-01..2026-10-25 OR LastModifiedTime:2026-01-01..2026-10-25) AND (kind:email OR kind:docs) AND HasAttachment:true) NOT newsletter
```

The person running the search pastes it into **Purview → eDiscovery → (case) → Search → Query → KeyQL editor**.

The people and places to search go in the email, not in the file. Purview picks them separately from the
query (as data sources), and KeyQL has no comments to carry them.

## How answers become KeyQL

[`src/query.rs`](src/query.rs) turns the answers into KeyQL, following Microsoft Learn's
*Keyword queries and search conditions for eDiscovery*:

| You answer… | The query gets… |
|---|---|
| Words and phrases, "all of these" | `"Highway 56" AND contract` (phrases quoted) |
| Words and phrases, "any of these" | `(budget OR "fiscal year")` |
| Only in the subject line or file title | `SubjectTitle:budget` on each word |
| Words close together | `(contract NEAR(5) breach)` |
| Leave out | one trailing `NOT (newsletter OR digest)` |
| Dates: "This year" | `sent:2026-01-01..2026-10-25 OR received:… OR LastModifiedTime:…` |
| Email / Teams / Files | `kind:email`, `kind:microsoftteams`, `kind:im`, `kind:docs`, `kind:meetings`… |
| From / to / involving | `from:`, `recipients:`, `participants:` |
| Attachments, file types, names | `HasAttachment:true`, `FileExtension:dwg`, `FileName:…` |
| Authors, importance, size | `author:`, `importance:high`, `size>2097152` (bytes) |
| Sensitive information | `SensitiveType:"U.S. Social Security Number (SSN)"` |

Rules it always follows:

- Operators are upper case (`AND`, `OR`, `NOT`, `NEAR(n)`).
- Several values in one condition are joined with `OR`, and conditions are joined with `AND`.
- Dates are **absolute**. "Last 30 days" is fixed on the day the file is saved, so the same search can be run again later and give the same results.

## Checks

Saving is blocked when:

- the request has no name
- there's nothing to search for, or there are only things to leave out
- a date can't be read

You're warned (but can still save) about:

- names where an email address is expected
- a `*` at the start of a word (Purview only supports it at the end)
- dates in the future
- sensitive-information filters on email (they only work on OneDrive and SharePoint files)
- a query over Purview's 4,000-character limit

## Design basis

The layout follows published form-usability guidance:

- **GOV.UK Design System:** one thing per page, check answers, error summary and error messages
- **NN/g:** wizards, required fields, placeholders, toggles vs. checkboxes
- **USWDS:** date inputs and date ranges
- **WCAG 2.2:** contrast, not relying on color alone, keyboard use

In practice:

- Labels and hints sit above the fields, and examples stay visible.
- Optional sections stay collapsed until needed.
- Problems appear once you've moved on from a step, both on the step and in a summary on the check page.
- Every problem has an icon and words, so color is never the only signal.
- Hint text has at least 7:1 contrast and field borders at least 3:1, in both color schemes.
- Anything still typed in a box is kept when you change page.

## Getting started

You need a Rust toolchain: <https://rustup.rs>.

### Desktop app

```sh
git clone https://github.com/simontemplarST/keyql-builder.git
cd keyql-builder
cargo run --release              # start the app
cargo run --release -- --example # start on the check page with a sample request
cargo test                       # query generator and UI helper tests
```

The release binary is in `target/release/` (`keyql-builder.exe` on Windows, which opens no console window).
Answers are saved in `%APPDATA%\keyql-builder\` on Windows, `~/Library/Application Support/keyql-builder/`
on macOS and `~/.local/share/keyql-builder/` on Linux.

### In a browser

The same code builds to WebAssembly with [trunk](https://trunkrs.dev):

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk

trunk serve --release            # http://127.0.0.1:8080   (add ?example for the sample request)
trunk build --release            # static files in dist/
```

In the browser, **Save request file…** downloads the `.kql` file, and **Show the file in its folder** is hidden.

> The post-build hook in [`Trunk.toml`](Trunk.toml) fixes the size the loading bar counts to (trunk records the
> size from before `wasm-opt`). It uses `sh`, `wc` and GNU `sed`; on macOS install `gsed` or drop the hook (the bar
> then stops short of 100% before the app starts).

## Hosting the web version

`dist/` is a plain static site: `index.html`, one `.js`, one `.mjs` (the loader) and one `.wasm`.
Any static web server works, with two things to get right:

1. **MIME types.** `.wasm` must be `application/wasm`, and `.mjs` must be a JavaScript type. nginx's default `mime.types` has no `.mjs`.
2. **Compression.** The `.wasm` is about 8 MB, or about 3 MB with gzip or zstd. Turn compression on, here or in a proxy in front.

A minimal nginx `server` block:

```nginx
server {
    listen 80;
    root /usr/share/nginx/html;          # contents of dist/

    gzip on;
    gzip_types application/wasm text/javascript application/javascript;

    # mime.types has no .mjs; known types still win, so this only affects the loader.
    location ~* "-[0-9a-f]{12,16}(_bg)?\.(js|mjs|wasm)$" {
        default_type text/javascript;
        add_header Cache-Control "public, max-age=31536000, immutable";   # file names carry a content hash
    }
    location / {
        add_header Cache-Control "no-cache";
        try_files $uri /index.html;
    }
}
```

`index.html` already carries the Content-Security-Policy in a `<meta>` tag. Sending the same policy as a
header as well is a good idea.

## Project layout

```text
src/query.rs     request model, KeyQL generator, plain-English summary, email text, checks (pure, unit tested)
src/app.rs       the wizard UI: steps, chip and date inputs, saving, settings
src/main.rs      desktop window setup, and the browser entry point
index.html       web page: loading screen and Content-Security-Policy
loader.mjs       download progress for the loading screen (trunk initializer)
Trunk.toml       web build settings and the post-build hook
docs/            screenshots and the sample .kql file
```

Built with [egui / eframe](https://github.com/emilk/egui), [chrono](https://github.com/chronotope/chrono),
[rfd](https://github.com/PolyMeilex/rfd) for the desktop save dialog and [trunk](https://trunkrs.dev) for the web build.

## License

MIT. See [LICENSE](LICENSE).

*Not affiliated with Microsoft. Microsoft Purview is a trademark of Microsoft Corporation.*
