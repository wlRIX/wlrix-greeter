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
//!
//! It is exposed a step at a time rather than as one call, because a graphical greeter
//! cannot block in a callback waiting for someone to click. [`Greetd::login`] keeps the
//! all-in-one shape for the terminal face, built on the same steps.
//!
//! **How many rounds there are, and whether a password is one of them, is PAM's to
//! decide.** The greeter must never predict it: an account with no password simply
//! returns [`Step::Started`] without ever asking. That is why this asks nothing of the
//! system -- it could not read `/etc/shadow` as the `greeter` user anyway.

use std::{
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
};

use greetd_ipc::{Request, Response, codec::SyncCodec};

/// Where greetd tells greeters to find it.
const SOCKET_ENV: &str = "GREETD_SOCK";

pub struct Greetd {
    stream: UnixStream,
    /// What to start once PAM is satisfied. Held from `create_session`, because the
    /// success that triggers it may be several answers away.
    command: Vec<String>,
    /// The question waiting to be answered, if any. Kept so `answer` can send what the
    /// protocol expects for its kind rather than trusting the caller to remember.
    pending: Option<QuestionKind>,
}

/// How far a login attempt has got.
pub enum Step {
    /// PAM wants something asked, or told. Answer it with [`Greetd::answer`].
    Ask(Question),
    /// The session is running. The greeter's work is done.
    Started,
    /// Refused. The description is PAM's, and is safe to show.
    Denied(String),
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
        Ok(Self::from_stream(stream))
    }

    /// Wrap an already-connected socket. The worker and the tests both build
    /// their `Greetd` this way.
    pub(crate) fn from_stream(stream: UnixStream) -> Self {
        Self {
            stream,
            command: Vec::new(),
            pending: None,
        }
    }

    fn round_trip(&mut self, request: Request) -> Result<Response, String> {
        request
            .write_to(&mut self.stream)
            .map_err(|err| format!("could not talk to greetd: {err}"))?;
        Response::read_from(&mut self.stream)
            .map_err(|err| format!("could not hear back from greetd: {err}"))
    }

    /// Begin an attempt. `command` is what a success eventually starts.
    ///
    /// Returns the first step: a question, an outright refusal, or -- for an account
    /// with no password -- a session already running.
    pub fn create_session(&mut self, username: &str, command: &[String]) -> Result<Step, String> {
        self.command = command.to_vec();
        self.pending = None;
        let response = self.round_trip(Request::CreateSession {
            username: username.to_string(),
        })?;
        self.advance(response)
    }

    /// Answer the outstanding question.
    ///
    /// Info and error notices are told, not asked: PAM still expects the round trip,
    /// but with nothing in it. That is decided here from the question's own kind rather
    /// than left to the caller, so a front end cannot get it wrong.
    pub fn answer(&mut self, reply: Option<String>) -> Result<Step, String> {
        let reply = match self.pending {
            Some(QuestionKind::Info | QuestionKind::Error) => None,
            _ => reply,
        };
        let response = self.round_trip(Request::PostAuthMessageResponse { response: reply })?;
        self.advance(response)
    }

    /// Interpret whatever greetd just said.
    fn advance(&mut self, response: Response) -> Result<Step, String> {
        match response {
            Response::AuthMessage {
                auth_message_type,
                auth_message,
            } => {
                let kind = QuestionKind::from(auth_message_type);
                self.pending = Some(kind);
                Ok(Step::Ask(Question {
                    kind,
                    text: auth_message,
                }))
            }
            Response::Success => {
                // PAM is satisfied. Nothing is started until now, so this is the first
                // point at which a session exists. Starting stays in here so that no
                // caller can authenticate and then forget to.
                self.pending = None;
                match self.round_trip(Request::StartSession {
                    cmd: self.command.clone(),
                    env: Vec::new(),
                })? {
                    Response::Success => Ok(Step::Started),
                    Response::Error { description, .. } => Err(description),
                    Response::AuthMessage { .. } => {
                        Err("greetd asked for more after starting".to_string())
                    }
                }
            }
            Response::Error {
                error_type,
                description,
            } => {
                // greetd cancels the attempt itself on error, so there is nothing to
                // clean up here.
                self.pending = None;
                match error_type {
                    greetd_ipc::ErrorType::AuthError => Ok(Step::Denied(description)),
                    greetd_ipc::ErrorType::Error => Err(description),
                }
            }
        }
    }

    /// Abandon an attempt, so the next one can begin.
    ///
    /// greetd allows one at a time, so an attempt left half-open blocks every later one.
    pub fn cancel(&mut self) -> Result<(), String> {
        self.pending = None;
        match self.round_trip(Request::CancelSession)? {
            Response::Success => Ok(()),
            Response::Error { description, .. } => Err(description),
            Response::AuthMessage { .. } => Ok(()),
        }
    }

    /// Run an attempt from start to finish, asking `answer` each question in turn.
    ///
    /// The terminal face's shape, kept because blocking is exactly what it wants.
    /// `None` from the closure means the person gave up.
    pub fn login(
        &mut self,
        username: &str,
        command: &[String],
        mut answer: impl FnMut(&Question) -> Option<String>,
    ) -> Result<Outcome, String> {
        let mut step = self.create_session(username, command)?;
        loop {
            match step {
                Step::Ask(question) => {
                    let Some(reply) = answer(&question) else {
                        self.cancel()?;
                        return Ok(Outcome::Denied("cancelled".to_string()));
                    };
                    step = self.answer(Some(reply))?;
                }
                Step::Started => return Ok(Outcome::Started),
                Step::Denied(reason) => return Ok(Outcome::Denied(reason)),
            }
        }
    }
}

/// Something PAM wants said to the person logging in.
#[derive(Debug)]
pub struct Question {
    pub kind: QuestionKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use greetd_ipc::{AuthMessageType, ErrorType, codec::SyncCodec};
    use std::thread;

    /// A scripted greetd on the other end of a socket pair.
    ///
    /// Every reply is decided in advance, so a test says exactly what PAM does and the
    /// state machine is checked against it. The requests it received come back at the
    /// end, which is how the tests below assert what went *out* -- the wire is the
    /// contract, not just the return values.
    fn scripted(replies: Vec<Response>) -> (Greetd, thread::JoinHandle<Vec<Request>>) {
        let (ours, theirs) = UnixStream::pair().expect("socket pair");
        let server = thread::spawn(move || {
            let mut stream = theirs;
            let mut seen = Vec::new();
            for reply in replies {
                match Request::read_from(&mut stream) {
                    Ok(request) => seen.push(request),
                    // The client finished early; that is a legitimate end.
                    Err(_) => break,
                }
                if reply.write_to(&mut stream).is_err() {
                    break;
                }
            }
            seen
        });
        (Greetd::from_stream(ours), server)
    }

    fn ask(kind: AuthMessageType, text: &str) -> Response {
        Response::AuthMessage {
            auth_message_type: kind,
            auth_message: text.to_string(),
        }
    }

    fn session() -> Vec<String> {
        vec!["wlrix-session".to_string()]
    }

    #[test]
    fn an_account_with_no_password_starts_without_being_asked() {
        // The whole reason the greeter never inspects /etc/shadow: PAM answering
        // Success straight away *is* "this account has no password".
        let (mut greetd, server) = scripted(vec![Response::Success, Response::Success]);
        let step = greetd.create_session("someone", &session()).unwrap();
        assert!(matches!(step, Step::Started));

        let seen = server.join().unwrap();
        assert!(matches!(seen[0], Request::CreateSession { .. }));
        assert!(
            matches!(&seen[1], Request::StartSession { cmd, .. } if cmd == &session()),
            "a success must start the session, with the command it was given"
        );
    }

    #[test]
    fn a_password_is_asked_for_then_accepted() {
        let (mut greetd, server) = scripted(vec![
            ask(AuthMessageType::Secret, "Password: "),
            Response::Success,
            Response::Success,
        ]);

        let step = greetd.create_session("someone", &session()).unwrap();
        let Step::Ask(question) = step else {
            panic!("expected a question");
        };
        assert_eq!(question.kind, QuestionKind::Secret);

        assert!(matches!(
            greetd.answer(Some("hunter2".into())).unwrap(),
            Step::Started
        ));

        let seen = server.join().unwrap();
        assert!(
            matches!(&seen[1], Request::PostAuthMessageResponse { response }
                     if response.as_deref() == Some("hunter2")),
            "the answer must reach greetd verbatim"
        );
    }

    #[test]
    fn notices_are_answered_with_nothing_whatever_the_caller_passes() {
        // Info and error messages are told, not asked. Sending PAM a body for one is a
        // protocol error, so the kind decides -- not the front end.
        let (mut greetd, server) = scripted(vec![
            ask(AuthMessageType::Info, "Password expires soon"),
            Response::Success,
            Response::Success,
        ]);
        greetd.create_session("someone", &session()).unwrap();
        greetd.answer(Some("ignored".into())).unwrap();

        let seen = server.join().unwrap();
        assert!(
            matches!(&seen[1], Request::PostAuthMessageResponse { response } if response.is_none()),
            "a notice must be acknowledged with no body"
        );
    }

    #[test]
    fn a_wrong_password_is_denied_not_an_error() {
        // Denied is an ordinary outcome the greeter recovers from by asking again;
        // Err means the socket or greetd itself is broken and it cannot continue.
        let (mut greetd, server) = scripted(vec![
            ask(AuthMessageType::Secret, "Password: "),
            Response::Error {
                error_type: ErrorType::AuthError,
                description: "Login incorrect".to_string(),
            },
        ]);
        greetd.create_session("someone", &session()).unwrap();
        match greetd.answer(Some("wrong".into())).unwrap() {
            Step::Denied(reason) => assert_eq!(reason, "Login incorrect"),
            _ => panic!("a bad password should be denied, not fatal"),
        }
        server.join().unwrap();
    }

    #[test]
    fn a_greetd_level_failure_is_fatal() {
        let (mut greetd, server) = scripted(vec![Response::Error {
            error_type: ErrorType::Error,
            description: "no such user".to_string(),
        }]);
        assert!(greetd.create_session("nobody", &session()).is_err());
        server.join().unwrap();
    }

    #[test]
    fn login_still_drives_the_whole_conversation() {
        // The terminal face's path, now built on the steps above. It must behave as it
        // did before the split.
        let (mut greetd, server) = scripted(vec![
            ask(AuthMessageType::Secret, "Password: "),
            Response::Success,
            Response::Success,
        ]);
        let outcome = greetd
            .login("someone", &session(), |q| {
                assert_eq!(q.kind, QuestionKind::Secret);
                Some("hunter2".to_string())
            })
            .unwrap();
        assert!(matches!(outcome, Outcome::Started));
        server.join().unwrap();
    }

    #[test]
    fn giving_up_cancels_rather_than_leaving_the_attempt_open() {
        // greetd allows one attempt at a time, so abandoning without cancelling would
        // block every later login.
        let (mut greetd, server) = scripted(vec![
            ask(AuthMessageType::Secret, "Password: "),
            Response::Success,
        ]);
        let outcome = greetd.login("someone", &session(), |_| None).unwrap();
        assert!(matches!(outcome, Outcome::Denied(_)));

        let seen = server.join().unwrap();
        assert!(
            matches!(seen[1], Request::CancelSession),
            "giving up must cancel the attempt"
        );
    }
}
