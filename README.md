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

| Option                   | Effect                                                               |
|--------------------------|----------------------------------------------------------------------|
| `--user <name>`          | log this user in without asking who                                  |
| `--session <command>...` | what to start (default: `wlrix-session`); takes the rest of the line |
| `-h`, `--help`           | usage                                                                |

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

| Variable      | Default | Purpose                                        |
|---------------|---------|------------------------------------------------|
| `prefix`      | `/usr`  | where the binaries go                          |
| `rootdir`     | *empty* | staged install, as a package build does        |
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
`selected`, `typed`, `password`, `error`.

The interesting part of a greeter is the conversation — how many rounds, what is sent when an answer is refused, what
happens on a wrong password — and none of that is checked by compiling.
