// SPDX-License-Identifier: GPL-3.0-or-later
//! The greetd conversation, on its own thread.
//!
//! PAM sleeps for a second or more after a wrong password. Doing that on the thread
//! drawing the login screen would freeze it mid-login: the caret would stop blinking, and
//! the window would stop answering, exactly when the person is most likely to think
//! something has broken. So the conversation runs here and the screen keeps painting.
//!
//! Replies come back over a `calloop` channel, which the Wayland event loop already
//! waits on -- no polling, and no second loop on the drawing side.
//!
//! Notices are handled here rather than passed up: an info or error message from PAM is
//! told, not asked, so the worker shows it and answers the round trip itself. The front
//! end only ever has to respond to a real question.

// The front end that drives this arrives in a later stage. Compiled meanwhile so it
// does not drift away from `greetd`.
#![allow(dead_code)]

use std::{sync::mpsc, thread};

use calloop::channel::{Channel, Sender};

use crate::greetd::{Greetd, Question, QuestionKind, Step};

/// What the front end asks for.
pub enum Command {
    /// Try to log this user in. Any attempt already running is abandoned first.
    Begin { username: String },
    /// Answer the outstanding question.
    Answer(String),
    /// Give up on the current attempt.
    Cancel,
}

// Handwritten so an answer -- which is usually a password -- cannot reach a log by way
// of a derived `Debug`.
impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Begin { username } => {
                f.debug_struct("Begin").field("username", username).finish()
            }
            Self::Answer(_) => f.write_str("Answer(<redacted>)"),
            Self::Cancel => f.write_str("Cancel"),
        }
    }
}

/// What the conversation says back.
#[derive(Debug)]
pub enum Reply {
    /// PAM wants an answer. A `Secret` question is what makes a password field appear --
    /// the greeter never predicts one.
    Question(Question),
    /// Something to show, already acknowledged.
    Notice(String),
    /// The session is running.
    Started,
    /// Refused; the attempt is over and another may begin.
    Denied(String),
    /// The socket or greetd itself failed. Nothing further will arrive.
    Broken(String),
}

/// Sends commands to the conversation.
pub struct Handle {
    commands: mpsc::Sender<Command>,
}

impl Handle {
    /// Ask for something. Fails only once the worker has stopped, which the front end
    /// learns about through [`Reply::Broken`] instead.
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

/// Start the conversation on its own thread.
///
/// `command` is what a successful login starts.
pub fn spawn(greetd: Greetd, command: Vec<String>) -> (Handle, Channel<Reply>) {
    let (commands, inbox) = mpsc::channel();
    let (replies, channel) = calloop::channel::channel();

    thread::Builder::new()
        .name("greetd".to_string())
        .spawn(move || run(greetd, command, inbox, replies))
        .expect("could not start the greetd thread");

    (Handle { commands }, channel)
}

fn run(
    mut greetd: Greetd,
    command: Vec<String>,
    inbox: mpsc::Receiver<Command>,
    replies: Sender<Reply>,
) {
    // Whether an attempt is part-way through, so a second `Begin` knows to abandon it.
    // greetd allows one at a time, and would refuse the new one otherwise.
    let mut live = false;

    while let Ok(next) = inbox.recv() {
        let step = match next {
            Command::Begin { username } => {
                if live && greetd.cancel().is_err() {
                    let _ = replies.send(Reply::Broken(
                        "could not abandon the previous attempt".to_string(),
                    ));
                    return;
                }
                greetd.create_session(&username, &command)
            }
            Command::Answer(text) => {
                if !live {
                    // Nothing asked for it; answering would confuse greetd.
                    continue;
                }
                greetd.answer(Some(text))
            }
            Command::Cancel => {
                if live {
                    let _ = greetd.cancel();
                    live = false;
                }
                continue;
            }
        };

        match step {
            Ok(step) => live = pump(&mut greetd, step, &replies),
            Err(err) => {
                let _ = replies.send(Reply::Broken(err));
                return;
            }
        }
    }
}

/// Carry the conversation forward until it needs a person, or ends.
///
/// Returns whether an attempt is still outstanding.
fn pump(greetd: &mut Greetd, mut step: Step, replies: &Sender<Reply>) -> bool {
    loop {
        match step {
            // Told, not asked: show it and answer the round trip without troubling the
            // front end.
            Step::Ask(Question {
                kind: QuestionKind::Info | QuestionKind::Error,
                text,
            }) => {
                let _ = replies.send(Reply::Notice(text));
                match greetd.answer(None) {
                    Ok(next) => step = next,
                    Err(err) => {
                        let _ = replies.send(Reply::Broken(err));
                        return false;
                    }
                }
            }
            Step::Ask(question) => {
                let _ = replies.send(Reply::Question(question));
                return true;
            }
            Step::Started => {
                let _ = replies.send(Reply::Started);
                return false;
            }
            Step::Denied(reason) => {
                let _ = replies.send(Reply::Denied(reason));
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greetd_ipc::{AuthMessageType, Request, Response, codec::SyncCodec};
    use std::{os::unix::net::UnixStream, time::Duration};

    /// A scripted greetd, optionally slow to answer.
    fn scripted(replies: Vec<Response>, delay: Duration) -> (Greetd, thread::JoinHandle<()>) {
        let (ours, theirs) = UnixStream::pair().expect("socket pair");
        let server = thread::spawn(move || {
            let mut stream = theirs;
            for reply in replies {
                if Request::read_from(&mut stream).is_err() {
                    break;
                }
                thread::sleep(delay);
                if reply.write_to(&mut stream).is_err() {
                    break;
                }
            }
        });
        (Greetd::from_stream(ours), server)
    }

    fn recv(channel: &mut Channel<Reply>) -> Reply {
        // `Channel` is a calloop event source, but it is a mpsc receiver underneath;
        // for a test the blocking read is what we want.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(reply) = channel.try_recv() {
                return reply;
            }
            assert!(std::time::Instant::now() < deadline, "no reply arrived");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_password_login_reports_question_then_started() {
        let (greetd, server) = scripted(
            vec![
                Response::AuthMessage {
                    auth_message_type: AuthMessageType::Secret,
                    auth_message: "Password: ".to_string(),
                },
                Response::Success,
                Response::Success,
            ],
            Duration::ZERO,
        );
        let (handle, mut replies) = spawn(greetd, vec!["wlrix-session".to_string()]);

        handle.send(Command::Begin {
            username: "someone".to_string(),
        });
        assert!(matches!(recv(&mut replies), Reply::Question(_)));

        handle.send(Command::Answer("hunter2".to_string()));
        assert!(matches!(recv(&mut replies), Reply::Started));
        server.join().unwrap();
    }

    #[test]
    fn a_notice_is_acknowledged_without_troubling_the_front_end() {
        let (greetd, server) = scripted(
            vec![
                Response::AuthMessage {
                    auth_message_type: AuthMessageType::Info,
                    auth_message: "Password expires soon".to_string(),
                },
                Response::Success,
                Response::Success,
            ],
            Duration::ZERO,
        );
        let (handle, mut replies) = spawn(greetd, vec!["wlrix-session".to_string()]);

        handle.send(Command::Begin {
            username: "someone".to_string(),
        });
        // The notice is shown and answered by the worker; the caller never replies to it.
        assert!(matches!(recv(&mut replies), Reply::Notice(text) if text.contains("expires")));
        assert!(matches!(recv(&mut replies), Reply::Started));
        server.join().unwrap();
    }

    #[test]
    fn a_slow_pam_does_not_block_the_caller() {
        // The point of the thread. `send` must return at once even though the reply is
        // most of a second away, because the caller is drawing the screen.
        let (greetd, server) = scripted(
            vec![Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                auth_message: "Password: ".to_string(),
            }],
            Duration::from_millis(600),
        );
        let (handle, mut replies) = spawn(greetd, vec!["wlrix-session".to_string()]);

        let before = std::time::Instant::now();
        handle.send(Command::Begin {
            username: "someone".to_string(),
        });
        assert!(
            before.elapsed() < Duration::from_millis(100),
            "sending blocked for {:?}; the UI thread would have frozen",
            before.elapsed()
        );
        assert!(matches!(recv(&mut replies), Reply::Question(_)));
        assert!(
            before.elapsed() >= Duration::from_millis(500),
            "reply was not actually slow"
        );
        server.join().unwrap();
    }

    #[test]
    fn answers_are_redacted_in_debug_output() {
        // A password must not be able to reach a log through a derived `Debug`.
        let shown = format!("{:?}", Command::Answer("hunter2".to_string()));
        assert!(!shown.contains("hunter2"), "the answer leaked: {shown}");
        assert!(shown.contains("redacted"));
    }
}
