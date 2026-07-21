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

mod greetd;
mod prompt;
mod theme;
mod worker;

use greetd::{Greetd, Outcome, QuestionKind};

/// What a successful login starts.
const DEFAULT_SESSION: &str = "wlrix-session";

struct Args {
    session: Vec<String>,
    /// Skip asking, for when greetd was told which user this is.
    username: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut session = vec![DEFAULT_SESSION.to_string()];
    let mut username = None;

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
            "--help" | "-h" => {
                println!(
                    "wlrix-greeter {}\n\n\
                     A greetd greeter. Started by greetd, which sets GREETD_SOCK.\n\n\
                     Usage: wlrix-greeter [options]\n\n\
                     Options:\n  \
                       --user <name>           log this user in without asking who\n  \
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

    Ok(Args { session, username })
}

fn main() -> std::process::ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("wlrix-greeter: {err}");
            return std::process::ExitCode::from(2);
        }
    };

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
