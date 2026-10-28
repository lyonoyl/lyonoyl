# Working on Lyonoyl Mail

How to run it, build it, and develop against a mail server. What the product is and how to install it is in [the README](../../README.md); the rules a change has to follow are in [the working contract](../../AGENTS.md).

## What a build needs

| Requirement | Version | Notes |
| --- | --- | --- |
| [Rust](https://rustup.rs) | the one `rust-toolchain.toml` names | through `rustup`, with the `rustup toolchain install` line `npm run check:machine` names |
| [Node.js](https://nodejs.org) | 20.19+, 22.12+ or 24+, what Vite and Vitest accept | ships with npm |
| macOS | 13 Ventura or newer | Xcode Command Line Tools |
| Windows | 11 | Visual Studio Build Tools with C++, Perl, and LLVM on Arm |
| Linux | WebKitGTK 2.44 or newer | `build-essential`, `pkg-config`, `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libsoup-3.0-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, and `libssl-dev` on Ubuntu |

The preflight needs more than a build:

| Requirement | Version | Install |
| --- | --- | --- |
| [Docker](https://docs.docker.com/get-docker/) with Compose | any current | Docker Desktop on macOS and Windows, Docker Engine with its Compose plugin on Linux |
| `cargo-audit` | any current | `cargo install cargo-audit --locked` |
| `cargo-about` | 0.9.2, the one the license list is collected with | `cargo install cargo-about --locked --version 0.9.2` |
| Python 3 and git | any current | the Xcode Command Line Tools on macOS, the system's packages on Linux |

`npm run check:machine` asks whether this machine has all of it, the Windows row aside, and names what is missing with how to get it; the preflight asks the same as its first step. `npm run app:build` and `npm run app:dev-build` ask the first table alone before they build, as `node scripts/check-machine.mjs --build` does.

## Running it

| Command | What it does |
| --- | --- |
| `npm start` | **Asks what you want and runs it.** It offers only what this machine can do, and says why when it leaves the release out. |
| `npm run app` | Work on the code: a debug build with hot reload. Editing the interface updates the open window; editing Rust rebuilds and restarts it. On macOS it signs what it launches—see below for why that is not about distributing anything. |
| `npm run app:build` | An application to keep and use, unsigned, on any of the three systems: the build the preflight makes as its last step, without the checks before it. It says where it put it and what to do with it, because a path inside `target/` does not look like somewhere to keep things. |
| `npm run app:dev-build` | macOS. A debug build of the same bundle, signed, at `target/debug/bundle/macos/Lyonoyl Mail.app`, for working against rather than for sending anybody. |
| `npm run app:release` | macOS. The one thing here another machine opens: signed, notarized, stapled, and then asked what Gatekeeper makes of it. |
| `npm run app:screenshots` | macOS. The application over a made-up scene, for taking screenshots: [A build for screenshots](#a-build-for-screenshots). |

**Only `app:release` signs for anybody else.** What `app:build` makes is refused by Gatekeeper on every machine but the one that built it—which is the right answer for a build you made yourself, and the script says so rather than leaving it to be discovered.

Two commands look like they should run the app but do not: `npm run build` only bundles the frontend, and `npm run dev` serves that frontend in a browser, where the native part is missing and every call into it fails.

Finished builds:

| Platform | Application | Installer |
| --- | --- | --- |
| macOS | `target/release/bundle/macos/Lyonoyl Mail.app` | `target/release/bundle/dmg/`, from `app:release` |
| Windows | `target/release/lyonoyl.exe` | none: a release carries the source, built there |
| Linux | `target/release/lyonoyl` | none: a release carries the source, built there |

`app:build` makes the application and stops there. An installer is a thing to send somebody, and the only one this repository produces is the signed macOS disk image.

### macOS: the keychain and development builds

Every account keeps its password in the system store, the three development accounts `scripts/reset-mail-server.sh` fills included: there is no development bypass, and the application holds no password of its own. The build for screenshots is apart from this, and holds nobody's mail: [A build for screenshots](#a-build-for-screenshots).

macOS asks before letting a development build read a password back, because every `cargo build` produces an ad-hoc signature the system treats as a new application. **Signing with a certificate is what stops it, and `npm run app` does that**—see below. On a checkout with no certificate set up, expect the dialog and answer it with **Always Allow**, which holds until the next Rust rebuild.

The measurements, and why a keychain timing taken from `cargo test` means nothing, are in [the platform differences](../platforms/differences.md).

### A mail server to develop against

Connecting the application to a real mailbox to debug synchronization is how a developer loses their own mail. There is a local server instead—Dovecot for reading and Postfix for sending, in two containers, reachable only from this machine. Start it before `cargo test`: without it, the tests that ask a real server what it did skip themselves and pass.

```bash
docker compose -f docker/mail-server/compose.yml up -d --build --wait
scripts/reset-mail-server.sh                   # every development mailbox back to the corpus
scripts/seed-mail-server.sh you@example.com 50 # a larger mailbox, 50 copies of the corpus
```

It reads on `127.0.0.1:1143` and takes mail to send on `127.0.0.1:1587`, both without encryption, which the client accepts for loopback addresses and nothing else. Any address works as a user name—its mailbox is made the first time it is used—and the password is always `lyonoyl`. It advertises `CONDSTORE`, `QRESYNC`, and `SPECIAL-USE`, so all three synchronization strategies can be exercised against it.

**The tests do not read the development accounts.** A test that needs mail signs in as an address of its own and fills that mailbox itself—`corpus-…@lyonoyl.local` with the corpus, over IMAP, left alone while it holds the corpus byte for byte from the first UID, and started over and filled again when it holds anything else—so a server started on an empty volume is enough for the whole suite. The development accounts are for a person looking at the application. `scripts/reset-mail-server.sh --if-fresh` fills them only when none of them holds a message in any folder and empties nothing; `scripts/preflight.sh` runs it that way.

**It relays nowhere.** Sending asks you to sign in first, and takes recipients at `example.com`, `example.net`, `example.org`, `lyonoyl.local`, `first-domain.test`, and `second-domain.test` only. Each message is filed in the inbox that signs in as the address it was sent to. Any other recipient is refused while the message is still being addressed, so nothing sent here reaches anybody.

**Watching a sent message arrive.** Add two accounts on the account screen—`zeta@first-domain.test` and `kappa@second-domain.test`, say—each reading from `127.0.0.1` port `1143` and sending through `127.0.0.1` port `1587`, no encryption on either, the address as the user name and `lyonoyl` as the password. Write from one to the other and send; the message is in the other's inbox at its next pass, or at once after you press the refresh button. The server's side of it, as it happens:

```bash
docker logs -f lyonoyl-mail-submission    # "status=sent (250 2.0.0 ... Saved)" is the hand-over
docker exec lyonoyl-mail-server doveadm search -u kappa@second-domain.test mailbox INBOX all
```

`scripts/reset-mail-server.sh` drops anything still waiting to be delivered before it empties the mailboxes, so a reset leaves nothing behind that arrives afterwards.

Stop it with `docker compose -f docker/mail-server/compose.yml down -v`; the `-v` throws the mailboxes away as well.

### A build for screenshots

`npm run app:screenshots` builds the application with the `screenshots` feature, which no other build compiles in, and opens it. It is the application in every part a person sees, over a profile of its own that is emptied at every start and filled from a scene: the accounts, their groups, the language, and the mail, read by the pass every account is read with, from servers that answer out of the scene.

- **Its mail comes from no server, and what is done in it goes to none.** Every pass, every message opened, and every account checked or added is answered from the scene, and an account the scene does not have has no connection. Marking a message, moving it, throwing it out, and sending one are taken as a server takes them and passed on to nobody: the scene does not change, and the next start plays it again. What a person asks the window for still goes where it would: a link, the pictures from the web in a message, the settings a domain publishes for its mail.
- **It touches nothing of the real application's.** Its profile is `~/Library/Application Support/Lyonoyl Mail Screenshots`, its secrets are kept in memory while it runs, and the system's store is never asked, it never looks for an update, and the system knows it as another application, `com.lyonoyl.mail.screenshots`, so it runs beside the real one.
- **It is left at `target/screenshots/Lyonoyl Mail.app`.** Opening it again plays the scene again, dated from that moment. An application `app:build` left in `target/release/bundle/` is set aside while it builds and put back.

The scenes are in `mocks/screenshots/`, a folder each, named by language; `en` is the one there is, and it is played unless another is named: `npm run app:screenshots -- --<language>` plays `mocks/screenshots/<language>`. A folder holds `scene.json`, and beside it whatever its messages written in HTML show. The test suite reads and plays every folder there, so a scene that would be refused is refused by `scripts/preflight.sh` before anybody builds it.

| Field | What it holds |
| --- | --- |
| `language` | The window's language, as its catalog is named. The system's where there is none. |
| `welcome` | `true` opens the window on the welcome, as on a first start. Behind it where there is none. |
| `appearance`, `palette` | `light`, `dark`, or `system`, and one of that appearance's palettes as the settings store it: `paper`, `nord`. Light on Paper where there is neither. |
| `accounts` | An `address`, a `name` where the account goes by one, and its `mail`. Every account has an inbox, drafts, sent mail, an archive, junk, and trash, whether or not anything is in them. The top of the sidebar is in the order they are listed, and a group stands where its first account is. |
| `from`, `to` | `Name <address>`, or an address alone. `to` is the account where there is none. |
| `subject`, `text` | The subject, and what the message says as plain text, which the list shows under the subject. |
| `html` | A file in the scene's folder holding the message as HTML, which is what the reader shows. A picture it names by a path in that folder, `<img src="moodboard/cover.jpg">`, travels inside the message, so the file opens in a browser the way the reader shows it. PNG, JPEG, GIF, or WebP. |
| `open` | The message the window opens on, chosen in the list. One in a scene at most. |
| `ago` | How long before the start it arrived: `40m`, `5h`, `3d`. |
| `folder` | `inbox`, `sent`, `archive`, `junk`, or `trash`. The inbox where there is none. |
| `unread`, `flagged` | As the list shows them. Read and unflagged where they are absent. |
| `id`, `answers` | A name for a message, and the name of the message this one answers: the two are one conversation, and the answer arrives after what it answers. |
| `groups` | A `name`, the `accounts` in it in the order they are drawn, and `collapsed` where it is folded. An account is in one group at most, and a group with none in it stands last. |

A field nothing reads, an address that is not one, an answer to nothing, and a file outside the scene's folder are refused with the place they are in. The window does not open over a scene it refused, and the script prints what was said.

## Before a commit

`scripts/preflight.sh` must be green before every commit, and it is the whole answer: nothing else runs anywhere, on any machine but this one. It fails the run rather than warning: a failed step is recorded and the steps after it still run, so the summary names every failure. Two things stop it outright: a tool it needs and cannot find, at the first step, and a mail server it cannot start, at the second—a check that skips itself and reports green is the fault it refuses.

**It ends with a summary.** Every step, passed or failed, and under a failed one what failed: for the two test suites, every failed test, grouped by the test program or the file it is in, with where it stopped, the first lines of what it said, and why where the reason is one the summary knows—a mail server that stopped, a connection to it that never completed, a machine with no port left. `scripts/failed-tests.mjs` reads what the suites wrote: the Rust suite's whole output stays in `target/preflight/rust-tests.log` and the component suite's report in `target/preflight/frontend-tests.json`. The Rust suite runs with `--no-fail-fast`, so a failed test program does not keep the ones after it from running.

| What it asks | Why it is in the list |
| --- | --- |
| This machine | `scripts/check-machine.mjs`: everything the steps below call, asked at once, each missing thing with how to get it—the two tables in [What a build needs](#what-a-build-needs). The `rustc` first on PATH must be rustup's own, since only it honors `rust-toolchain.toml`, and a newer clippy refuses this tree on lints the pinned one lacks. On a Mac that has run 49.7 days without a restart, it watches for 35 seconds whether the system still clears closed connections: one that keeps them hangs new connections at random, the tests' to the mail server among them, so it says to restart the Mac. |
| The development mail server | Alone, the tests that ask a real mail server what it did skip themselves and pass when nothing answers, so this run starts the server itself and sets `LYONOYL_MAIL_SERVER_REQUIRED=1`, which turns each of those skips into a failure. It asks Docker first, and stops at once, saying which, when that gets no answer within 20 seconds or Docker refuses this user. It then starts the server, waits up to 180 seconds for it to report healthy, and stops if either of its two ports does not come up. Where it stops, it names the command that ran and prints what Docker says about the two containers. A server whose development accounts hold no mail at all is filled from the corpus; one where any of them holds mail is left as it is. |
| `cargo fmt --check`, `cargo clippy -D warnings` | Formatting and lints, with warnings refused. |
| `cargo clippy --features screenshots` | The same over the build for screenshots, which no other step compiles. |
| `cargo test --workspace` | Everything, including the tests that talk to that server. |
| `cargo check --release` | That the release profile still compiles, which the debug one does not prove. |
| `scripts/check-layering.sh` and its self-test | The boundaries, the permanent prohibitions, the content policy, and the anonymity rules—and then whether the gate would still refuse each of them. |
| `npm run typecheck`, `npm test` | The types and the component suite. |
| The generated bindings | Regenerated and compared, so a DTO that drifted cannot be committed. |
| The license list | Rebuilt from the tree and compared with what is committed: a dependency arrives with its license rather than announcing it. Then the window is built in memory and every package in it must have a notice in the list. A package that publishes no license file stops the collector; its project's notice goes in `src-tauri/licences-copied/` and is named in `scripts/collect-npm-licences.mjs`. |
| `cargo audit --deny yanked`, `npm audit --audit-level=high` | An advisory published against something already here—on the npm side, one rated high or critical—and a crate pulled from the registry. |
| `npm run build` | The bundle built rather than only checked. |
| `npx tauri build` | The application, built the way `npm run app:build` builds it: linked, with the frontend it carries, and on macOS packed as the `.app` with its icons and resources. A check that compiles is not a build that builds. |

**Ten advisories print on every run, and they are known.** `unic-char-property`, `unic-char-range`, `unic-common`, `unic-ucd-ident`, `unic-ucd-version` (RUSTSEC-2025-0075, -0080, -0081, -0098, -0100) and `proc-macro-error` (RUSTSEC-2024-0370) are unmaintained, all pulled in by build-time machinery rather than by anything that reads mail, and none of them is a vulnerability. `zvariant`, `zbus_names`, and `zvariant_derive` (RUSTSEC-2026-0370, -0371, -0372) are unmaintained too: they are the D-Bus layer that notifications, the Secret Service, opening a link, and the one-copy rule speak on Linux, and none of them is a vulnerability either. `glib` 0.18.5 (RUSTSEC-2024-0429) has unsoundness in two iterator implementations; it reaches this tree only through the Linux window toolkit, and nothing here touches `VariantStrIter`. Anything else it prints is new.

**One run at a time.** A second run in the same tree stops at once and names the process already running: the gate's self-test plants violations in the tree, and two runs would refuse each other's. A run killed outright leaves its lock and what it planted; the next run takes the lock over and removes those files, unstaging the one the self-test stages.

**A commit for the project is the product's, and the gate checks it.** A clone that commits under the product's name—`git config user.name` set to *Lyonoyl Mail*—has to commit at the address the product has committed under before, with commit signing turned off. The address is the one real thing involved, so it is not written here: the gate reads it from the history, and `LYONOYL_COMMIT_ADDRESS` in the environment names a different one when it has to change. Any other name is a contributor's own, and the gate does not ask about it; a pull request lands as one commit authored by the product.

## The log

Every run writes what happened—the journal, as the code calls it—into a `logs` folder in the data folder, beside the `databases` folder that holds `profile.sqlite3` and `accounts/`. One file per run, named for the moment it started and the process, rolling at 8 MB and stopping after eight of them; twenty runs or 64 MB are kept, whole runs at a time, and the oldest go first.

```
YYYY-MM-DDThh:mm:ss.mmmZ      312 detail  body.rendered kind=html held=0 toFetch=0 blocked=2 notShown=9 links=14 truncated=no bytes=~65536 took=3ms
```

The moment in UTC, milliseconds since this run began, how much it is, the event, then its fields. `awk '{print $4}'` over a file is a list of everything that happened.

**Finding it** is a button rather than a path to remember: Settings → Storage → *Show the log folder* opens it in whatever the system uses to look at folders. No path crosses the boundary in either direction—the window asks for the folder, and which one it is belongs to the core.

**Three levels.** `trouble` is somebody losing something, `worth` is a milestone of the application—an update offered—and `detail` is one line per item: a message opened, a pass, and what it cost. Every build writes `worth`, so nothing about one message is written down unless somebody turns it up. Turn it up either way:

```bash
LYONOYL_LOG=detail npm run app     # a terminal
touch "<data directory>/logs/detail"   # a person who has never opened one
```

The file is read at startup, which is also the only way to turn it up on the run where the profile will not open—and that is the run where it is wanted. It stays up for as long as the file is there, and removing it turns it back down.

**Why updating mail takes as long as it does is a question the journal answers.** At `detail`, every pass records what it cost in `took`—`saidHello` of it opening connections, and `inList`, `inFolders`, `inBackfill`, and `inBodies` by phase—beside `opened`, which is how many times the application said hello to a server during it, split by phase the same way in `onList` to `onBodies`. A round records its own total and how many accounts it read at once. `scripts/where-the-time-went.sh` reads the newest journal and prints them; it takes `LYONOYL_LOGS` for a directory somewhere else.

`opened` is the number that decides the rest. The rule it exists for is in [the response budgets](../performance/response-budgets.md): a unit of work may not carry its own connection. A pass over six folders that says `opened=7` is opening one per folder; one that says `opened=57` is opening one per message as well.

**What a record may say is decided by the compiler, not by care.** Everything a field can carry is in `src-tauri/src/ports/journal.rs`, and none of it holds a string: there is nowhere to put an address, a subject, a filename, or the text of somebody else's error, so none of them can arrive. Sizes are rounded to a power of two, because an exact length says which message was opened. `nothing_a_message_wrote_reaches_the_journal` renders the whole corpus with the journal turned up and searches the file for anything those messages said, through base64 as well as plain.

**`log` and `tracing` are silenced at compile time**, and that is a security measure rather than tidiness: `async-imap` writes the request line it is about to send at trace level, which is `LOGIN` with the password on it, and the raw response, which is the message. Both crates are named in `src-tauri/Cargo.toml` with `max_level_off` and `release_max_level_off` so the macros expand to nothing, which no subscriber, environment variable, or dependency can undo. The test in `lib.rs` is what fails if the features are ever dropped.

## Asking a webview engine what it does

Three of the four layers message rendering rests on are properties of an engine this project did not write, and two of them cannot be checked by looking at a window: whether the policy served with a message is **enforced** looks identical to whether it is ignored, until something reaches the network. [The platform differences](../platforms/differences.md) say, row by row, which of the engines' answers were measured and which are only reasoned.

The probe asks the engine instead of watching it. It renders one message through the real scheme handler, and that message carries three pictures: two that traveled inside it, which the frame must fetch, and one pointing at a socket in this process, which the frame must not. Everything the frame asks for is counted; anything arriving at the socket is the policy having been ignored.

```bash
cargo build --package lyonoyl          # a debug build: the probe does not exist in a release one
LYONOYL_ENGINE_PROBE=1 ./target/debug/lyonoyl
```

**Close the application first.** One instance of this build runs at a time, so a probe started while the app is open exits at once—no window, no report, no line saying why. An empty answer here is that, and not a measurement.

**A real message instead of that one.** Give the variable a path or the name of a message in `tests/fixtures/eml/` and the probe renders that message through the whole of the real path—the sanitizer, the fetcher, the network, the store—and reports what the window then asked for:

```bash
LYONOYL_ENGINE_PROBE=tests/fixtures/eml/rating-stars-deep-in-tables.eml ./target/debug/lyonoyl
```

**Why one picture is not on screen** is said out loud by a development build, at the moment it is decided: whether nothing arrived, whether the host answered with a kind this build does not show, or whether the bytes were not the picture the header claimed. No address and no host appears in it—those are written by whoever sent the message. Run `npm run app` from a terminal and the lines are there among the rest.

`longest_wait_ms` is the other one. The probe hands the main thread a piece of work every fifty milliseconds while the engine lays out a message sitting on the bound this build allows, and records how long the longest one waited. A window that has stopped answering is a person with no sidebar, no list, and no way to leave the message—and from outside that looks exactly like a message that is merely slow. Counting how many pieces arrived would not see it: work handed to a blocked loop is not lost, it queues and runs late.

`parts_missed` is the number worth having: a picture the frame asked for and the store no longer had. Zero is what an ordinary message must give. Anything else is a message rendering with holes in it, and it is the number that tells that apart from a picture that was never fetched at all. A development build also says the same thing on standard output the moment it happens, so `npm run app` shows it without a probe.

What it shows the engine is one message carrying everything a policy has to hold back at once: two pictures that came with it, one pointing at a socket in this process, a stylesheet reaching for that same socket by `@import`, `@font-face`, and a background, and a link. `paths_that_left` names whichever of them got out, so a failure says which way rather than only that there was one.

It opens a window, waits ten seconds, writes `engine-probe.json` into the working directory, and stops. The exit code is the same answer in one number: `0` all is well, `2` **the policy was not enforced**, `3` the frame never fetched the message, `1` the probe could not run. The probe ends the process itself, because `AppHandle::exit` does not carry the number on Linux.

What each engine answered is in [the platform differences](../platforms/differences.md#showing-a-message-and-what-each-engine-actually-does).

`pages_asked_for` counts every request the frame makes of the scheme—the document's, and each picture's, which is looked up as a page first—so it runs ahead of `parts_asked_for`, which counts the pictures alone. The page the probe puts the frame in comes from the same scheme and is counted with neither, and whether the frame loaded the message is asked of the message's own name. Both kinds of name answer every request until their body is closed: WebView2 asks for a picture twice where the others ask once, and a name that answered only once would leave the picture missing there.

**Make it fail before believing it.** Every answer but one checks itself: the frame loaded, the picture was fetched. "Nothing reached the socket" does not—an engine that refuses that fetch for a reason of its own reads exactly like a policy that held. So weaken the policy the frame is served with in `src-tauri/src/infrastructure/html/serving.rs` to `img-src * lyomsg:`, `font-src *`, and `style-src *`, rebuild, and run it again: the socket must ring, `paths_that_left` must name the way out, and the exit code must be `2`. Weakened, all three engines fetch all four. Then put the policy back. `img-src *` alone is not that weakening: it does not name the scheme, and [the platform differences](../platforms/differences.md) have the run where it came back quiet for the wrong reason.

**Run it in a virtual machine over SSH.** It needs a desktop session, so the machine has to have one open—but nobody has to look at it. On Ubuntu that is a graphical login; on Windows an interactive session. Copy the report back and it is the platform's row, measured.

An SSH shell is not inside that session and has to be handed its environment, or the window has nowhere to open:

```bash
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export WAYLAND_DISPLAY=wayland-0     # DISPLAY=:0 on an X11 session
export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u)/bus
```

**It is a debug build only.** `#[cfg(debug_assertions)]`, and `cargo check --release` proves it compiles out.

### Setting up a machine to measure on

Three scripts build the machines the other two engines run on, and [the virtual machines guide](../platforms/virtual-machines.md) is the whole procedure step by step—what is free, what goes from where to where, and what has to be true before anything can be driven from outside the guest.

| | Run it | What it does |
| --- | --- | --- |
| `scripts/prepare-host.sh` | On your machine | Makes the key the guests will trust, and gathers the script and the public key into a folder and an `.iso` to carry in |
| `scripts/prepare-linux-vm.sh` | Once, inside the Linux guest | SSH with that key, the Tauri build dependencies, Rust, Node |
| `scripts/prepare-windows-vm.ps1` | Once, inside the Windows guest, elevated | SSH with that key, and PowerShell as the shell a session is given |

Each guest script checks every step after taking it and stops at the first that did not hold, and each is safe to run again. The first one has to be run by a person at the guest's own keyboard: until it has, there is no way in—which is what it is for.

## A component from shadcn/ui

The interface is built on shadcn/ui, and shadcn/ui is copied in rather than installed: its components are in `src/shared/ui/primitives/`, kept as its command wrote them, and the three variants they are styled through—`data-open`, `data-closed`, `data-disabled`—are in `src/index.css`. The command is not a dependency of this project, so a new component is added by running it from the registry:

```bash
npx shadcn@latest add <component>
```

It writes into `src/shared/ui/primitives/`, as `components.json` says. If it adds `shadcn` to `package.json` or `@import "shadcn/tailwind.css"` to `src/index.css`, take both out again: a variant the new component uses and `src/index.css` does not hold yet is copied there from that stylesheet. The notice for everything copied is `src-tauri/licences-copied/shadcn-ui.md`, and the license list `scripts/preflight.sh` rebuilds carries it.

## Signing, and why it is not only for releases

**The keychain is the reason a development build is signed**, and not distributing anything. An unsigned build is named to the system by the hash of its own binary, so every rebuild is a new application, the keychain stops recognizing it, and it can end up unable to read the key its own databases are encrypted under. Signed, it is named by the bundle and the certificate instead, and that name does not move. [The platform differences](../platforms/differences.md) carry the measurement.

`scripts/holds-still.sh` is that property as a check: it asks what a build is named by and refuses one named by a hash. Everything below runs it, because "it is signed" is a different claim—a signature can still leave the hash in the name.

**`npm run app` signs what it launches**, which is where this matters most—a session of hot reloading is a rebuild every few minutes, and an unsigned rebuild is a new application to the keychain. Tauri only signs when it bundles and `tauri dev` never bundles, so the build and the launch are taken apart by a runner: `scripts/dev-runner-macos.sh` builds, signs, and then launches. Tauri is handed a process either way.

That runner is a shell script and the keychain it exists for is macOS's, so `scripts/dev.mjs` hands it to Tauri on macOS and nowhere else—Windows and Linux get an ordinary `tauri dev`. On macOS without a certificate set up, the runner says so at every rebuild and carries on as an ordinary `cargo`: a development session is not worth stopping over.

**The identifier is the part that is easy to get wrong.** A bare binary's identity is its file name unless it is told otherwise, so a signed `lyonoyl` would be a *different* application from `Lyonoyl Mail.app` and would need its own permission. The runner names it with the bundle's own identifier, read from `tauri.conf.json` so the two cannot drift apart, and then both carry the same rule byte for byte—one answer covers everything.

**`npm run app:dev-build` is for running the real bundle**, signed the same way. Notarization is what lets *other* machines open the application; this build is for this machine, so there is no App Store Connect key, no network, and no waiting on Apple.

**`npm run app:release` is for a build somebody else opens.** Gatekeeper refuses an application that is not notarized on every machine that did not build it, so this one signs, notarizes, staples, and then asks Gatekeeper what it made.

**Nothing about the identity is in this repository, and that is invariant 21 rather than an oversight.** No certificate, no team, no account, no key. `tauri.conf.json` names no signing identity, so a plain checkout builds unsigned and says so. What the build needs it reads from a file kept outside the tree:

```
~/.config/lyonoyl/signing.env      chmod 600
```

`LYONOYL_SIGNING_ENV` names another path for it. Each value in single quotes—a certificate name holds spaces and parentheses, and a line that does not quote it is a shell syntax error about a parenthesis. The first is all a development build needs; the next three are what notarizing a release needs on top; the last two are the update key, which `scripts/make-update-key.sh` writes there itself. Every build made with the file compiles the public half in, a development build as well, so a development build asks about updates like any other copy, and a signed one is offered a newer release once there is one:

| Name | What it is | Needed for |
| --- | --- | --- |
| `APPLE_SIGNING_IDENTITY` | The Developer ID Application certificate, named in full | Both |
| `APPLE_API_ISSUER` | The issuer of the App Store Connect key | A release |
| `APPLE_API_KEY` | That key's identifier | A release |
| `APPLE_API_KEY_PATH` | Where the `.p8` file is, also outside this tree | A release |
| `LYONOYL_UPDATE_PUBKEY` | The public half of the update key, compiled into every build made with this file | An update |
| `LYONOYL_UPDATE_KEY_PATH` | Where the secret half is, also outside this tree | An update |

The notary is asked with an App Store Connect key: one file, carrying no account password, revoked on its own without touching anything else.

### The order

**For a signed development build**, which is the one wanted day to day:

1. **A Developer ID Application certificate**, requested from the developer portal and installed in the login keychain with its private key. `security find-identity -v -p codesigning` lists it.
2. **Write `~/.config/lyonoyl/signing.env`** holding `APPLE_SIGNING_IDENTITY` and nothing else, in single quotes as above, and `chmod 600` it.
3. **`npm run app` or `npm run app:dev-build`.** Both sign. The second refuses before building if the file is missing, unreadable, or names a certificate this machine does not hold, and afterwards checks that what it built is named by the certificate rather than by a hash; the first warns at every rebuild and carries on as an ordinary `cargo` when the file is missing or unreadable, and stops with what `codesign` said when the certificate it names cannot sign.
4. **Let the signed build read the secrets the unsigned builds stored, once.** Their rule names the build that wrote them, and a signed application is a different one, so the first signed run is asked about each item exactly as a rebuild is. Answer Always Allow: the rule then names the signed build, which holds still from one build to the next. Keep the database key's item rather than removing it. Removed, it is not made again over the profile it opened: the next start offers Start fresh, which sets that profile aside with its accounts, settings, and drafts, and only the start after it makes a new key, over no databases.

**For a release**, the certificate in step 1 is the organization's Developer ID Application certificate, and to that add:

5. **An App Store Connect key** with the Developer role, downloaded once as a `.p8`. Apple does not offer it a second time. Put it outside this tree and `chmod 600` it.
6. **Add the other three values** to the same identity file: `APPLE_API_ISSUER`, `APPLE_API_KEY`, and `APPLE_API_KEY_PATH`. No team identifier: the notary is asked with the key, and the key carries the team. It is read only when notarizing as an Apple ID and a password, which this does not do.
7. **`npm run app:release`.** Same refusals, plus a key that is missing or readable by more than its owner. Then the preflight, the build, the signature, the notarization, and the staple.
8. **It opens what it made and asks Gatekeeper about that**, rather than assuming: the disk image is found wherever the build put it, its own signature verifies, and then it is mounted so every question is asked of the application a person would drag out of it—named by the certificate rather than by a hash, signed, carrying no path of the machine that built it, hardened runtime on, a notarization ticket stapled to it, and accepted by `spctl`. Any of those failing fails the run.

   The stapled ticket is the one that earns its place. Tauri skips notarization with a warning and exits zero when the credentials are missing, so a forgotten secret otherwise produces a green build and an application every other Mac refuses to open.

A *Developer ID Installer* certificate is for `.pkg` files and is needed for neither—a `.dmg` is signed with the Application one.

## An update

**A copy updates itself only from what this project sealed, and only after its user said yes.** The first start asks, on the last step of the onboarding, whether to look for a newer version once a day, with the box clear; the answer is written when the onboarding is finished, and the settings change it later. The look names the product and not its version. Nothing is fetched before that answer is written. A build made without the update key never looks, whatever the answer.

**A copy asks one address, the project's site, `lyonoyl.com`**, for the files a release lays out below. The site answers with each file or redirects the copy to wherever it keeps it, over HTTPS and in a chain of at most ten addresses, and how it does that is its own business. The site is trusted with nothing but being there: what installs is decided by the seal, the digest, and the Developer ID.

What a copy fetches is two small files—`updates/v1/macos/latest.json` and its seal, `latest.json.minisig`—and, once a person presses *Install*, the archive the manifest names for their kind of Mac, from the release's folder on the site; a name that would leave that folder is refused before anything is fetched. Before it is unpacked, the archive is held to the size and SHA-256 the signed manifest names. It is unpacked in the application's own data folder, or in a hidden folder beside the running copy when that copy is on another volume, to at most a gigabyte, and must hold one application that meets the running copy's own designated requirement—the same identifier, the same team, Apple's certificate authority—which `codesign` checks over every file the bundle's signature seals. That application must also run on this Mac: code for the kind of Mac the copy runs as, and a system no newer than this one, read from its property list and its executable's header. A copy whose own requirement names no team is not one an update replaces.

### The key, once

```
scripts/make-update-key.sh
```

It makes the key outside this tree—`~/.config/lyonoyl/update.key`, locked with a password the tool asks for at the terminal—and adds `LYONOYL_UPDATE_PUBKEY` and `LYONOYL_UPDATE_KEY_PATH` to the identity file. Every build made from then on compiles the public half in, and **a copy takes an update sealed by that key and by no other, for as long as it exists**: a second key is a release every installed copy refuses, and so is a lost one. Keep the key and its password somewhere that outlives this machine. The script refuses to run over a key that is there, or one the identity file already names, and refuses a key that opens with no password.

**The team is fixed the same way.** A copy holds an update to the identifier and the team its own signature names, so a release signed by another team, or under another bundle identifier, is one every copy installed before it refuses.

### Every release

1. **`npm run app:release`**, from a commit, which ends with *Updates: on*. A release made without the key ends with *Updates: off*, and nothing made from it is ever updated in place.
2. **The notes**: one JSON object holding a non-empty text under the name of every catalog in `src/shared/i18n/locales/`, because a copy shows the one in the language its user chose. The script refuses notes that leave a catalog out or name a language that has none.
3. **`scripts/release-update.sh --notes <notes.json>`**, against the newest disk image or the one named after it, from the commit the image was built from. It asks the application in the image what Gatekeeper would—signature, stapled ticket, assessment—and that its signing requirement names Apple's authority and a team, that it is this tree's version, and that it carries the update key. Then it lays the release out under `target/site/`, the way the site serves it:

   | File | What it is |
   | --- | --- |
   | `releases/<version>/lyonoyl-mail-<version>-macos-<kind>.dmg` | What a person downloads |
   | `releases/<version>/lyonoyl-mail-<version>-macos-<kind>.app.tar.gz` | What a copy takes, with nothing of this machine in it |
   | `releases/<version>/lyonoyl-mail-<version>-source.tar.gz` | The commit it was built from, as git holds it |
   | `releases/<version>/notes.json` | What changed, in every catalog language |
   | `releases/<version>/SHA256SUMS`, `SHA256SUMS.minisig` | Every file's digest, sealed with the update key |
   | `updates/v1/macos/latest.json`, `latest.json.minisig` | The manifest, sealed |

   `<kind>` is `aarch64`, `x86_64`, or `universal` for an image carrying both, as Rust names them. The code every copy runs then reads all of it back—the seals, the manifest's shape, the sums and every file they name, what the folder holds besides, each archive's size and digest, the application inside and its signature against the one it was packed from, its version, and the kinds of Mac it holds code for. The key's password is asked for at the terminal, or taken from `LYONOYL_UPDATE_KEY_PASSWORD` where a password manager hands it over, and it leaves the environment before anything else runs.

   A release with an image for each kind of Mac is a second run with the second image, from the same commit, which adds its kind to the manifest and the sums of the same version. Whatever is in the release's folder is published and summed, so a file an earlier run left there is refused until it is gone.
4. **The files**: the site answering for `releases/<version>/` with every file in `target/site/releases/<version>/`, however it does it.
5. **`scripts/release-check.sh <version>`**, which brings every file back from the site through the client every copy uses and holds each to the sums sealed here. A site that answers a browser and refuses a copy—a redirect chain too long, a challenge in front of the files, a certificate a copy does not take—is found here rather than by every copy.
6. **The manifest, last**: the site answering for `updates/v1/macos/` with what is in `target/site/updates/v1/macos/`. A manifest out before its files sends every copy after a file that is not there. The manifest and its seal can be cached apart for a few minutes after they change, and a copy that asks in those minutes is told it could not check.
7. **`scripts/release-check.sh <version>` once more**, which says the manifest is out.

**What the site must do** is answer for each of those paths over HTTPS, with the file itself or by redirecting the copy, in a chain of at most ten addresses, the first included, and with no bot check or challenge in front of it: a copy is not a browser. Whatever publishes to the site—a token, an account—is kept outside the tree, as the signing identity is.

**A version is published once.** No file of a release is replaced or deleted afterwards; a mistake is fixed by the next patch version. Running the script again makes the same archive and the same source byte for byte, so files already uploaded stay valid; what is new is the seals, and they travel with the sums and the manifest.

**What the read-back cannot ask is the copies already out there.** It holds the application to the requirement of the one it was packed from, so a release signed under another identifier or team passes it, and is refused by every copy installed before it—the paragraph about the team above is the whole of the defense.
