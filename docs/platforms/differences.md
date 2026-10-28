# Platform differences

Lyonoyl Mail ships on macOS; Windows and Linux are built from source. This page records where the three differ, what the code does about it, and which differences are unverified.

Verified means someone ran the application and watched the behavior, or the engine probe ran and reported a number. Reasoned means the platform API was read and the call is the right one, but nobody has run it there.

## What is supported

| | Supported | Built and measured on | What a release carries |
| --- | --- | --- | --- |
| macOS | 13 Ventura and later | a Mac with Apple Silicon | a signed, notarized `.dmg` |
| Windows | 11 | Windows 11 on arm64, a virtual machine | the source, built there |
| Linux | Ubuntu 24.04 LTS | Ubuntu 24.04 on arm64, a virtual machine | the source, built there |

What this page marks verified was run on these three. A release is the macOS build and the source; Windows and Linux are built by whoever runs them, and only what this page records has been checked there.

**Windows 11 rather than 10**, because WebView2 ships with the system there. On Windows 10 it is a runtime a person has to have, and the frame a message is shown in is the whole of how a message is rendered.

**Ubuntu 24.04 LTS is the reference, not the definition of Linux.** Another distribution works if it has these, and a person can check for themselves rather than look for their name in a list:

| Needs | Why |
| --- | --- |
| WebKitGTK 2.44 or newer (`libwebkit2gtk-4.1`) | The engine every question on this page is about |
| A Secret Service provider, or the master password | Where account passwords and the key the mail is written under live. The master-password path is reasoned |
| An XDG portal backend, or Zenity | The file dialog is a portal over D-Bus, and the dialog library falls back to Zenity where there is none; without either there is no dialog |
| glibc 2.39 or newer, for the `.deb` | What 24.04 builds against. Below it the package does not install; the `.AppImage` carries more of its own |

**What the packages are told, and where.** `npm run app:build` makes a bare binary on Windows and Linux, and nothing here makes or checks a package; these are the fields `tauri build` reads for whoever bundles one. There is no "minimum Windows version" or "minimum Linux version" to set—those floors are the engine and the libraries, not a field. What `tauri.conf.json` does carry:

| | Set to | Why |
| --- | --- | --- |
| `macOS.minimumSystemVersion` | `13.0` | Otherwise the floor is whatever the Rust target defaults to—a side effect standing where a decision belongs |
| `windows.webviewInstallMode` | `skip` | The default downloads WebView2 from Microsoft **during installation**. On Windows 11 it is part of the system, so the installer does not reach the network before the application has been started once. The cost: on a Windows without it—a stripped or LTSC build—the application does not start and the installer does not say why |
| `linux.deb.depends` | the portal, a GTK or KDE backend, Zenity | The file dialog is an XDG portal over D-Bus, with Zenity where there is none |
| `linux.rpm.depends` | the portal and Zenity | The same |

`macOS.hardenedRuntime` is on by Tauri's default, which is what notarization requires, and `scripts/release-macos.sh` refuses a build without it.

**What the application weighs.** A release build for Apple Silicon is **27.5 MB** on disk, the whole `.app` bundle counted. Taken with `npm run app:build`, which is the same build the installers are cut from. The other two systems are not measured here.

## Where data is kept

| Platform | Folder | Base |
| --- | --- | --- |
| macOS | `~/Library/Application Support/Lyonoyl Mail/` | data directory |
| Windows | `%LOCALAPPDATA%\Lyonoyl Mail\` | **local** data directory |
| Linux | `~/.local/share/lyonoyl-mail/` | `$XDG_DATA_HOME` |

The databases are in a folder of their own inside it, `databases`—`profile.sqlite3`, `accounts/`, and on a Linux desktop with no keyring the file of secrets—beside the journal's `logs` and the updater's `updates`. A start that finds them at the top of the data folder moves them into it, the whole set or nothing.

Windows uses the local folder rather than the roaming one on purpose: the mail index reaches gigabytes, and a roaming profile copies its contents over the network at every sign-in. SQLite in WAL mode can also corrupt a database on a network share.

Linux keeps the lowercase hyphenated name its file managers and shells expect.

## How many files the application may hold open

Every account holds its database, the database's log, and its connections to the server, and the system's own libraries open their files out of the same allowance. Before anything opens a file, the application raises its soft limit to 10,240, or to the hard limit where that is lower, and leaves a limit that is already higher as it is.

| Platform | Started from | Soft limit at the start | Status |
| --- | --- | --- | --- |
| macOS | Finder, the Dock, or `open` | 256 | **verified** |
| macOS | a terminal | the shell's own: 1,048,576 on the machine it was measured on | **verified** |
| Linux | a desktop session | 1,024 under systemd's defaults | reasoned |
| Windows | anywhere | no limit of this kind, and nothing is changed | reasoned |

When the limit runs out, AppKit, Metal, and the keychain cannot read their own files, and the application stops before its window is drawn. A run from a terminal, which is how development and the tests run, starts far above 256 and never reaches it, so the test of this starts the code again under the 256 a launch from Finder gets.

## Only one copy at a time

Opening the application while it is already open brings the existing window forward rather than starting a second copy. The same on all three, through `tauri-plugin-single-instance`: a named mutex on Windows, a name on the D-Bus session bus on Linux, and a socket in `/tmp` on macOS.

**Verified** on macOS. Reasoned elsewhere: the mechanism differs per platform and only the plugin knows the difference, so what has been watched here is the behavior, not each implementation.

## Window position and size

The window stores its position, its size, and its mode (normal, maximized, full screen), and is **made** that way rather than made and then changed. A window created at one size and moved to another afterwards is a window a person watches change, and on macOS both the move and the maximize are animated. Asking for all three as the window is built leaves nothing to animate.

The size is read as the inner size and written back the same way, so nothing drifts on any platform: Windows measures the client rectangle, Linux asks GTK for the window size, macOS the content view.

| Platform | Position | Size | Mode | Status |
| --- | --- | --- | --- | --- |
| macOS | `NSWindow` frame | content view | full screen reported as maximized as well, so it is checked first | **verified** |
| Windows | `GetWindowRect` / `SetWindowPos` | client rectangle | distinct states | reasoned |
| Linux (X11) | `gtk_window_move` | GTK size | distinct states | reasoned |
| Linux (Wayland) | **not possible** | works | works | reasoned |

### Wayland cannot place its own windows

The protocol gives a client no way to learn its absolute position or to move itself; the compositor decides. GTK accepts the move request and ignores it, and the position it reports is whatever it last cached. Nothing fails and nothing is lost: the size and the mode are restored, the position is not. There is no fix at the application level—every Wayland application behaves this way.

### Mixed DPI on Windows

Coordinates are stored in logical units, and the divisor between logical and physical differs per monitor. A window left on a 150% secondary monitor can therefore be restored to the wrong place when the primary monitor is at 100%.

The rule that discards geometry landing outside every connected screen keeps the worst case away—the window never opens out of sight—but it can open on the wrong monitor.

### The settling window after a restore

A window manager answers a size request with what it is willing to grant, and that answer arrives as a resize event. Events are ignored for 500 ms after the restore so the answer does not overwrite the size the user chose. On a heavily loaded machine, or with a slow compositor, a correction could arrive later than that and be stored. If a user reports "the window keeps shrinking," this is the first place to look.

## The window's own bar

| Platform | The bar | Status |
| --- | --- | --- |
| macOS | none: the content reaches the top and the buttons float over its corner | **verified** |
| Windows | the system's own | reasoned |
| Linux | the system's own | reasoned |

macOS puts the application's name in the menu bar, so a strip inside the window repeating it is a strip of mail nobody can read. The title bar is therefore taken away there and the content given the height back, with only the top-left corner moved down out from under the close, minimize, and zoom buttons.

Nowhere else. On Windows and Linux the title bar is how a window is moved, closed, and told apart in a task switcher, and an application without one behaves unlike every other one on the desktop.

Which of the two the interface is in is read from the user agent, because that is the one thing available before anything else loads. Being wrong about it costs a few pixels of padding rather than anything a person notices.

## The count on the application's icon

What nobody has read, across every account's inbox, drawn on the icon the system shows in its Dock or taskbar. Zero takes the badge off rather than drawing a 0.

| Platform | How | Status |
| --- | --- | --- |
| macOS | `set_badge_count`—a number on the Dock icon | reasoned |
| Linux | `set_badge_count`—the same call, where the desktop supports a launcher badge | reasoned |
| Windows | **no count**: the taskbar takes a picture laid over the icon, so a number there means drawing one into an image at the size it asks for. Nothing is drawn | reasoned |

Every row is reasoned rather than measured: the count that reaches the call is covered by tests, and nobody has watched an icon change on any of the three.

The number is read from the same query the sidebar's per-account counts come from, so the icon and the rows under it cannot disagree. It is redrawn when the application starts, when a pass reports a change, and after every change a person makes, not on a timer: the query walks every account's inbox.

## The picture the application shows while it runs

The kitten on one of nine backgrounds, chosen on the last step of the onboarding or under Appearance in the settings—Ember, the one it is built with, until somebody picks another—and the same picture whether the window is light or dark. What the system draws from the application's own files—Finder, the Start menu, a pinned icon while the application is closed, a notification—stays the picture it was built with, whichever is chosen.

| Platform | How | Status |
| --- | --- | --- |
| macOS | `setApplicationIconImage` on the shared `NSApplication`, on the main thread—the one `unsafe` call in the code | **verified** |
| Windows | `set_icon` on the main window, which its taskbar button shows as well | reasoned |
| Linux | **not offered**: a desktop shows the picture of the application's desktop entry wherever it finds one, rather than its window's | reasoned |

It is put on when the application starts and after every change to a setting.

## The keychain prompt on macOS, and why it is a development-build problem

A macOS keychain item remembers which application created it, by that application's **code signature**, and asks the user before letting anyone else read it. A signed application reading back its own secrets is not asked at all—that is the normal, quiet case, and it is what a released build does.

A development build is not that. `cargo build` produces an ad-hoc signature with no team identifier and an identifier that carries a hash of the binary:

```
Identifier=lyonoyl-72f66b62bab0efc6
Signature=adhoc
TeamIdentifier=not set
```

The hash changes with every rebuild, so macOS sees a different application each time, the stored item's access list never matches, and the dialog appears again. **Always Allow** adds the current binary to the list and holds until the next Rust rebuild; a change to the interface alone does not rebuild it.

Two rules follow:

- **Secrets are created by the application, never by a test binary.** An item written by `cargo test` belongs to the test binary, and the application is a stranger to it from the first read. The local development accounts are added through the ordinary account form, like any other.
- **Signing is what removes the dialog**, and a certificate is what makes a signature stable. Nothing else does: keeping secrets outside the keychain would trade a dialog for the one thing this product promises not to do.

**A certificate is what makes the requirement stop moving.** Measured: signed with a certificate rather than ad-hoc, the designated requirement `codesign` records against a keychain item names the certificate instead of the binary, and is **byte for byte the same after a rebuild**—no `cdhash`, nothing derived from the bytes. That is the whole of why signing removes the dialog, and it holds for any certificate the system trusts for code signing. `scripts/holds-still.sh` is the same question asked of a build, and every signed build here runs it: `npm run app` with a certificate set up, `npm run app:dev-build`, and `npm run app:release`.

**How a build is signed** is in [the development guide](../development/guide.md#signing-and-why-it-is-not-only-for-releases), and the identity it signs with is kept outside the tree. On a checkout nobody has set up, a development build is ad-hoc and asks as above.

**A development account uses the keychain exactly like any other.** There is no bypass and no constant password: `SystemCredentialStore` treats every account the same, so everything above applies to the three local accounts as much as to a real one.

**The keychain API decides whether anyone is asked at all.** macOS keeps a password behind a list of applications allowed to read it. `SecItemAdd`—the modern API, and the one the `keyring` crate uses—stores an item with **no such list**, so every read asks the person, whatever the application is signed with and whether or not the keychain is unlocked. `SecKeychainAddGenericPassword` records the application that created the item, and reading back what you wrote yourself is then silent.

So on macOS the store is `security-framework` directly rather than `keyring`. A secret written through the modern API is invisible to the older API's search, so a read that finds nothing there falls back to the modern one, moves the secret across, and never has to ask about it again. Measured: writing and reading a secret takes about a second, with no dialog—`cargo test --lib a_secret_this_application_wrote -- --ignored`.

**A password stored by another build is typed once more.** An item created by a build that no longer exists cannot be read quietly by this one, and no routine fixes that from the outside—a routine that rewrites the entries is itself a test binary, which is the first rule above. Adding the account again asks for the password, stores it from the running application, and the question stops: `AddAccount` recognizes an account that is already there but has no readable password and replaces the entry instead of refusing.

**Where a binary sits decides how long the keychain takes to identify it.** Measured with the same binary and the same signature, run from two places:

| Run from | `SecKeychainAddGenericPassword` |
| --- | --- |
| `target/debug/`—where the application is | 18 ms |
| `target/debug/deps/`—where test binaries are | 30 s |

`deps` held 672,356 files. Identifying an executable by its path means the system looking at the directory holding it, and in one that size it takes half a minute. Nothing about the signature, the certificate, or the item's access list is involved: strip the signature and it is still 30 s there and 18 ms one level up.

**So a keychain measurement taken from `cargo test` is not a measurement of this application.** From where the application actually runs, taking a handle costs 45 µs, finding an item 3.7 ms and writing one 18 ms. The `#[ignore]`d tests that reach the real store are still worth having, and what they say about *time* has to be read knowing where they ran from.

**A dialog nobody sees does not stop the mail.** It can be behind another window or on another desktop, and a read that waits for it waits forever: the synchronization round stops, every account with it, and nothing on screen explains why. So a read gives the store twenty seconds and then treats it as unavailable—long enough for someone who sees the dialog to answer, short enough that one they do not see costs a round rather than the day. Storing a secret has no such limit: that happens because a person asked for it, and they are there.

**The key the databases are written under is still listened for once those twenty seconds pass.** The window comes up saying the key could not be had, and the read goes on: a person who answers the dialog after that has the databases open the moment they do, and the window reads everything again, with nothing to quit and start again.

Windows and Linux do not behave this way. Credential Manager grants any process of the same user access to its own entries without asking; Secret Service asks once to unlock the collection, per session rather than per application.

## Where secrets go, and what happens when there is nowhere

macOS and Windows always have a store: Keychain and Credential Manager. A Linux desktop may have no Secret Service at all—a minimal window manager, a server session, a container—and then there is nowhere to put a mailbox password.

| Platform | Where secrets go | When the store is absent | Status |
| --- | --- | --- | --- |
| macOS | Keychain, through the older API that records the creating application | cannot happen | **verified** |
| Windows | Credential Manager, through `keyring` | cannot happen | reasoned |
| Linux | Secret Service, through `keyring` | a master password and a file beside the databases | reasoned |

Which one a device gets is decided by the first start that writes anything, by reading an entry that was never written. A store that is there answers "no such entry"; one that is not there fails to be reached at all. Two answers count as absent—no store configured for this build, and a platform that could not answer—and everything else counts as present, **including a store that exists but is locked**. That asymmetry is deliberate: asking someone whose keyring is merely locked for a master password would put their secrets in a file while their system store sits there, which is exactly the confusion that is forbidden.

**Once something is written, what is beside the databases decides, and nothing is asked.** The key the databases are written under stays in the store it was made in. A file beside them is where that key is, and it stays their store once a keyring answers. Databases with no file beside them were written under a key a keyring holds: when that keyring does not answer, the window says to start or unlock it and offers Start fresh for one that is gone for good, and no file is made—it would hold everything but that key. No store that answers that it holds no key for databases already written is given a new one, whichever store it is: a keyring whose own database is locked answers that it holds nothing, the item can sit in a keychain that has dropped out of the list macOS searches, and a key made then is kept in front of the one the databases were written under while opening nothing. A key is made only by a start that finds no databases. Where the key may come back, the window says it could not be read: a store that would not hand it over, a Linux keyring that says it holds nothing, and a file of secrets. The macOS keychain and Credential Manager are always there, and an answer from either that it holds no key is not waited out: the window shows the databases as files that will not open and offers Start fresh beside them. Start fresh sets the databases aside and leaves the file where it is, so a device with a file keeps it and a device without one is asked again at the next start.

The file itself is Argon2id at 256 MB of memory over the master password, and one sealed block underneath a header that names that cost—sealed together with it, so a file claiming a cheaper derivation is refused rather than opened. Mode `0600`, written through a neighbor and a rename. Nothing is created until a master password is chosen, and the file is closed again at every start.

The file, the derivation, and the classification are covered by tests; the rows are reasoned because a real Secret Service that is absent, present, or locked has not been run on a Linux desktop.

## Asking where a file goes

Saving a message hands it to the system's own panel, and the three systems disagree about how one is raised.

| Platform | The panel | Backend | Status |
| --- | --- | --- | --- |
| macOS | `NSSavePanel` | AppKit, and **only from the main thread** | **verified** |
| Windows | `IFileSaveDialog` | Win32 | reasoned |
| Linux | the desktop portal's file chooser | XDG portal over D-Bus, out of process | reasoned |

**What is saved carries the mark of a download on macOS and nowhere else.** The file is marked `com.apple.quarantine`—`0081`, the moment, *Lyonoyl Mail*—through the file itself as it is written, which is what Gatekeeper reads before anything in it runs; on a FAT or exFAT disk the mark lives in a `._` file beside it. **Verified** on this Mac: the adapter's tests read it back, and on a FAT32 and an exFAT disk image it is in the `._` file beside the saved one. Windows keeps its mark in a second stream, and nothing here writes one; Linux has no such mark.

**Only from the main thread** is not a preference. AppKit draws a panel on the main thread alone: the dialog library hops there itself while the application runs and panics where it does not, and the other two systems are forgiving by accident rather than by design. So the panel is raised on the thread the event loop runs on, and the command that wants an answer waits on a blocking thread. Which leaves one way to hang the application—asking from the event loop's own thread, so that it waits for itself—and that thread's identity is recorded at startup, where it is known, and compared before anything is raised.

**Linux takes the portal rather than the toolkit.** The library offers a GTK backend as well, and this process already runs a GTK event loop of its own; a second one nested inside it is a class of problem worth not having. The portal is a separate process reached over D-Bus, so there is no toolkit reentrancy at all. The cost is a runtime dependency: a Linux package pulls in a portal backend, and Zenity, which is what the library falls back to when a portal is not there.

## Logging out

Quitting, closing the window, and a restart reach the run callback in `src-tauri/src/lib.rs` on all three, and that callback is where the message being written is kept on the way out. Logging out reaches it where the system tells the application it is ending: macOS calls `applicationWillTerminate`, as it does for Cmd+Q, Windows sends `WM_ENDSESSION`, and tao ends its loop for either, which reaches the callback as `RunEvent::Exit`.

A Linux desktop tells this application nothing. tao registers its GTK application with no session manager, so nothing asks it before the session ends, and the session then takes the process down—with the display it draws on, or with a signal nothing here handles—without the callback being run. Logging out there keeps only what was last saved of the message being written, and nothing else the callback writes on the way out is written either.

| Platform | Logging out reaches the run callback | Status |
| --- | --- | --- |
| macOS | through `applicationWillTerminate` | reasoned |
| Windows | through `WM_ENDSESSION` | reasoned |
| Linux | **not at all** | reasoned |

## Showing a message, and what each engine actually does

A message body is served on its own scheme with the policy in the response header and embedded in `<iframe sandbox="">`. Every part of that is engine behavior, and the three engines are three different programs.

**Any system can answer for itself.** The engine probe—how to run it is in [the development guide](../development/guide.md#asking-a-webview-engine-what-it-does), and [the virtual machines guide](virtual-machines.md) builds the machines—writes the rows on this page—whether the frame loaded, whether the picture was fetched, whether the policy was enforced. Somebody running a distribution not listed here can run the probe and send back its row.

**How the rows below are answered.** A probe renders one message through the real scheme handler and counts what the frame asks for, with one picture pointed at a socket in this process that the policy must forbid—so "the policy is enforced" stops being a thing to believe and becomes a number.

The message it renders carries three pictures—one that came with it, a drawing, which is the kind an engine may treat differently, and one pointing at the socket—a stylesheet of its own that reaches for the same socket four ways, and a link. That row about the stylesheet matters more than the others: a `style=` attribute is filtered declaration by declaration, while a message's own `<style>` is kept as written, so there are two barriers there and one here, and the one here is the policy.

**Four of the five reaches are exercised and one is not.** Weakened, all three engines fetch the `@import`, the `@font-face`, the background, and the picture; in place, none of them fetches any. The cursor cannot be made to fetch without a pointer over the element, so it is carried and not counted. The font is *applied* rather than only declared—declared and unused it is never fetched, and a fixture that does not create the condition it checks comes back quiet whatever the policy says.

**Nothing here clicks the link, and nothing can.** A document under `sandbox` has the sandboxed automatic features flag set, so it cannot even refresh itself to one—measured rather than assumed. The link is carried for the other half of that question: whether an engine ever leaves a message frame *without* being asked to. None does. Whether a link a person clicks is refused before it navigates is answered by a person. **Linux has not been asked that question**, and it stays unasked: Linux is built from source and not shipped ([the working contract](../../AGENTS.md), section 6). On Windows the answer is no, since the frame's navigation is not handed to the handler at all. All three engines answered the rest: the document loaded, the picture that came with the message was fetched, nothing reached the socket.

| What | macOS (WKWebView) | Windows (WebView2) | Linux (WebKitGTK) |
| --- | --- | --- | --- |
| A frame on a custom scheme loads at all | **verified** | **verified** | **verified** |
| The response-header policy applies to it | **verified**—the message's own styles apply and nothing else loads | **verified**—weakened, the picture left; in place, nothing did | **verified**—weakened, the picture left; in place, nothing did |
| A picture served from the same scheme is shown | **verified**—in the frame the reader sees, `<iframe sandbox="">`, whose origin is opaque | **verified**—the same, with the policy naming the scheme in both of its spellings | **verified**—same frame, same opaque origin |
| A link inside the frame is refused before it navigates | **verified by a person**—the message stays on screen and the window is asked; no probe can click | **no**—the frame's navigation is not handed to the handler at all, and that is a property of the dependency rather than an untested question | **not asked**—nobody has clicked one here, and Linux is not shipped |
| Nothing leaves the frame unasked | **verified**—the message carries a link and the engine never went to it | **verified**—the same | **verified**—the same |
| A stylesheet the message wrote cannot reach the network | **verified**—`@import`, `@font-face`, and a background at a live socket, all three refused; the cursor is carried and not counted | **verified**—the same three, the same way | **verified**—the same three, the same way |
| A costly message does not take the window's thread | **verified**—a message on the bound leaves the main thread's longest wait at 115 ms, and one at 46,000 elements with an expensive stylesheet at 113 ms | **verified**—36 ms on the bound | **verified**—21 ms on the bound |

**The probe opens the message inside the frame the reader gets.** The reader shows it inside `<iframe sandbox="">`, and an empty `sandbox` gives the document a **unique opaque origin**, in which `img-src 'self'` matches no URL at all—measured on WKWebView both ways: at the top of a window the picture is fetched, inside the frame it is not. So the policy names the scheme it serves on instead of leaning on `'self'`, which grants a message nothing, since `lyomsg` is not a scheme the sanitizer passes. All three engines are measured in that frame.

**A custom scheme has two spellings and a policy has to carry both.** It is `lyomsg://` on macOS and Linux and `http://lyomsg.localhost` on Windows, so `img-src lyomsg:` alone is a policy that names nothing on Windows—measured on WebView2, where the frame loaded and every picture in it was refused. The address is built in Rust, so the difference lives in one place; `tauri.conf.json` writes `frame-src` both ways, and the message's own policy does the same. A test holds both, because dropping either is invisible on whichever machine the change is typed on.

**Chromium hands a parent's policy down into a frame on a custom scheme.** With a permissive page around it, a message whose own policy names the wrong spelling still showed its pictures—the inherited one allowed them. That is why the probe's own wrapper is written with the policy the application window carries rather than a convenient one: a wrapper that permits more than the window does buys a measurement that the product would not get.

**Weakening the policy to `img-src *` proves nothing here**: a run that used it would come back quiet for the wrong reason: `*` does not cover a scheme that is not one of the network ones, so it forbids the picture this build serves rather than permitting everything. The weakening that isolates the network question is `img-src * lyomsg:`—measured on WebKitGTK, that one sends the engine to the socket and the run comes back a `2`, while `*` alone sends it nowhere and fetches nothing.

Verified means one of two things: someone opened a real message in the running application and looked—a newsletter with nested tables, inline styles, three remote pictures, and three links, and a message carrying a picture inside itself, which was done on macOS—or the probe was run and reported a number, which was done on all three.

The probe also hands the main thread a small piece of work every fifty milliseconds and records how long the longest one waited. That is the row about a costly message, and it is measured rather than watched: no engine loses anything at 46,000 elements either, because a message is laid out in a frame of its own and the thread it costs is not the one the window answers on. The instrument is not blind—a deliberate three-second block on the main thread comes back as 2,960 ms.

Every row above that says **verified** was answered by the probe opening the message in the frame the reader gets, and on each system the same run was made to fail first: with `img-src * lyomsg: …`, `font-src *`, and `style-src *`, all three engines fetched `imported.css`, `a-font.woff2`, `a-background.png`, and the picture pointing at the socket, and each run came back a `2`. A quiet answer counts because of that.

**Measured**: macOS 15 on Apple Silicon; Windows 11 Home 26200 on arm64 with WebView2 151.0.4129.107; Ubuntu 24.04.4 LTS on arm64 with WebKitGTK 2.52.3 in a Wayland session. The engines are the same on x86-64, and the display server is not the subject of any row here. [The virtual machines guide](virtual-machines.md) is how the two guests were built.

### The three engines do not fetch the same number of times

| | Requests the scheme received, the document's and the pictures' together | Requests for a picture inside the message |
| --- | --- | --- |
| WKWebView | 3 | 1 per picture |
| WebKitGTK | 2 | 1 |
| WebView2 | 3 | 2 |

A picture's name answers every request until its body is closed, since a webview decides for itself how often it fetches one, and WebView2 asks twice. **A document's name answers the same way.** A second request answered with nothing would be an empty document replacing the one already rendering and canceling the pictures it had begun to load—a message that appears blank, or with its pictures missing, for reasons nothing on screen explains. Closing its body ends a name: the reader leaving the message, or a redraw replacing it.

### The number an application exits with does not reach the shell on Linux

`AppHandle::exit(code)` carries the code on macOS and does not on Linux: the probe reported that the policy had been ignored and the shell still saw a `0`. Anything whose answer is an exit code has to end the process itself—the probe calls `std::process::exit` after its report is written and flushed. With that, the number arrives on all three: measured `2` for a policy that was not enforced on macOS, Windows, and Linux alike.

### A 204 does not cancel a navigation on a custom scheme

WKWebView does not treat `204 No Content` as "nothing to show, stay where you are" for a scheme the application serves: it commits the navigation and replaces the document, and the message is gone. What works is refusing the navigation before it happens, in the handler Tauri gives the window, which is also what "navigation is intercepted in Rust" means. The other two engines are untested here. Where the handler is shown a frame's navigation, the mechanism does not rest on what a status code means to an engine; WebView2 never shows it one, so on Windows it does (next section).

### The link gate does not cover the message frame on Windows

Refusing the navigation is what keeps the message on screen when a link is followed, and the handler that refuses it is given the window at creation. WebView2 does not hand a *frame's* navigation to that handler—it has `FrameNavigationStarting` for that, and the layer this application is built on does not expose it. A link the sanitizer rewrote reaches the scheme handler as a request instead, and that handler answers it: the window is told about the link and asks, exactly as elsewhere, and nothing is served back. Whether WebView2 then keeps the message on screen is not measured.

A link that got past the sanitizer is the case the gate exists for, and on Windows nothing in this application refuses it. Nobody is asked about such a link on any system: the window asks only about a link the sanitizer rewrote. Whether the window's own `frame-src`, which names only the message scheme, stops the frame before it leaves is not measured.

## Where the tests that ask a real server run

Wherever Docker will hold a Linux container, the Mac included. Measured on the two guests as well:

| Machine | Docker | What happens |
| --- | --- | --- |
| macOS, Apple Silicon | Docker Desktop | both containers start; reading answers on 1143 and sending on 1587 |
| Windows 11 guest | Windows containers | `FROM alpine:3.20` → no matching manifest for `windows/amd64` |
| Linux guest | yes | reading answers on 1143; the sending container on 1587, and the reading image as it stands, have not been run there |

Where nothing answers, the tests that ask a real server skip themselves, and the preflight refuses to run without one.

## Other differences

- **Keyboard**: every shortcut answers to Command or Control on all three, read in one function in `src/shared/ui/keys.ts`, so no screen chooses for itself; `src/shared/platform.ts` writes each shortcut the way its system does.
- **Updates**: on macOS a newer release is swapped in for the running one; on Windows and Linux the application never replaces itself.
- **Native dialogs**: the file picker is the system's own, asked for in Rust, and a path never comes from the window. Seen on macOS; on the two guests the portal is a dependency of the package rather than something this application arranges.
- **Notifications**: nothing in the application asks for permission: a notice goes to the system, and each system decides whether to ask; none has been watched doing it here. A built `.app` carries `CFBundleIdentifier` and `icon.icns`, and macOS attributes its notices to it; a build `npm run app` runs is posted as Terminal, which the notification plugin names for every development build.
- **What a certificate can make this application fetch**: everything it fetches other than mail—a picture, server discovery, the update check—is checked on macOS against the chain the host sent, with the system told to fetch nothing. Left to itself the system fetches an intermediate the host left out from the address the certificate names, which whoever answered chose. Told to fetch nothing, macOS refuses such a chain unless this Mac already keeps the missing certificate, while mail keeps the system's own check and has it fetched. Measured by a test serving a certificate that names an address on this machine for its issuer: nothing reaches that address, and with the system's own check in its place the system asks it. Linux checks a chain against the system's store and fetches nothing, for mail or anything else. Windows keeps the system's own check for everything, and it can fetch the issuer a certificate names and ask where its revocation is kept, so a host a message names can still have a request sent to an address the address policy never sees. Reasoned from the verifier's code, not run.
