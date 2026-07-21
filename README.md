# wlrix-greeter

The wlRIX login greeter. A [greetd](https://git.sr.ht/~kennylevinsen/greetd)
front-end presenting an IRIX-style login screen (the `clogin` look).

- **Language:** Rust
- **License:** GPL-3.0-or-later
- **Reference:** [cosmic-greeter](https://github.com/pop-os/cosmic-greeter)

The greeter never checks a password. greetd runs as root, holds the PAM conversation, and
starts the session; the greeter only relays the questions PAM wants asked and hands back
the answers. Everything security-relevant is on the other side of the socket.

**The face is a terminal for now.** greetd greeters run directly on a VT, so this is a
working login before any of the Indigo Magic UI exists. The graphical greeter replaces
the prompts and speaks to the same greetd client underneath.

## Build

```sh
cargo build
```

Running it directly does nothing useful: without `GREETD_SOCK` there is nothing to
authenticate against, and it says so rather than presenting a login it cannot complete.

## Options

| Option                   | Effect                                                               |
|--------------------------|----------------------------------------------------------------------|
| `--user <name>`          | log this user in without asking who                                  |
| `--session <command>...` | what to start (default: `wlrix-session`); takes the rest of the line |
| `-h`, `--help`           | usage                                                                |

## greetd configuration

`/etc/greetd/config.toml`:

```toml
[terminal]
vt = 1

[default_session]
command = "wlrix-greeter"
user = "greeter"
```

## Testing without greetd

Real greetd runs as root and starts real sessions, so it is not something to point at a
dev machine casually. `examples/fake_greetd.rs` speaks the same protocol and
authenticates nobody — it asks for a password, accepts one fixed answer, and prints what
the greeter asked it to start:

```sh
cargo run --example fake_greetd -- /tmp/fake-greetd.sock &
GREETD_SOCK=/tmp/fake-greetd.sock cargo run
```

The interesting part of a greeter is the conversation — how many rounds, what is sent
when an answer is refused, what happens on a wrong password — and none of that is
checked by compiling.
