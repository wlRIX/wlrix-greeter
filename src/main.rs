// SPDX-License-Identifier: GPL-3.0-or-later
//! wlRIX greeter.
//!
//! A `greetd` greeter: it asks who you are, relays PAM's questions, and asks greetd to
//! start the session. It never checks a password itself -- greetd does that, as root,
//! on the other side of a socket.
//!
//! The face is a terminal for now. greetd greeters run directly on a VT, so this is a
//! working login before any of the Indigo Magic UI exists; the graphical greeter
//! replaces [`prompt`] and speaks to the same [`greetd`] underneath.

use wlrix_greeter::greetd::{Greetd, Outcome, QuestionKind};
use wlrix_greeter::{prompt, ui};

/// What a successful login starts.
///
/// The launcher, not `wlrix-session` itself: it is what sets the session's environment --
/// `XDG_CURRENT_DESKTOP`, the toolkit backends, the keyring, the systemd user environment --
/// and then execs the session manager. Starting the binary directly skips all of it.
const DEFAULT_SESSION: &str = "start-wlrix";

struct Args {
    session: Vec<String>,
    /// Skip asking, for when greetd was told which user this is.
    username: Option<String>,
    /// Use the terminal face instead of the graphical one. The graphical face is the
    /// default now that it can log in; the terminal face stays for a VT with no
    /// compositor, and as a fallback.
    text: bool,
    /// Which color scheme to draw in.
    ///
    /// A flag rather than a config key, and deliberately: the greeter reads no config file
    /// of its own -- it runs before there is a user whose settings could be read, and its
    /// configuration is greetd's, root-owned under `/etc/greetd/`. That is the right place
    /// for this, so greetd passes it:
    ///
    /// ```toml
    /// command = "wlrix-compositor -c 'wlrix-greeter --palette gotham'"
    /// ```
    palette: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut session = vec![DEFAULT_SESSION.to_string()];
    let mut username = None;
    let mut text = false;
    let mut palette = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            // Everything after this is the session command and its arguments.
            "--session" => {
                let rest: Vec<String> = args.by_ref().collect();
                if rest.is_empty() {
                    return Err("--session needs a command".to_string());
                }
                session = rest;
                break;
            }
            "--user" => {
                username = Some(
                    args.next()
                        .ok_or_else(|| "--user needs a name".to_string())?,
                );
            }
            // Fall back to the terminal prompts, for a VT with no compositor.
            "--text" => text = true,
            "--palette" => {
                palette = Some(
                    args.next()
                        .ok_or_else(|| "--palette needs a scheme name".to_string())?,
                );
            }
            "--help" | "-h" => {
                println!(
                    "wlrix-greeter {}\n\n\
                     A greetd greeter. Started by greetd, which sets GREETD_SOCK.\n\n\
                     Usage: wlrix-greeter [options]\n\n\
                     Options:\n  \
                       --user <name>           log this user in without asking who\n  \
                       --text                  use the terminal prompts, not the graphical screen\n  \
                       --palette <name>        colour scheme: classic (default), classic-g10,\n  \
                                               classic-g24, gotham\n  \
                       --session <command>...  what to start (default: {DEFAULT_SESSION});\n  \
                                               takes the rest of the line\n  \
                       -h, --help              this message",
                    env!("CARGO_PKG_VERSION")
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(Args {
        session,
        username,
        text,
        palette,
    })
}

fn main() -> std::process::ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("wlrix-greeter: {err}");
            return std::process::ExitCode::from(2);
        }
    };

    if !args.text {
        // Send the graphical greeter's stderr to a file first: under greetd it draws over the
        // console its stderr goes to, so without this a bad run leaves nothing to read. Announce
        // the path on stdout (still the terminal) so it can be found when run by hand.
        if let Some(path) = wlrix_greeter::logging::to_file() {
            println!("wlrix-greeter: logging to {}", path.display());
        }
        // Resolve rather than reject: a mistyped scheme name in greetd's config must not
        // stop the login screen from coming up, because there is no way to fix it from a
        // machine that will not let you log in.
        let (palette, unknown) = wlrix_ui::palette::resolve(args.palette.as_deref());
        if let Some(why) = unknown {
            eprintln!("wlrix-greeter: {why}; using {}", palette.id);
        }
        return match ui::run(args.session, args.username, palette) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("wlrix-greeter: {err}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    let mut greetd = match Greetd::connect() {
        Ok(greetd) => greetd,
        Err(err) => {
            eprintln!("wlrix-greeter: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };

    println!("wlRIX");

    // Until a login succeeds. A greeter that gave up after one wrong password would
    // leave the machine unusable.
    loop {
        let username = match &args.username {
            Some(username) => username.clone(),
            None => match prompt::ask("login: ") {
                Ok(Some(username)) if !username.trim().is_empty() => username.trim().to_string(),
                // Empty line: ask again rather than sending greetd a blank user.
                Ok(Some(_)) => continue,
                // Input ended. Nothing more can be asked, so there is no point looping.
                Ok(None) => return std::process::ExitCode::FAILURE,
                Err(err) => {
                    eprintln!("wlrix-greeter: could not read: {err}");
                    return std::process::ExitCode::FAILURE;
                }
            },
        };

        let attempt = greetd.login(&username, &args.session, |question| match question.kind {
            // Told, not asked. The empty answer is discarded by the caller; PAM just
            // wants the round trip.
            QuestionKind::Info => {
                println!("{}", question.text);
                Some(String::new())
            }
            QuestionKind::Error => {
                eprintln!("{}", question.text);
                Some(String::new())
            }
            QuestionKind::Secret => prompt::ask_secret(&question.text).ok().flatten(),
            QuestionKind::Visible => prompt::ask(&question.text).ok().flatten(),
        });

        match attempt {
            Ok(Outcome::Started) => {
                // greetd takes over from here: it stops the greeter and runs the
                // session in its place.
                return std::process::ExitCode::SUCCESS;
            }
            Ok(Outcome::Denied(reason)) => {
                eprintln!("{reason}");
                // A named user who cannot log in has no second thing to try, so
                // looping would spin.
                if args.username.is_some() {
                    return std::process::ExitCode::FAILURE;
                }
            }
            Err(err) => {
                eprintln!("wlrix-greeter: {err}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }
}
