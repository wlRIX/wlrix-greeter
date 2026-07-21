// SPDX-License-Identifier: GPL-3.0-or-later
//! A stand-in for `greetd`, to test the greeter against.
//!
//! Real greetd runs as root, holds a PAM conversation, and starts sessions, so it is not
//! something to point at a dev machine casually. This speaks the same protocol and
//! authenticates nobody: it asks for a password, accepts one fixed answer, and reports
//! what the greeter asked it to start.
//!
//! It exists because the interesting part of the greeter is the conversation -- how many
//! rounds, what is sent when an answer is refused, what happens on a wrong password --
//! and none of that is checked by compiling.
//!
//! Usage:
//! ```sh
//! cargo run --example fake_greetd -- /tmp/fake-greetd.sock [password]
//! GREETD_SOCK=/tmp/fake-greetd.sock cargo run
//! ```
//! Not part of the greeter; a dev tool only.

use std::os::unix::net::{UnixListener, UnixStream};

use greetd_ipc::{AuthMessageType, ErrorType, Request, Response, codec::SyncCodec};

/// Accepted by the fake, and by nothing else anywhere.
const DEFAULT_PASSWORD: &str = "correct-horse";

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        eprintln!("usage: fake_greetd <socket-path> [password]");
        std::process::exit(2);
    });
    let password = args.next().unwrap_or_else(|| DEFAULT_PASSWORD.to_string());

    // A stale socket from a previous run would otherwise make binding fail.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("could not bind the socket");
    println!("fake greetd on {path} (password: {password})");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => serve(stream, &password),
            Err(err) => eprintln!("accept failed: {err}"),
        }
    }
}

/// One greeter connection, for as long as it keeps talking.
fn serve(mut stream: UnixStream, password: &str) {
    // What the current attempt is waiting for. greetd allows one at a time.
    let mut awaiting_password = false;

    while let Ok(request) = Request::read_from(&mut stream) {
        let response = match request {
            Request::CreateSession { username } => {
                println!("  create_session: {username}");
                awaiting_password = true;
                Response::AuthMessage {
                    auth_message_type: AuthMessageType::Secret,
                    auth_message: "Password: ".to_string(),
                }
            }
            Request::PostAuthMessageResponse { response } => {
                if !awaiting_password {
                    Response::Error {
                        error_type: ErrorType::Error,
                        description: "nothing was asked".to_string(),
                    }
                } else {
                    awaiting_password = false;
                    match response.as_deref() {
                        Some(given) if given == password => {
                            println!("  password accepted");
                            Response::Success
                        }
                        _ => {
                            println!("  password refused");
                            Response::Error {
                                error_type: ErrorType::AuthError,
                                description: "Login incorrect".to_string(),
                            }
                        }
                    }
                }
            }
            // The point of the test: what the greeter would have launched.
            Request::StartSession { cmd, env } => {
                println!("  start_session: cmd={cmd:?} env={env:?}");
                Response::Success
            }
            Request::CancelSession => {
                println!("  cancel_session");
                awaiting_password = false;
                Response::Success
            }
        };

        if response.write_to(&mut stream).is_err() {
            break;
        }
    }
    println!("  greeter disconnected");
}
