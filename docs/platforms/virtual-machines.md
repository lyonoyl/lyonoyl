# Machines to measure on

Three of the four layers message rendering rests on are properties of an engine this project did not write, and they are three different engines: WKWebView, WebView2, WebKitGTK. Answering them means running the same probe on all three, and almost nobody has three machines.

Two virtual machines on one desktop close that. This page is how to build them, step by step, and what has to be true before work can carry on inside them.

What the probe is and how to read what it says is in [the development guide](../development/guide.md#asking-a-webview-engine-what-it-does); what has been answered is in [the platform differences](differences.md).

---

## The host is a Mac

The guests run on a Mac with Apple Silicon. That is why they are ARM64—a guest matches the host it is virtualized on—and why `prepare-host.sh` builds its disc with `hdiutil`, which exists nowhere else.

A release is the macOS build and the source; Windows and Linux are built from source by whoever runs them ([the working contract](../../AGENTS.md), section 6). These guests are for whoever wants to look at either of them before a person who runs it does—and macOS is the one engine answered both ways: by the probe, as the other two are, and by a person looking at a real message.

So there is one script per **role**:

| | Runs on | Its job |
| --- | --- | --- |
| `prepare-host.sh` | The host, a Mac | Makes the key, and packs what has to travel into a guest |
| `prepare-linux-vm.sh` | Inside the Linux guest | Opens it to SSH with that key and installs what a build needs |
| `prepare-windows-vm.ps1` | Inside the Windows guest | Opens it to SSH with that key |

On a host that is not a Mac, the two guest scripts are unaffected; only the disc-building step in `prepare-host.sh` has nowhere to run, and it says so and names what to use instead.

---

## What each piece costs

| Piece | What it is | What it costs |
| --- | --- | --- |
| [UTM](https://getutm.app) | The hypervisor, on macOS | Free, Apache-2.0. The App Store copy is the same application with a price on it; the download from the site or from GitHub is the one to take |
| Ubuntu 24.04 LTS | The Linux guest | Free |
| Windows 11 ARM64 | The Windows guest | A Windows license, or the terms Microsoft publishes with an evaluation image |
| OpenSSH, rustup, NodeSource | Everything the guest scripts install | Free |
| Visual Studio Build Tools 2022, LLVM, Strawberry Perl, Node | The Windows toolchain, installed by hand | Free to download, under each one's own terms |

No paid runner, no subscription, and no signing certificate is involved in measuring anything.

**What you need of your own:** a Mac with Apple Silicon if you want ARM64 guests, about 40 GB of free disk for each, and 4–6 GB of memory to lend one while it runs. On an Intel Mac, or on a Linux host with a different free hypervisor, the same steps work with x86-64 guests—nothing here depends on the architecture except which image you download and the Windows toolchain, whose x64 builds replace the ARM64 ones in the table below.

---

## The whole thing, in order

| Step | Where you are | What moves, and where to |
| --- | --- | --- |
| 0 | Your machine | `prepare-host.sh` writes a folder and an `.iso` to the Desktop |
| 1 | UTM | A new guest is created |
| 2 | The guest's installer | The system is installed and an account is made |
| 3 | The guest's desktop | The `.iso` is attached and its script is run **there** |
| 4 | UTM | The guest is shut down; its network becomes a forwarded port |
| 5 | The guest's desktop | It is started and logged into, and left that way |
| 6 | Your machine | `ssh` reaches it, and everything after this is over that |

---

### 0. On your machine: make the key and the disc

```bash
bash scripts/prepare-host.sh --for windows     # or --for linux, or --for both
```

It makes `~/.ssh/id_lyonoyl_vm` if it is not there yet—an ed25519 key with no passphrase, because these guests are driven without a person present and a passphrase means answering a prompt at every connection. It opens throwaway machines behind a local port forward and nothing else, and it is not the key you push with.

Then it gathers what has to travel: the script for that platform, the **public** half of the key beside it, and a note saying what to type. On macOS it also builds an `.iso` of that folder, which is the way into a guest that has nothing installed in it yet—drag-and-drop needs guest tools, and a fresh system does not have them.

Nothing in that folder is secret, and nothing in it is committed here.

### 1. In UTM: create the guest

**Virtualize**, not Emulate—an ARM64 guest on an Apple Silicon host runs at its own speed.

| | Linux | Windows |
| --- | --- | --- |
| Memory | 4 GB is enough, 6 is comfortable | 6 GB |
| CPU cores | 4–6 | 4–6 |
| Disk | 30 GB | 40 GB |
| Display | Leave hardware acceleration on if the installer accepts it; if it refuses, carry on without it | Default |

Network can stay on the default for now. It is changed in step 4, and it has to be changed with the guest shut down.

### 2. Install the system, and make the account

Ordinary installation, nothing special. **Write down the account name**—it is what you log in as over SSH, and on Windows it is not always what the login screen shows.

Log in graphically once the installer is done.

### 3. In the guest: attach the disc and run the script

In UTM, attach the `.iso` from step 0 as a CD/DVD drive, then open it in the guest.

**Linux**: open the disc in the file manager, right-click inside it, **Open in Terminal**, then:

```bash
bash prepare-linux-vm.sh --nopasswd-sudo
```

`bash`, not `./`—a mounted disc is read-only and usually marked no-exec, and running the file directly fails for a reason that has nothing to do with the script.

**Windows**: open the disc and **right-click `prepare-windows-vm.cmd` → Run as administrator**. That is the whole of it.

The launcher is there because the window Windows opens by default is `cmd`, where a `.ps1` cannot simply be typed, and because the disc is not always drive `D:`. It works from a double-click, from `cmd`, and from PowerShell alike, refuses to run unelevated with a sentence saying so, and finds the script and the key by its own path rather than by a letter.

Do it while the guest still has network: the OpenSSH server is fetched through Windows Update. That step can print nothing for ten minutes on an ARM64 guest and is not stuck—the script says so before it starts, and warns first if the update service is switched off, which is the one thing that does wedge it.

Each finds the `.pub` file beside it, so there is nothing to type. Each checks every step after taking it and stops at the first that did not hold; a run that ends in **Ready** has been verified, not assumed. Running one again is safe—it reports what is already in place and leaves it alone.

#### About the disc

It is built by `prepare-host.sh` and it is **not** kept here. An `.iso` is a built thing: binary, carrying somebody's public key, and stale the moment the script inside it changes. `.gitignore` refuses one.

**A disc that was already attached is the trap.** Rebuild the image, leave it attached, and the guest goes on serving the copy it had from a cache—the old script, under the same name, looking exactly right.

Nothing has to be compared by hand. Each guest script prints its own SHA-256 near the top, on the line that starts `This script:`, and the note on the disc says what that line should read; `prepare-host.sh` prints the same fingerprint when it builds. If the two differ, eject the disc in UTM and attach it again. That is the whole fix.

`--nopasswd-sudo` weakens the Linux guest on purpose. Without it, every command driven over SSH stops at a password prompt with nobody there to answer it. It belongs on a machine that exists to be thrown away and is reachable only through a local port forward. Leave it off otherwise.

What the Linux script installs, beyond SSH: the build dependencies a Tauri build needs on 24.04, Rust through rustup, and Node 22. The Windows script installs SSH and stops—see "The Windows toolchain" below.

### 4. Shut it down, and give it a forwarded port

With the guest **off**, in UTM: Network → **Emulated VLAN**, then add a port forward. The two fields are easy to fill in the wrong order, and filling them wrongly fails silently, so they are spelled out:

| Field | Value | What it is |
| --- | --- | --- |
| **Guest Port** | `22` | Inside the machine, where sshd listens. Always 22 |
| **Host Port** | `2222` for Linux, `2223` for Windows | On your machine. The number that goes after `-p` |
| **Host Address** | `127.0.0.1` | On your machine, the only address the forward listens on. Left empty, it listens on every network the Mac is on |

**The host port must be above 1024.** Put `22` in the Host Port field and nothing binds: a port below 1024 needs root, QEMU is not running as root, and the forward simply never appears. Nothing reports it. `ssh` says *connection refused*, the guest looks perfectly healthy, and its `sshd` is listening on a port nothing on the host reaches. It happens on both guests.

What the two fields actually became can be read off the running process, which settles it in one line:

```bash
ps -Ao command= | grep -o 'hostfwd=[^ ,]*'      # hostfwd=tcp:<address>:<host>-:<guest>
```

`hostfwd=tcp:127.0.0.1:2223-:22` is right. `hostfwd=tcp::2223-:22` listens on every network the Mac is on, and `hostfwd=tcp:127.0.0.1:22-:2223` has the two ports swapped.

Not Shared Network. In that mode the guest can reach the host and the host cannot reach the guest, which is exactly the wrong direction here.

A forwarded port also does not depend on a bridge, on the network you happen to be on, or on DHCP.

### 4a. Windows needs Perl

`libsqlite3-sys` is built with `bundled-sqlcipher-vendored-openssl`—every page of every database is encrypted, and that feature builds OpenSSL from source. On Windows that build is driven by Perl, and without one `cargo build` stops at `openssl-sys` with *Command 'perl' not found*. It is not a guest quirk: anybody building this on Windows meets it.

```powershell
winget install --id StrawberryPerl.StrawberryPerl -e --silent
```

Then `C:\Strawberry\perl\bin` has to be on `PATH` for the build. A shell opened before the install does not have it.

### 5. Start it, log in, and leave it

Log in graphically and leave the session open. The probe opens a real window, and a window needs a session to open into. **Nobody has to look at it**—the report is a file and an exit code—but there has to be one.

### 6. From your machine

```bash
ssh -i ~/.ssh/id_lyonoyl_vm -p 2223 <account>@127.0.0.1
```

That is the last step anybody performs by hand.

On Windows, a session opened this way for an account in the Administrators group arrives **already elevated**—sshd builds the full token rather than the filtered one. Installing components, writing under `HKLM`, and running the installer all work over the connection, with no prompt anybody would have to answer at the guest's own screen.

---

## Before the work can carry on

Four things have to be true. Until they are, nothing can be driven from outside the guest:

- [ ] The guest is **running**, and somebody is **logged in graphically**. A locked screen is fine; a login screen is not.
- [ ] `ssh` with the key reaches it on its forwarded port, and does not ask for a password.
- [ ] The account name is known, because it is half of the address.
- [ ] On Linux, `sudo` does not ask for a password—or somebody is at the keyboard for every command that needs it.

Everything after that is files and commands over that connection.

---

## Getting things in and out

| Direction | How | When |
| --- | --- | --- |
| In, before SSH | The `.iso` from step 0, attached as a drive | The only way into a guest with nothing installed in it |
| Both, after SSH | `rsync -az -e "ssh -i <key> -p <port>" …` | Everything else, including the source tree |
| In, quick | Paste into a terminal in the guest window | One or two commands, no more |

The source tree travels without `target/`, `node_modules/`, and `.git/`—a few hundred files. No git credentials are needed in a guest, and none should be put there.

Drag-and-drop is not on this list. It needs guest tools installed, and installing them is a longer road than attaching a disc.

---

## The traps

Each of these reports itself somewhere other than where it happens, which is what makes it expensive: the message names a line, a port, or a program that is perfectly fine, and the cause is elsewhere. The scripts handle the ones a script can, and the gate refuses the last.

**Linux: no host keys.** A freshly installed `openssh-server` can refuse to start with `sshd: no hostkeys available`, and `journalctl` does not say so—`sudo sshd -t` does. `sudo ssh-keygen -A` fixes it.

**Windows: the key is not read from the profile.** For an account in the Administrators group, sshd reads `C:\ProgramData\ssh\administrators_authorized_keys` and ignores `~\.ssh\authorized_keys` completely. It also ignores that file unless Administrators and SYSTEM are the only ones who can write it—and the rights have to be granted **by SID**, because on a localized Windows those two group names are translated and a grant by name fails while looking like it worked.

**Windows: the firewall rule covers the wrong network.** The rule the OpenSSH capability leaves behind applies to the **Private** profile. Step 4 then changes the network, and the adapter that appears is classified **Public**—so a running `sshd`, listening on `0.0.0.0:22`, is blocked, and the only symptom is a connection that opens and never receives a banner. `prepare-windows-vm.ps1` sets the rule to every profile and checks that it took. Where a guest's rule covers only Private:

```powershell
Set-NetFirewallRule -Name OpenSSH-Server-In-TCP -Profile Any
```

**Windows: a native program writing to the error stream is not failing.** Driving a build over SSH through PowerShell with `$ErrorActionPreference = 'Stop'` and `2>&1`, the first line `cargo` prints about updating its index becomes a terminating error, and the run dies before the compiler starts. Read the exit code instead, and set the preference back to `Continue` around anything native.

**Windows: a script that is not ASCII is read as something else.** Windows PowerShell 5.1—the one that ships with Windows 11—reads a file with no byte order mark as ANSI rather than UTF-8. A UTF-8 em dash decoded that way becomes three characters, the last of which is a right double quote, and PowerShell accepts that as a string delimiter. One of them unbalances every quote below it, and the parse error is reported dozens of lines further down, on a line that is perfectly correct. ASCII decodes identically either way, so `scripts/prepare-windows-vm.ps1` stays ASCII and `scripts/check-layering.sh` fails if a byte above 0x7F appears in any `.ps1`, `.cmd`, or `.bat` under `scripts/`.

Checking a PowerShell script with `pwsh` on macOS does not catch this: that parser reads UTF-8, so it sees the file the way it was written rather than the way Windows will.

---

## The Windows toolchain

It is installed by hand over SSH, and this is what a build needs:

| | What | Note |
| --- | --- | --- |
| Visual Studio Build Tools 2022 | `Microsoft.VisualStudio.Workload.VCTools` and `Microsoft.VisualStudio.Component.VC.Tools.ARM64` | The bootstrapper from `aka.ms/vs/17/release/vs_BuildTools.exe`, run with `--quiet --wait --norestart` |
| **LLVM** | `clang.exe` on the path | `ring` does not build on `aarch64-pc-windows-msvc` without it, and the failure names `cc-rs`, not `ring`. The Visual Studio installer's `modify` refuses to add it; the standalone `LLVM-*-woa64.exe` from the LLVM releases installs natively and silently with `/S /D=C:\LLVM` |
| Rust | `rustup-init.exe` for `aarch64-pc-windows-msvc`, minimal profile | |
| Node 22 | the `arm64.msi`, `msiexec /quiet` | |

A debug build of the whole application then takes about five minutes on four cores, and leaves roughly 19 GB free on a 40 GB disk.

---

## When you are done

Shut the guest down. Do not delete it—building one again is an hour, and it is wanted whenever the probe changes, whenever a platform-specific defect shows up, and for every row [the platform differences](differences.md) mark reasoned.
