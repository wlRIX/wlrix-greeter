// SPDX-License-Identifier: GPL-3.0-or-later
//! Talking to `greetd`.
//!
//! `greetd` owns the login: it runs as root, holds the PAM conversation, and starts the
//! session. The greeter never sees a password check, only the questions PAM wants asked
//! and the answers to hand back. That is the point of the split, and it is why this file
//! is small -- everything security-relevant happens on the other end of the socket.
//!
//! The conversation is: `CreateSession` with a username, then as many
//! `PostAuthMessageResponse` rounds as PAM asks for, then `StartSession`. A failure at
//! any point cancels the attempt and it begins again.

use std::{
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
};

use greetd_ipc::{Request, Response, codec::SyncCodec};

/// Where greetd tells greeters to find it.
const SOCKET_ENV: &str = "GREETD_SOCK";

pub struct Greetd {
    stream: UnixStream,
}

/// How a login attempt ended.
pub enum Outcome {
    /// The session was started; the greeter's work is done.
    Started,
    /// Authentication failed. The description is PAM's, and is safe to show.
    Denied(String),
}

impl Greetd {
    /// Connect to the socket greetd handed us.
    ///
    /// Being started by greetd is the only supported way to run: without the socket
    /// there is nothing to authenticate against, and a greeter that carried on would be
    /// a login prompt that cannot log anyone in.
    pub fn connect() -> Result<Self, String> {
        let path = std::env::var_os(SOCKET_ENV)
            .map(PathBuf::from)
            .ok_or_else(|| format!("{SOCKET_ENV} is not set; greetd starts this, not you"))?;
        Self::connect_to(&path)
    }

    pub fn connect_to(path: &Path) -> Result<Self, String> {
        let stream = UnixStream::connect(path)
            .map_err(|err| format!("could not connect to {}: {err}", path.display()))?;
        Ok(Self { stream })
    }

    fn round_trip(&mut self, request: Request) -> Result<Response, String> {
        request
            .write_to(&mut self.stream)
            .map_err(|err| format!("could not talk to greetd: {err}"))?;
        Response::read_from(&mut self.stream)
            .map_err(|err| format!("could not hear back from greetd: {err}"))
    }

    /// Run one login attempt from start to finish.
    ///
    /// `answer` is asked each of PAM's questions in turn. Returning `None` means the
    /// person gave up, and the attempt is canceled rather than left half-open --
    /// greetd allows only one session at a time, so an abandoned attempt would block
    /// every later one.
    pub fn login(
        &mut self,
        username: &str,
        command: &[String],
        mut answer: impl FnMut(&Question) -> Option<String>,
    ) -> Result<Outcome, String> {
        let mut response = self.round_trip(Request::CreateSession {
            username: username.to_string(),
        })?;

        loop {
            match response {
                Response::AuthMessage {
                    auth_message_type,
                    auth_message,
                } => {
                    let question = Question {
                        kind: auth_message_type.into(),
                        text: auth_message,
                    };
                    let Some(reply) = answer(&question) else {
                        self.cancel()?;
                        return Ok(Outcome::Denied("cancelled".to_string()));
                    };
                    // Info and error messages are told, not asked; PAM still expects
                    // the round trip, with nothing in it.
                    let reply = match question.kind {
                        QuestionKind::Info | QuestionKind::Error => None,
                        _ => Some(reply),
                    };
                    response =
                        self.round_trip(Request::PostAuthMessageResponse { response: reply })?;
                }
                Response::Success => {
                    // PAM is satisfied. Nothing is started until now, so this is the
                    // first point at which a session exists.
                    return match self.round_trip(Request::StartSession {
                        cmd: command.to_vec(),
                        env: Vec::new(),
                    })? {
                        Response::Success => Ok(Outcome::Started),
                        Response::Error { description, .. } => Err(description),
                        Response::AuthMessage { .. } => {
                            Err("greetd asked for more after starting".to_string())
                        }
                    };
                }
                Response::Error {
                    error_type,
                    description,
                } => {
                    // greetd cancels the attempt itself on error, so there is nothing
                    // to clean up here.
                    return match error_type {
                        greetd_ipc::ErrorType::AuthError => Ok(Outcome::Denied(description)),
                        greetd_ipc::ErrorType::Error => Err(description),
                    };
                }
            }
        }
    }

    /// Abandon an attempt, so the next one can begin.
    fn cancel(&mut self) -> Result<(), String> {
        match self.round_trip(Request::CancelSession)? {
            Response::Success => Ok(()),
            Response::Error { description, .. } => Err(description),
            Response::AuthMessage { .. } => Ok(()),
        }
    }
}

/// Something PAM wants said to the person logging in.
pub struct Question {
    pub kind: QuestionKind,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum QuestionKind {
    /// An answer that may be shown as it is typed, such as a one-time code.
    Visible,
    /// An answer that must not be, such as a password.
    Secret,
    /// Not a question: something to tell the person.
    Info,
    /// Not a question: something that went wrong.
    Error,
}

impl From<greetd_ipc::AuthMessageType> for QuestionKind {
    fn from(kind: greetd_ipc::AuthMessageType) -> Self {
        match kind {
            greetd_ipc::AuthMessageType::Visible => Self::Visible,
            greetd_ipc::AuthMessageType::Secret => Self::Secret,
            greetd_ipc::AuthMessageType::Info => Self::Info,
            greetd_ipc::AuthMessageType::Error => Self::Error,
        }
    }
}
