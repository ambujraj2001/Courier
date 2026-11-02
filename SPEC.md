# Courier — Product & Technical Specification

**Status:** Draft for approval
**Date:** 2026-11-01
**Target platform:** macOS 11+ (Apple Silicon and Intel), packaged as `Courier.app`
**Stack:** Rust (stable), `egui`/`eframe`, `reqwest`, `tokio`, local JSON files

---

## 1. Summary

Courier is a native desktop API client for building, sending, organizing, scripting and testing HTTP requests. It runs fully offline: there are no accounts, no sign-in, no telemetry and no cloud sync. Everything is stored as JSON files on the user's machine.

Courier reads and writes **Postman Collection Format v2.1** (and reads v2.0), Postman-format environment and globals files, and **cURL** commands. A team can move collections between Courier and other tools without losing data.

### 1.1 Clean-room statement

The feature set below came from studying the installed Postman app (v11.93.0) as a reference product, using `rea` (see Appendix A). Courier copies **no** source code, assets, icons, fonts, themes, copy text or branding from it. Interoperability uses only the publicly documented Collection v2.1 JSON format and the public cURL command-line syntax. Courier has its own name, icon, colour palette and UI wording.

### 1.2 Goals

1. Cover the everyday work of a Postman-style client: requests, auth, variables, environments, collections, history, scripts, tests and a collection runner.
2. Import and export Collection v2.1 and cURL losslessly for every field Courier supports. Fields it doesn't support are kept and written back unchanged.
3. Feel fast and native: start in under 1 s and never block the UI on the network or disk.
4. Test every feature automatically; commit each feature on its own.

### 1.3 Non-goals (v1)

These areas exist in the reference product and are **out of scope** for v1. Section 17 lists them as possible future work.

- Accounts, workspaces shared with others, cloud sync, comments, forking, pull requests
- WebSocket, Socket.IO, gRPC, GraphQL-over-WS, MQTT and SSE-specific clients (plain HTTP GraphQL bodies **are** in scope)
- Mock servers, monitors, API design/spec editor, documentation publishing, flows
- Performance and load testing, AI assistants, the Interceptor browser extension and capture proxy
- Importing OpenAPI, RAML, WSDL, WADL or HAR (cURL and Postman JSON only in v1)
- Code generation for languages other than cURL and raw HTTP

---

## 2. Concepts & glossary

| Term | Meaning |
|---|---|
| **Request** | One HTTP call: method, URL, params, headers, body, auth, scripts and settings. It can be saved in a collection or left unsaved in a tab. |
| **Collection** | A named tree of folders and requests, with its own auth, variables, scripts and description. One collection is one JSON file. |
| **Folder** | A node in a collection that holds requests and sub-folders. It has its own auth, scripts and description. |
| **Example** | A saved request/response pair attached to a request (v2.1 `response[]`). |
| **Environment** | A named set of variables. At most one is active at a time. |
| **Globals** | One set of variables that is always in scope. |
| **Variable** | `{{name}}` placeholder resolved at send time from the scopes in §6. |
| **Script** | JavaScript run before a request (pre-request) or after the response (post-response/tests). |
| **Runner** | Runs a collection or folder in order, over N iterations or the rows of a data file, and reports test results. |
| **History** | A log of every request sent, with a snapshot of its response. |

---

## 3. User interface & workflows

### 3.1 Window layout

```
┌──────────────────────────────────────────────────────────────────────────┐
│ [≡] Courier   [+ New] [Import]                     Env: [Staging ▾] [⚙]  │  top bar
├────┬──────────────┬──────────────────────────────────────────────────────┤
│ 📁 │ Collections  │ [GET Users ×] [POST Login ●] [+]                      │  tab strip
│ 🌐 │  ▸ Shop API  │ ┌──────────────────────────────────────────────────┐ │
│ 🕘 │    ▸ Auth    │ │ [GET ▾] {{baseUrl}}/users?page=2   [Send ▾][Save] │ │  URL bar
│    │      POST Lo │ ├──────────────────────────────────────────────────┤ │
│    │    GET Users │ │ Params  Auth  Headers(3)  Body  Scripts  Settings │ │  request tabs
│    │              │ │  …editor…                                         │ │
│    │              │ ├──────────────────────────────────────────────────┤ │
│    │              │ │ Body  Cookies  Headers(12)  Tests(4/5)            │ │  response tabs
│    │              │ │ 200 OK · 142 ms · 3.4 KB        [Pretty|Raw|Prev] │ │
│    │              │ │  …viewer…                                         │ │
├────┴──────────────┴──────────────────────────────────────────────────────┤
│ [Console ▴]  [Runner]  [Cookies]                          Layout: [⇅|⇆]  │  status bar
└──────────────────────────────────────────────────────────────────────────┘
```

- **Activity rail** (far left) switches the sidebar between **Collections**, **Environments** and **History**.
- **Sidebar** can be resized, collapsed (`⌘\`) and filtered with a search box at the top.
- **Tab strip** holds open requests, collection/folder settings, environment editors and runner results. Tabs show the method colour and a dirty dot `●`. Tabs can be reordered by dragging; middle-click closes. "Reopen closed tab" is supported.
- **Request/response split** can be vertical (stacked) or horizontal (side-by-side). The toggle and split ratio are saved.
- **Environment selector** in the top bar, with a quick-look popover listing the active environment's and globals' current values.
- **Status bar**: console toggle, runner, cookie manager, layout toggle.
- **Themes**: Light, Dark, and System (follows macOS). Courier's own palette.
- **UI zoom**: `⌘+`, `⌘-` and `⌘0`. The response font size is set separately.

### 3.2 Core workflows

1. **Quick request.** `⌘T` opens a new untitled tab, the user types a URL, and `⌘↩` sends it. The response appears and an entry is added to History. Closing a dirty tab asks Save / Don't save / Cancel.
2. **Save to collection.** `⌘S` saves the tab back to its source. `⇧⌘S` (Save As) opens a picker to choose collection, folder and name, with "+ New folder" and "+ New collection" options.
3. **Organize.** In the sidebar, the user can create, rename (`⌘E`/F2), duplicate (`⌘D`), delete (with confirmation) and drag-and-drop items between folders and collections. A right-click menu shows the same actions plus Run, Export, Copy as cURL and "Add request/folder".
4. **Environments.** Create, duplicate, rename, delete, import and export environments, and set one as active. The editor is a table with columns Enabled · Variable · Type (default/secret) · Initial value · Current value.
5. **History.** Entries are grouped by day (Today, Yesterday, date). Click to open a copy in a new tab; right-click to save to a collection, copy as cURL or delete. Includes search and "Clear all".
6. **Import.** One dialog accepts a dropped file, a file picker, or pasted text. It detects whether the input is a v2.0/v2.1 collection, an environment, globals, a cURL command, or a Courier backup. It shows a preview (name and number of items) before importing. Pasting a cURL command into the URL bar also offers to convert it in place.
7. **Export.** Export a collection (v2.1 JSON), environment, globals, or a full Courier data backup. Copy a single request as cURL or raw HTTP.
8. **Run.** Right-click a collection or folder and choose **Run**, or open Runner from the status bar. Configure options, start the run, and watch results stream in.

### 3.3 Keyboard shortcuts (defaults)

| Action | Shortcut |
|---|---|
| New request tab | `⌘T` |
| New (collection/folder/environment chooser) | `⌘N` |
| Send request | `⌘↩` |
| Send and download response | `⌥⌘↩` |
| Save / Save As | `⌘S` / `⇧⌘S` |
| Close tab / Reopen closed tab | `⌘W` / `⇧⌘T` |
| Next / previous tab | `⌃⇥` / `⌃⇧⇥` |
| Jump to tab 1–8 / last | `⌘1`…`⌘8` / `⌘9` |
| Focus URL bar | `⌘L` |
| Find in sidebar | `⌘F` (sidebar focused) |
| Find in editor/response | `⌘F` (editor focused) |
| Import | `⌘O` |
| Toggle sidebar | `⌘\` |
| Toggle request/response layout | `⌥⌘V` |
| Toggle console | `⌥⌘C` |
| Manage environments | `⌥⌘E` |
| New runner tab | `⇧⌘R` |
| Rename / duplicate / delete in sidebar | `⌘E` / `⌘D` / `⌫` |
| Settings | `⌘,` |
| Shortcut cheat sheet | `⌘/` |
| Zoom in / out / reset | `⌘+` / `⌘-` / `⌘0` |

### 3.4 Accessibility & UX rules

- Full keyboard navigation of the sidebar tree (arrow keys, `←`/`→` to collapse/expand, `↩` to open).
- Every icon-only button has a tooltip.
- Text contrast meets WCAG AA in both themes.
- Long operations (send, run, import) show progress and can be cancelled.
- Destructive actions ask for confirmation. Delete from the sidebar can be undone for 10 s via a toast.
- `{{variables}}` are highlighted in URL, header, param and body editors: green when resolved, red when unresolved. Hovering shows the value and its scope. Typing `{{` opens autocomplete.

---

## 4. Requests

### 4.1 Methods

`GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`, plus any custom method token typed into the dropdown (for example `PURGE`, `PROPFIND`, `LINK`). Each method has a distinct colour in the sidebar and tabs.

### 4.2 URL & query params

- Free-text URL with `{{var}}` support. A missing scheme defaults to `http://`.
- Params table (Enabled · Key · Value · Description) kept **two-way in sync** with the URL query string. Editing either updates the other while keeping order, duplicates and disabled rows.
- **Path variables**: `:id` segments appear in a separate "Path Variables" table and are replaced at send time.
- **Bulk edit** mode on every key/value table: `key:value` lines, with `//` to disable a row.
- URL encoding: values are percent-encoded at send time unless the request setting `disableUrlEncoding` is on. Already-encoded sequences are left alone.

### 4.3 Headers

- Key/value table with Enabled, Description and bulk edit. Header-name autocomplete from a built-in list of standard headers, and value suggestions for common ones (`Content-Type`, `Accept`, …).
- **Auto-generated headers**, shown as a separate collapsible group that can be turned off one by one: `User-Agent: Courier/<ver>`, `Accept: */*`, `Accept-Encoding: gzip, deflate, br`, `Connection: keep-alive`, `Host`, `Content-Type` and `Content-Length` (from the body), and `Cache-Control: no-cache` (global setting). Auth-generated headers are previewed here read-only.

### 4.4 Body modes

| Mode | Details |
|---|---|
| **none** | No body. |
| **form-data** | `multipart/form-data`. Rows are text or file. A file row can hold several files and an optional per-row content type. Files are referenced by absolute path, or by a path relative to the working directory (setting). |
| **x-www-form-urlencoded** | Key/value table, encoded at send time. |
| **raw** | Text editor with a language selector: Text, JSON, JavaScript, HTML, XML. It sets `Content-Type` unless the user has overridden it. JSON mode has syntax highlighting, error markers and **Beautify**. Comments (`//`, `/* */`) in JSON are stripped before sending (Postman-compatible behaviour). |
| **binary** | A single file sent as the raw body. |
| **GraphQL** | Query editor plus a JSON variables editor, sent as `{"query":…,"variables":…}`. |

Switching modes keeps each mode's content until the request is saved. All body text supports `{{var}}`.

### 4.5 Per-request settings (`protocolProfileBehavior`)

| Setting | Default |
|---|---|
| Enable SSL certificate verification (`strictSSL`) | inherit global (on) |
| Automatically follow redirects (`followRedirects`) | on |
| Follow original HTTP method on redirect (`followOriginalHttpMethod`) | off |
| Follow Authorization header on cross-origin redirect (`followAuthorizationHeader`) | off |
| Remove Referer header on redirect (`removeRefererHeaderOnRedirect`) | off |
| Maximum redirects (`maxRedirects`) | 10 |
| Disable URL encoding (`disableUrlEncoding`) | off |
| Disable cookie jar (`disableCookies`) | off |
| Send body with GET/HEAD (`disableBodyPruning`) | off |
| HTTP version (`protocolVersion`): auto / HTTP/1.1 / HTTP/2 | auto |
| Request timeout (ms, 0 = none) | inherit global |

Unknown `protocolProfileBehavior` keys are kept on import and written back on export.

### 4.6 Sending

- Sends run on the tokio runtime. The UI stays responsive, and **Cancel** aborts the request at once.
- Order of operations: resolve the inheritance chain (auth and scripts from collection → folders → request) → run pre-request scripts → resolve variables → apply auth → build the `reqwest` request → send → capture timing → run post-response scripts → store in history → update the cookie jar.
- **Timing breakdown** (best effort with reqwest): total time, time to first byte and download time. DNS/TCP/TLS phases are listed as later work (§17).
- Response size is reported as headers plus body (compressed size on the wire when known, otherwise decoded size).
- Responses are decompressed automatically (gzip, deflate, brotli).
- **Send and Download** streams the body straight to a file chosen by the user.
- Bodies larger than the `maxResponseSize` setting (default 50 MB) are cut off in memory, with a notice.

---

## 5. Authorization

### 5.1 Inheritance

Every collection, folder and request has an auth setting. **Inherit auth from parent** is the default for folders and requests. **No Auth** stops inheritance. The UI says which ancestor the effective auth comes from. All auth fields support `{{var}}`.

### 5.2 Supported types

| Type | v1 scope |
|---|---|
| **No Auth** | ✔ |
| **Inherit** | ✔ |
| **API Key** | key, value, add to Header or Query Params |
| **Bearer Token** | token, sends `Authorization: Bearer …` |
| **Basic Auth** | username and password, base64-encoded |
| **Digest Auth** | username, password, realm, nonce, algorithm (MD5, MD5-sess, SHA-256, SHA-256-sess, SHA-512-256), qop (auth, auth-int), nc, cnonce, opaque. Automatic challenge handling: send, receive 401 with `WWW-Authenticate`, retry once. "Disable retrying" option. |
| **OAuth 1.0** | consumer key/secret, token/secret, signature method (HMAC-SHA1, HMAC-SHA256, HMAC-SHA512, PLAINTEXT; RSA-SHA* with a PEM private key), timestamp, nonce, version, realm, callback, verifier. Add to header or to query/body. Option to include the body hash. Option to leave empty params out of the signature. |
| **OAuth 2.0** | See §5.3. |
| **JWT Bearer** | algorithm (HS256/384/512, RS256/384/512, ES256/384, PS256/384/512), secret or private key, payload JSON, extra header JSON, prefix, sent as a header or query param. |
| **Hawk** | Hawk ID and key, algorithm (sha1/sha256), user, nonce, ext, app, dlg, timestamp, include payload hash. |
| **AWS Signature V4** | access key, secret key, session token, region, service. Add to header or query. |
| **NTLM**, **Akamai EdgeGrid**, **ASAP** | Kept in the data model, imported and exported unchanged. The UI shows "not supported in this version". Implementing them is future work (§17). |

### 5.3 OAuth 2.0

- **Grant types:** Authorization Code, Authorization Code with PKCE (S256/plain), Implicit, Password Credentials, Client Credentials.
- **Fields:** auth URL, token URL, callback URL, client ID, client secret, scope, state, audience, resource, code verifier, client authentication (Basic header vs. client credentials in body), extra token/auth request parameters.
- **Browser flows** (Authorization Code, PKCE, Implicit) open the system browser. Courier listens on a loopback redirect (`http://127.0.0.1:<port>/callback`, port chosen by the user or random) to receive the code. This needs no account or cloud callback.
- **Tokens:** a named token list per auth config, showing type, expiry and scope. Choose the token to use, delete tokens, and choose the header prefix (default `Bearer`). Add as header or query.
- **Auto refresh** with the refresh token when the token expires, if enabled.
- Tokens are stored locally (§12.4).

---

## 6. Variables

### 6.1 Scopes & precedence (highest wins)

1. **Local**: set by scripts with `pm.variables.set`, lasts for one request or run.
2. **Data**: the current runner iteration row.
3. **Environment**: the active environment.
4. **Collection**: collection variables.
5. **Global**.

### 6.2 Values

- Each environment or globals variable has an **initial value** (exported and shared) and a **current value** (local, used when sending, changed by scripts). When the current value is empty, the initial value is used. Actions: "Persist all" (current → initial) and "Reset all" (initial → current).
- **Secret type** masks values in the UI with a reveal toggle and in console logs. Export can leave out secret current values.
- **Resolution:** `{{name}}` is replaced recursively, for example `{{base}}` → `{{host}}/v1` → `api.example.com/v1`. Depth is limited to 20, and cycles leave the remaining text as-is and log a warning. Unresolved variables stay as the literal `{{name}}`.
- **Dynamic variables** (generated fresh for each use): `$guid`, `$randomUUID`, `$timestamp`, `$isoTimestamp`, `$randomInt` (0–1000), `$randomAlphaNumeric`, `$randomBoolean`, `$randomColor`, `$randomHexColor`, `$randomEmail`, `$randomExampleEmail`, `$randomUserName`, `$randomFirstName`, `$randomLastName`, `$randomFullName`, `$randomPassword`, `$randomPhoneNumber`, `$randomIP`, `$randomIPV6`, `$randomMACAddress`, `$randomUrl`, `$randomDomainName`, `$randomUserAgent`, `$randomCity`, `$randomCountry`, `$randomCountryCode`, `$randomStreetAddress`, `$randomCompanyName`, `$randomJobTitle`, `$randomWord`, `$randomWords`, `$randomLoremSentence`, `$randomLoremParagraph`, `$randomPrice`, `$randomCurrencyCode`, `$randomDatePast`, `$randomDateFuture`, `$randomDateRecent`, `$randomSemver`, `$randomFileName`, `$randomMimeType`, `$randomLatitude`, `$randomLongitude`. Unknown `$random*` names are left as-is. Values come from small built-in word lists written for Courier; no data is copied from other projects.
- **Autocomplete & hover:** see §3.4. A variable's scope is shown with a coloured badge.

---

## 7. Collections & folders

- Collection properties: name, description (Markdown, shown rendered and editable), auth, pre-request and post-response scripts, variables (Enabled · Key · Initial · Current · Type), `info._postman_id` kept on round-trip.
- Folder properties: name, description, auth, scripts.
- Request properties: name, description, everything in §4–5, scripts, **examples**.
- **Examples:** "Save response as example" stores the request and response (status, headers, body). Examples are listed under the request in the sidebar and open read-only with an Edit toggle.
- **Ordering** is the order in the file and can be changed by drag-and-drop.
- **Search** filters the tree by name or URL, matching case-insensitive substrings.
- One JSON file per collection. Files are loaded lazily, so large collections (10k requests) render without a hitch through a virtualized tree.

---

## 8. History

- Every send from a request tab appends a history entry: timestamp, the final resolved request (method, URL, headers, body up to 1 MB), the original unresolved request, the response (status, headers, body up to the history cap of 1 MB, time, size), and the collection/request ID it came from.
- Runner sends are **not** added to history; they appear in run results instead.
- Retention: keep the newest 1,000 entries (setting: 100–10,000, or "keep for N days").
- Grouped by day, with text search over URL and method. Actions: open, save to collection, copy as cURL, delete, clear all.
- Stored in `history/` as daily JSON files (§12), so trimming is cheap.

---

## 9. Scripting

### 9.1 Engine

- JavaScript runs in an embedded **QuickJS** interpreter (`rquickjs` crate), one sandbox context per script run. There is no file system, process or network access except through the `pm` API (`pm.sendRequest`).
- Timeout: 10 s per script (setting). Memory limit: 64 MB per context.
- `console.log/info/warn/error/debug` go to the Courier Console with the request name and a timestamp.
- Built-in helper modules available through `require(...)`: `lodash`-like helpers are **not** bundled. v1 provides only Courier-written helpers: `atob`, `btoa`, `crypto-js`-compatible hashing through `pm.crypto` (MD5, SHA-1, SHA-256, SHA-512, HMAC, base64), `uuid`, `JSON`, and a small `xml2Json` helper. Unsupported `require` names throw a clear error.

### 9.2 Execution order

For each request:

```
collection pre-request → folder pre-request (outer→inner) → request pre-request
→ send →
collection post-response → folder post-response (outer→inner) → request post-response
```

Changes that scripts make to environment, collection and global variables are written to the stores after each script and persisted to disk after the request (or at the end of the run if "persist variables" is on, §10).

### 9.3 `pm` API (v1 surface)

| Area | Members |
|---|---|
| Variables | `pm.variables.{get,set,has,unset,replaceIn,toObject}`, `pm.environment.{get,set,has,unset,clear,replaceIn,toObject,name}`, `pm.collectionVariables.{…}`, `pm.globals.{…}`, `pm.iterationData.{get,has,toObject}` |
| Request | `pm.request.url` (string or object with `toString()`, `getHost()`, `query`), `pm.request.method`, `pm.request.headers.{get,has,add,upsert,remove,toObject}`, `pm.request.body` (mode plus raw/urlencoded/formdata), `pm.request.auth`. Changes made in pre-request scripts apply to the outgoing request. |
| Response | `pm.response.code`, `.status`, `.headers.{get,has,toObject}`, `.responseTime`, `.responseSize`, `.text()`, `.json()`, `.cookies` |
| Response assertions | `pm.response.to.have.status(code\|text)`, `.to.have.header(name[,value])`, `.to.have.body(str)`, `.to.have.jsonBody([path[,value]])`, `.to.be.ok/success/error/clientError/serverError/json`, `.to.have.responseTime.below(ms)` |
| Tests | `pm.test(name, fn)` (also async with a `done` callback), `pm.test.skip(name, fn)`, `pm.expect(value)` |
| `pm.expect` | Courier's own Chai-style BDD implementation: chains `to/be/been/is/that/which/and/has/have/with/at/of/same/not/deep/nested/own/include/any/all`; assertions `equal/eql/a/an/ok/true/false/null/undefined/exist/empty/above/below/least/most/within/lengthOf/property/keys/include/match/oneOf/string/members/instanceOf/closeTo/throw/satisfy`. Failure messages state expected vs actual. |
| Info | `pm.info.{requestName,requestId,iteration,iterationCount,eventName}` |
| Flow | `pm.execution.setNextRequest(nameOrId\|null)`, `pm.execution.skipRequest()` (pre-request only), legacy `postman.setNextRequest` |
| Network | `pm.sendRequest(urlOrRequest, callback(err, res))`, plus a Promise form when no callback is given |
| Cookies | `pm.cookies.{get,has,toObject}`, `pm.cookies.jar().{get,getAll,set,unset,clear}` |
| Legacy compatibility | `responseBody`, `responseCode.code`, `responseTime`, `responseHeaders`, `tests["name"] = bool`, `environment`, `globals`, `postman.{setEnvironmentVariable,getEnvironmentVariable,setGlobalVariable,…}` |
| Visualizer | `pm.visualizer.set(template, data)` is accepted and **ignored with a console notice** in v1 (needs an HTML renderer; future work). |

### 9.4 Script editor

A code editor with syntax highlighting, line numbers, and `⌘F` find. A snippet side panel inserts common snippets written by Courier, for example "Status code is 200", "Set an environment variable" and "Response time below 200 ms".

### 9.5 Test results

The Response → **Tests** tab lists each test as Pass, Fail (with message) or Skipped, with All/Passed/Failed/Skipped filters. Script errors appear as a red banner with the line number.

---

## 10. Collection Runner

- **Inputs:** collection or folder; ordered list of requests with checkboxes to include and drag-to-reorder; environment; iterations (1–10,000); delay between requests (ms); data file (CSV with a header row, or a JSON array of objects) with a preview; **Persist responses for session** (keep bodies in results; off keeps only metadata); **Keep variable values** (write variable changes made during the run back to the stores; on by default); **Stop run if an error occurs**; **Stop run on first test failure**.
- If a data file is given and iterations isn't set, the iteration count is the number of data rows. If iterations is larger than the row count, the last row repeats (reference behaviour).
- **Flow control:** `setNextRequest(name)` jumps to that request within the run scope. `null` ends the iteration. `skipRequest()` skips the current request.
- **Live results view:** per-iteration sections; each request shows method, URL, status, time, size and its tests. Summary bar with totals for passed/failed tests and average response time. Filters: all/passed/failed. Click a row to see the request and response details.
- **Run controls:** Pause, Resume, Stop. "Run again" with the same configuration.
- **Results:** stored in `runs/` (keep the last 50). Export as Courier run JSON, and as JUnit XML for CI tooling.
- The run engine lives in the core library, so it can be tested headless. A CLI (`courier run <collection.json> -e env.json -n 3 -d data.csv`) is listed in §17.

---

## 11. Import & Export

### 11.1 Postman Collection v2.1 / v2.0

- **Import:** validate `info.schema` (v2.0.0 or v2.1.0 URL) and map every field in the format to Courier's model. Both versions share structure; v2.0 auth parameter maps are converted to v2.1 arrays.
- **Export:** write v2.1.0 (`https://schema.getpostman.com/json/collection/v2.1.0/collection.json`) with pretty-printed JSON and stable key order.
- **Round-trip guarantee:** import → export → import gives an equal model. Unknown or unsupported fields (for example `protocolProfileBehavior` keys, NTLM auth, `event` scripts of other types, item-level `variable`) are kept in an `extra` map with `#[serde(flatten)]` and written back.
- Field coverage: `info` (name, `_postman_id`, description, schema), `item` (recursive folders/requests), `request` (method, header, url as object and raw string, body in every mode, including `options.raw.language`, auth, description), `response` (examples), `event` (prerequest/test with `script.exec` arrays), `variable`, `auth`, `protocolProfileBehavior`.
- The URL object is written with `raw`, `protocol`, `host[]`, `port`, `path[]`, `query[]` and `variable[]`, all derived from the raw URL.
- **Environment files** (`_postman_variable_scope: "environment"`) and **globals** files (`"globals"`) are imported and exported in that format, including `type: "secret"`.
- **Postman data dump** (a multi-collection backup) is imported if it is detected; this is best effort.

### 11.2 cURL

**Import** tokenizes the command shell-style (single/double quotes, `\` line continuation, `$'…'` ANSI-C quoting, Windows `^` continuation) and supports:

| Flags | Mapping |
|---|---|
| `-X/--request` | method |
| `--url`, positional URL | URL |
| `-H/--header` | header (`Cookie:` kept as a header) |
| `-d/--data`, `--data-raw`, `--data-ascii`, `--data-binary` | body (raw or urlencoded, guessed from `Content-Type`). Several `-d` flags are joined with `&`. `@file` becomes a binary/file body reference. |
| `--data-urlencode` | urlencoded field |
| `-F/--form`, `--form-string` | form-data (text or `@file`, `;type=`) |
| `-G/--get` | move data into the query string, method GET |
| `-I/--head` | HEAD |
| `-u/--user` | Basic auth (`--digest` → Digest, `--ntlm` → NTLM) |
| `-A/--user-agent`, `-e/--referer` | headers |
| `-b/--cookie` | `Cookie` header |
| `-k/--insecure` | `strictSSL: false` |
| `-L/--location`, `--max-redirs` | follow redirects / max redirects |
| `--compressed` | `Accept-Encoding` header |
| `--http1.1`, `--http2` | protocol version |
| `-T/--upload-file` | binary body, method PUT |
| `-m/--max-time` | timeout |

Other flags (`-v`, `-s`, `-o`, `-i`, …) are ignored with a notice. Unknown flags give a warning rather than failing. Several commands in one paste, or `--next`, become several requests.

**Export** ("Copy as cURL") gives a multi-line command with `\` continuations, using `--location` when following redirects, `--request` only when the method is not implied, `--header`, `--data-raw`/`--form`/`--data-urlencode`/`--data-binary @file`, and auth turned into headers or `--user`. Variables are resolved by default; an option keeps the `{{var}}` placeholders. Quoting is safe for POSIX shells.

### 11.3 Other exports

- **Raw HTTP** snippet (request line, headers, body).
- **Courier backup:** a single JSON file containing all collections, environments, globals and settings (not history), and restore from it.

---

## 12. Storage

### 12.1 Location

`~/Library/Application Support/Courier/` (found with the `directories` crate). It can be overridden with the `COURIER_DATA_DIR` environment variable, which tests use.

```
Courier/
├── settings.json
├── workspace.json          # open tabs (incl. unsaved drafts), active env, sidebar & layout state
├── globals.json
├── cookies.json
├── oauth_tokens.json
├── collections/<uuid>.json # one Courier collection per file (v2.1-shaped, see 12.2)
├── environments/<uuid>.json
├── history/<YYYY-MM-DD>.json
└── runs/<uuid>.json
```

### 12.2 Format & safety

- Every file has a top-level `"courierFormat": 1` used for migrations. Collection files keep the v2.1 shape plus Courier metadata under `"_courier"`, so a user can open one by hand in other tools.
- **Atomic writes:** write to `*.tmp` in the same directory, `fsync`, then `rename`. A corrupt file (JSON parse error) is moved to `*.corrupt-<timestamp>` and the user is warned, never silently wiped.
- Writes are debounced (300 ms) and run off the UI thread on a single writer task, so the order of writes is guaranteed.
- On quit, any pending writes are flushed. Unsaved tab drafts are kept in `workspace.json`, so restarting the app loses nothing.

### 12.3 File permissions

Data files are created with mode `0600` and directories with `0700`.

### 12.4 Secrets

v1 stores secret variable values and OAuth tokens in the local JSON files (with the 0600 permissions above). Storing them in the macOS Keychain is an open question (§16).

---

## 13. Cookies

- One cookie jar per domain, following RFC 6265 rules for domain, path, expiry, `Secure` and `HttpOnly`. Responses fill the jar and requests use it automatically, unless `disableCookies` is set.
- **Cookie manager** dialog: list domains, then add, edit or delete cookies or whole domains. A **domain allowlist** controls which domains scripts may read through `pm.cookies.jar()`.
- The Response → Cookies tab shows the cookies set by that response.
- The jar is persisted to `cookies.json`.

---

## 14. Settings

| Group | Settings |
|---|---|
| General | Request timeout (ms, 0 = none); max response size (MB); send `Cache-Control: no-cache`; trim whitespace in keys/values; open new requests in a new tab; working directory for file uploads; allow reading files outside the working directory; language detection for response bodies (auto/JSON/…); history retention |
| Appearance | Theme (System/Light/Dark); UI zoom; editor and response font size; line wrapping; show whitespace; request/response layout |
| Certificates | CA certificate (PEM) on/off and path; client certificates per host and port (CRT + KEY, or PFX with passphrase) |
| Proxy | Use system proxy; respect `HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY`; custom proxy (HTTP/HTTPS/SOCKS5, host, port, basic auth, bypass list) |
| SSL | Global SSL verification on/off; minimum TLS version |
| Shortcuts | View-only list in v1 |
| Data | Show data folder; export backup; import backup; clear history; reset all settings |

---

## 15. Architecture

### 15.1 Crate layout

One Cargo package, `courier`, with a library target (all logic, no UI dependencies) and a binary target (the egui app). This keeps the core testable without a window and lets `cargo-bundle` work unchanged.

```
src/
├── lib.rs
├── model/        # Collection, Folder, Request, Auth, Body, Variable, Environment, Example, ids
├── storage/      # data dir, atomic JSON IO, migrations, debounced writer
├── vars/         # scope chain, {{}} resolution, dynamic variables
├── http/         # request builder (reqwest), settings, redirects, timing, cancel, download
├── auth/         # basic, bearer, apikey, digest, oauth1, oauth2 (+ loopback server), jwt, hawk, awsv4
├── cookies/      # jar + persistence
├── script/       # rquickjs sandbox, pm API bindings, Courier's expect library (JS shim)
├── runner/       # iteration engine, data files, flow control, results, JUnit export
├── history/      # append, trim, query
├── formats/
│   ├── postman_v21.rs   # import/export collection, environment, globals
│   └── curl.rs          # tokenizer, parser, generator
└── app/          # (binary-only) egui UI: shell, sidebar, tabs, editors, dialogs, theme
main.rs           # eframe bootstrap
```

### 15.2 Concurrency

- The `eframe` UI runs on the main thread. A multi-threaded **tokio** runtime runs on a background thread.
- The UI sends `Command`s (Send, Cancel, RunCollection, SaveCollection, …) over an `mpsc` channel. The core sends `Event`s back (ResponseReady, RunProgress, Saved, Error) and calls `ctx.request_repaint()`.
- Each in-flight request holds a `tokio_util::sync::CancellationToken`.
- Script execution runs on a blocking thread (`spawn_blocking`) because QuickJS is not `Send` across awaits. `pm.sendRequest` from a script blocks that thread on a tokio handle.

### 15.3 Key dependencies

| Purpose | Crate |
|---|---|
| UI | `eframe`, `egui`, `egui_extras` (tables, syntax highlighting), `rfd` (native file dialogs) |
| HTTP | `reqwest` (rustls, gzip/brotli/deflate, multipart, stream, cookies off since Courier manages its own jar, socks) |
| Async | `tokio`, `tokio-util` |
| Serialization | `serde`, `serde_json`, `csv` |
| Scripting | `rquickjs` |
| Auth crypto | `hmac`, `sha1`, `sha2`, `md-5`, `base64`, `jsonwebtoken`, `rsa`, `rand` |
| Misc | `uuid`, `chrono`, `url`, `percent-encoding`, `shell-words` (cURL tokenizing, with extra handling), `directories`, `thiserror`, `anyhow`, `tracing` |
| Testing | `wiremock` (local HTTP mock), `tempfile`, `pretty_assertions`, `egui_kittest` (UI smoke tests) |

Exact versions are pinned in `Cargo.toml` when implementation starts.

### 15.4 Testing strategy

- **Unit tests** in each module: model serde, variable resolution (precedence, recursion, cycles, dynamic variables), every auth signer against known test vectors from the RFCs and AWS documentation, cURL tokenizer and parser tables, the v2.1 mapper.
- **Integration tests** (`tests/`): send real requests to a local `wiremock` server for every body mode, redirect, cookie, timeout, cancel and digest challenge case. Run a collection end to end with data files, `setNextRequest`, and stop-on-failure.
- **Round-trip fixtures:** hand-written Collection v2.1 fixtures (Courier's own, no third-party collections) covering every auth type, body mode and nesting depth. The test is import → export → JSON equality.
- **Scripting tests:** a table of script snippets with their expected test results and variable changes.
- **UI smoke tests** with `egui_kittest`: open the app state, create a request, send it to the mock server, and check that the response is shown.
- All tests must pass, with `cargo clippy -- -D warnings` and `cargo fmt --check` clean, before each feature commit.

### 15.5 Packaging

- `cargo install cargo-bundle`, then `cargo bundle --release` produces `target/release/bundle/osx/Courier.app`.
- `Cargo.toml` `[package.metadata.bundle]`: name `Courier`, identifier `app.courier.desktop`, an original icon (`assets/icon.png` drawn for Courier, at 1024² down to 16²), category `public.app-category.developer-tools`, `osx_minimum_system_version = "11.0"`, a copyright line for Courier.
- The build is ad-hoc signed (`codesign -s -`) so it launches locally. Developer ID signing and notarization are out of scope.

---

## 16. Open questions (defaults chosen; tell me if you want otherwise)

1. **Script engine:** `rquickjs` (QuickJS: near-complete ES2023, small, fast; builds C code). Alternative: `boa_engine` (pure Rust, slower, less complete). **Default: rquickjs.**
2. **Secrets:** plaintext JSON with 0600 permissions (default) vs. macOS Keychain via the `keyring` crate.
3. **Bundle identifier:** `app.courier.desktop`. Any preferred ID or copyright holder?
4. **Code editor:** `egui_extras` syntax highlighting with a plain multiline `TextEdit` (default) vs. a heavier third-party code-editor widget.
5. **Scope trims:** NTLM, EdgeGrid and ASAP auth are kept-but-not-sent in v1, and HTML response preview shows rendered text only (egui can't embed a browser). OK?

---

## 17. Future work (not in v1)

WebSocket/gRPC/SSE clients · OpenAPI/HAR/RAML import · multi-language code generation · `courier` CLI runner · HTML response preview and Visualizer · NTLM/EdgeGrid/ASAP auth · DNS/TCP/TLS timing phases · Keychain-backed secrets · request capture proxy · mock server · Git-friendly folder-per-request storage · plugin API.

---

## 18. Implementation plan (one commit per feature, each with tests)

| # | Feature commit | Tests |
|---|---|---|
| 0 | Scaffold: Cargo package (lib + bin), `.gitignore`, fmt/clippy config, `git init` | builds, empty test passes |
| 1 | Data model (collections, folders, requests, bodies, auth, variables, environments, examples) | serde round-trip unit tests |
| 2 | Storage: data dir, atomic writes, debounced writer, corrupt-file handling, migrations stub | tempdir tests |
| 3 | Variables: scopes, precedence, recursive resolution, dynamic variables | unit tests |
| 4 | HTTP engine: methods, params/path vars, headers, all body modes, settings, redirects, timeout, cancel, decompression, download, timing | wiremock integration tests |
| 5 | Auth I: inheritance, API key, Bearer, Basic, Digest | RFC 7616 vectors, wiremock challenge test |
| 6 | Auth II: OAuth 1.0, JWT, Hawk, AWS SigV4 | published test vectors |
| 7 | Auth III: OAuth 2.0 (all grants, PKCE, loopback redirect, token store, refresh) | wiremock token endpoint tests |
| 8 | Cookie jar + persistence | unit and integration |
| 9 | Postman Collection v2.1/v2.0 + environment/globals import and export | fixture round-trips |
| 10 | cURL import and export | parser tables, generator → parser round-trip |
| 11 | Scripting engine + `pm` API + expect library + test results | script snippet tables |
| 12 | History store | unit |
| 13 | Collection runner (iterations, data files, flow control, stop rules, JUnit export) | integration |
| 14 | UI shell: theme, layout, sidebar (collections/env/history), tabs, workspace restore | kittest smoke |
| 15 | UI: request editor (all tabs), response viewer, send/cancel, save/save-as, variable highlighting | kittest |
| 16 | UI: environments and globals editor, collection/folder settings, examples | kittest |
| 17 | UI: import/export dialogs, cURL paste-to-convert, copy as cURL | kittest |
| 18 | UI: runner, console, cookie manager, settings, shortcuts | kittest |
| 19 | App icon, `cargo-bundle` metadata, release build of `Courier.app` | bundle builds and launches |

---

## Appendix A — Reference study method (`rea` 6.8.0)

Read-only static inspection of `/Applications/Postman.app` v11.93.0. Postman was never launched or modified.

- `rea inspect-plist` showed the bundle metadata and a custom URL scheme.
- `rea inspect-desktop-application` found an Electron runtime (`Contents/Resources/app.asar`).
- `rea inspect-artifact` gave an inventory of 445 entries. Module names showed the feature areas: the offline "Scratch Pad" / Lightweight API Client mode, collection/environment/history sidebars, runner, the cURL/OpenAPI/RAML/WSDL/WADL/HAR/GraphQL importers, code generators, WebSocket/gRPC/MQTT workbenches, mock server, performance testing, Interceptor/proxy capture, an AI agent, and 11 editor themes.
- `rea extract-artifact` extracted the contents to a temporary folder for **feature-level enumeration only**: identifier names for auth types, body modes, the dynamic-variable catalogue, the `pm` sandbox API names, per-request `protocolProfileBehavior` keys, OAuth 2.0 grant types, settings keys, keyboard-shortcut actions, recognised cURL flags, and supported collection export versions (v2.1 default, v2.0, v1).
- `rea analyze-javascript-application` (full semantic reconstruction) was started and then stopped, because the enumeration above was enough for a functional spec.

Nothing from those files is copied into Courier. This spec describes behaviour in Courier's own words, and all wording, structure, visuals and code will be original.
