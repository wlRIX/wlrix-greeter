# wlrix-greeter

The wlRIX login greeter. A [greetd](https://git.sr.ht/~kennylevinsen/greetd)
front-end presenting an IRIX-style login screen (the `clogin` look).

- **Language:** Rust
- **License:** GPL-3.0-or-later
- **Reference:** [cosmic-greeter](https://github.com/pop-os/cosmic-greeter)

The greeter never checks a password. greetd runs as root, holds the PAM conversation, and starts the session; the
greeter only relays the questions PAM wants asked and hands back the answers. Everything security-relevant is on the
other side of the socket.

## Build

```sh
cargo build
```

Running it directly does nothing useful: without `GREETD_SOCK` there is nothing to authenticate against, and it says so
rather than presenting a login it cannot complete.

## Options

| Option                   | Effect                                                                     |
|--------------------------|----------------------------------------------------------------------------|
| `--user <name>`          | log this user in without asking who                                        |
| `--session <command>...` | what to start (default: `start-wlrix`); takes the rest of the line         |
| `--palette <name>`       | color scheme: `classic` (default), `classic-g10`, `classic-g24`, `gotham`  |
| `-h`, `--help`           | usage                                                                      |

`--palette` has to come *before* `--session`, which takes the rest of the line.

## The login screen's settings

The greeter reads no config file of its own: it runs before there is a user whose settings could be read. Its keyboard
layout, color scheme and displays are the compositor's, and `wlrix-greeter-start` gives that compositor a file of its own:

```sh
wlrix-compositor --config /etc/wlrix/greeter.toml -c wlrix-greeter
```

`/etc/wlrix/greeter.toml` is a `compositor.toml` — the same sections, the same rules — and the **Login Settings** panel
(`wlrix-settings-login`, in wlrix-apps) writes it, including copying the keyboard, scheme and displays of whoever is
logged in. It can be edited by hand just as well; `wlrix-compositor --check-config` says whether an edit would be accepted.
Until it exists the compositor falls back to `/etc/wlrix/compositor.toml`, so a fresh install comes up with the system's
keyboard and pointer, and `install` does not create it.

Displays are `[[output]]` sections in that file. The compositor also keeps a saved-state file of output changes made over
`wlr-output-management`, which wins field by field; under the greeter that is
`/var/lib/wlrix-greeter/.local/state/wlrix/outputs.toml`, and nothing at the login screen writes it. If one is there from
some earlier experiment, delete it, or it will override what the panel set.

The compositor passes its `[appearance] palette` to the greeter in `WLRIX_PALETTE`, so the dialog and the frame around it
are one setting. `--palette` overrides it, for running the greeter by hand; it cannot come through `-c`, which takes one
program and no arguments.

An unrecognized scheme name logs a line and draws the default. A mistyped scheme must not stop the login screen from
coming up, because there is no way to fix it from a machine that will not let you log in.

## Installing

```sh
just release
sudo just install
sudo systemd-sysusers && sudo systemd-tmpfiles --create
sudo systemctl disable greetd.service
sudo systemctl enable wlrix-greeter.service
```

`install` deliberately does not build: it is normally run as root, and building as root leaves a target directory nobody
can write to afterwards.

| Variable     | Default | Purpose                                        |
|--------------|---------|------------------------------------------------|
| `prefix`     | `/usr`  | where the binaries go                          |
| `rootdir`    | *empty* | staged install, as a package build does        |
| `pam-flavor` | `arch`  | which PAM stack to install (`arch` / `debian`) |

What lands:

| Path                               |                                              |
|------------------------------------|----------------------------------------------|
| `$prefix/bin/wlrix-greeter`        | the greeter                                  |
| `$prefix/bin/wlrix-greeter-start`  | the launcher greetd runs                     |
| `$prefix/lib/sysusers.d/…`         | the `wlrix-greeter` account                  |
| `$prefix/lib/tmpfiles.d/…`         | its home and runtime directories             |
| `$prefix/lib/systemd/system/…`     | the `wlrix-greeter.service` unit             |
| `/etc/pam.d/wlrix-greeter`         | the stack that authenticates whoever logs in |
| `/etc/pam.d/wlrix-greeter-greeter` | the stack for the greeter's own session      |
| `/etc/greetd/wlrix-greeter.toml`   | greetd configuration                         |
| `$prefix/share/wlrix-greeter/…`    | the default user picture                     |

`/etc/wlrix/greeter.toml` is not in the list: it is the administrator's, written by the Login Settings panel or by hand.

`install` never touches `/etc/greetd/config.toml`: that file is the administrator's, and may already point at another
greeter.

### The service

`wlrix-greeter.service` **is** greetd, started against `/etc/greetd/wlrix-greeter.toml`. It does not run alongside the
distribution's `greetd.service` — that is the same daemon wanting the same VT and the same socket — so the unit declares
`Conflicts=greetd.service` and enabling one means disabling the other. `[Install] Alias=display-manager.service` is what
makes `systemctl enable`
hand it the display-manager slot that `graphical.target` pulls in.

Modeled on the unit Arch's greetd package ships, with three differences: the config path, the conflict above, and
`StartLimitBurst`/`StartLimitIntervalSec` in `[Unit]`, which is where systemd moved them.

### The two PAM services

greetd holds the PAM conversation, as root; the greeter never checks a password. That means two stacks, and greetd has
to be *told* both names or it uses its own defaults and neither of these files is ever read:

- **`wlrix-greeter`** — named by `[general] service`. Authenticates whoever is logging in, and is where
  `pam_gnome_keyring` belongs: the keyring is unlocked with that person's password.
- **`wlrix-greeter-greeter`** — named by `[default_session] service`. Starts the greeter itself as the `wlrix-greeter`
  account, before anybody has logged in. Shipped rather than left to greetd's `greetd-greeter` default, which no
  distribution is obliged to provide — Arch's greetd package ships only `greetd`, and PAM's fallback for an unknown
  service denies everything, so the greeter would not start at all.

### Telling logind it is a Wayland session

`StartSession` carries `XDG_SESSION_TYPE=wayland`, and it has to be *here* rather than in the session's own startup
script. greetd hands a greeter's environment to `pam_putenv` **before** `pam_open_session`, and says in its own source
that it does so precisely so it can affect `pam_systemd.so` — so this is what decides how logind records the session,
and it is why `loginctl show-session -p Type` answers `wayland`.

Left out, `pam_systemd` infers the type from how the login arrived, and a greetd login is a console login as far as
logind is concerned, so it lands on `tty`. That is not cosmetic: Chromium and Edge resolve their default
`--ozone-platform-hint=auto` by reading `XDG_SESSION_TYPE`, and fall back to X11 when it does not say `wayland` — the
desktop's browser was running under XWayland for exactly this reason. `start-wlrix` force-exports the variable as well,
which covers starting a session from a TTY by hand, but an export cannot reach PAM: by the time the script runs, logind
has already made up its mind.

`cosmic-greeter` does the same thing, per session type, and additionally derives `XDG_CURRENT_DESKTOP` and
`XDG_SESSION_DESKTOP` from the chosen session's `DesktopNames`. Those two are not sent here: `pam_systemd` has no
opinion on either, `start-wlrix` already sets both, and nothing runs in between that reads them.

### PAM flavors

The two stacks come in `arch` and `debian` variants, and they are not interchangeable: Debian's
`common-*` includes do not exist on Arch, and Arch's `system-login` does not exist on Debian. Installing the wrong one
leaves a stack whose includes are missing, which is a login that cannot succeed. Nothing is auto-detected — a package
build stages into `rootdir` on whatever machine the builder happens to be, which says nothing about where the package
will land.

```sh
sudo just pam-flavor=debian install
```

## Testing without greetd

Real greetd runs as root and starts real sessions, so it is not something to point at a dev machine casually.
`examples/fake_greetd.rs` speaks the same protocol and authenticates nobody — it asks for a password, accepts one fixed
answer, and prints what the greeter asked it to start:

```sh
just fake-greetd &                       # defaults to /tmp/fake-greetd.sock
GREETD_SOCK=/tmp/fake-greetd.sock cargo run
```

`just preview <state>` draws the login dialog with no greetd behind it at all, for eyeballing against the reference:
`selected`, `typed`, `password`, `error`. A second argument picks the scheme.

`examples/render_preview.rs` writes the same frame to a PNM with **no compositor at all**, which is what lets a change
to the drawing be diffed against the frame before it rather than eyeballed:

```sh
cargo run --example render_preview -- before.pnm            # classic
cargo run --example render_preview -- after.pnm gotham password
```

## Shared drawing code

The canvas, the Motif bevel, the widgets, the fonts and the palette come from
[`wlrix-ui`](https://github.com/wlRIX/wlrix-ui). This repo used to carry its own copy of all of it — `wlrix-desktop`
copied that copy, and the two drifted.

It is a **git dependency pinned by rev**, not a path dependency, so this repo still builds from a fresh standalone
clone. `just link-ui` in `wlrix-epoch` patches in a sibling checkout for working on both sides at once.

## User pictures

Two kinds, and IRIX showed their state two different ways. Both were measured off the real login screen rather than
guessed.

**A user with a picture of their own** shows it exactly as it is, in a frame: one black pixel around it at rest; under
the pointer, two white ones — that pixel and the picture's own outermost row; selected, the same two in lamp yellow. The
picture is the first of `~/.face.icon`, `~/.face` and `/var/lib/AccountsService/icons/<login>` that decodes — the places
`wlrix-lock` looks, in its order — center-cropped to a square. PNG and JPEG, through the same `image` crate and features
the lock uses. Only regular files are read, so a `~/.face` that points at a FIFO cannot hang the login screen.

The greeter runs as its own account, so a home directory that is not world-readable (`0700`, the default on several
distributions) hides `~/.face` from it. The AccountsService copy is what it finds then — the one KDE's and GNOME's
settings panels write.

**Everyone else** shows IRIX's figure at a workstation, `assets/default-user.png`, and there **only the white body
changes.** At rest it is a 50% checkerboard of white and the palette's rest gray; under the pointer it is solid white;
selected, solid lamp yellow. The black outline and the gray shadow are identical in all three. The rule is `wlrix-ui`'s
`icon_state`, the same one the desktop's icons follow, so a selected user here and a selected file there are the same
yellow and change the same way.

The picture is found as `wlrix-greeter/default-user.png` on the XDG data directories, which `just install` puts under
the prefix, and then in the checkout's own `assets/`, so `just render` and `just preview` show it without an install.
With neither, a user without a picture of their own shows their name alone.

The interesting part of a greeter is the conversation — how many rounds, what is sent when an answer is refused, what
happens on a wrong password — and none of that is checked by compiling.
