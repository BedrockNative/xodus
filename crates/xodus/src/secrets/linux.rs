use dbus_secret_service::{EncryptionType, Error, SecretService};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(super) enum SetupError {
    #[error(
        "The desktop password service is unavailable. Enable GNOME Keyring or a Secret Service-compatible KWallet in your desktop session, then retry. Do not run Xodus with sudo."
    )]
    Unavailable,
    #[error(
        "Keyring unlock or setup was cancelled or timed out. Retry the operation to reopen the desktop password prompt. Credentials were not accessed."
    )]
    Cancelled,
    #[error(
        "The desktop could not create or unlock the password keyring. Check that its graphical password prompt is installed and available, then retry Sign in. Credentials were not saved to a plaintext fallback."
    )]
    Access,
    #[error(
        "No default password keyring exists. Sign in through Xodus to create one using the desktop password dialog, then start the service again."
    )]
    Missing,
}

#[derive(Clone, Copy)]
pub(super) enum Preparation {
    Login,
    Service,
}

enum State {
    Missing,
    Locked,
    Ready,
}

trait Keyring {
    fn state(&self) -> Result<State, SetupError>;
    fn create(&self) -> Result<(), SetupError>;
    fn unlock(&self) -> Result<(), SetupError>;
}

fn ensure_ready(keyring: &impl Keyring, preparation: Preparation) -> Result<(), SetupError> {
    match keyring.state()? {
        State::Missing => match preparation {
            Preparation::Login => keyring.create(),
            Preparation::Service => Err(SetupError::Missing),
        },
        State::Locked => keyring.unlock(),
        State::Ready => Ok(()),
    }
}

struct DesktopKeyring(SecretService);

fn access_error(error: Error) -> SetupError {
    match error {
        Error::Prompt => SetupError::Cancelled,
        _ => SetupError::Access,
    }
}

impl Keyring for DesktopKeyring {
    fn state(&self) -> Result<State, SetupError> {
        match self.0.get_default_collection() {
            Ok(collection) => Ok(if collection.is_locked().map_err(access_error)? {
                State::Locked
            } else {
                State::Ready
            }),
            Err(Error::NoResult) => Ok(State::Missing),
            Err(error) => Err(access_error(error)),
        }
    }

    fn create(&self) -> Result<(), SetupError> {
        eprintln!(
            "Create a password keyring in the desktop dialog to keep your Xodus sign-in secure."
        );
        // The alias makes creation race-safe. Never overwrite another default
        // collection, pick the volatile session collection, or choose a password.
        self.0
            .create_collection("Login", "default")
            .map_err(access_error)?
            .ensure_unlocked()
            .map_err(access_error)
    }

    fn unlock(&self) -> Result<(), SetupError> {
        eprintln!("Unlock your password keyring in the desktop dialog to continue.");
        self.0
            .get_default_collection()
            .map_err(access_error)?
            .ensure_unlocked()
            .map_err(access_error)
    }
}

pub(super) fn prepare(preparation: Preparation) -> Result<(), SetupError> {
    let service = SecretService::connect_with_max_prompt_timeout(EncryptionType::Dh, 300)
        .map_err(|_| SetupError::Unavailable)?;
    ensure_ready(&DesktopKeyring(service), preparation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Mock {
        state: Result<State, SetupError>,
        fail_prompt: bool,
        calls: RefCell<Vec<&'static str>>,
    }
    impl Keyring for Mock {
        fn state(&self) -> Result<State, SetupError> {
            match &self.state {
                Ok(State::Missing) => Ok(State::Missing),
                Ok(State::Locked) => Ok(State::Locked),
                Ok(State::Ready) => Ok(State::Ready),
                Err(_) => Err(SetupError::Access),
            }
        }
        fn create(&self) -> Result<(), SetupError> {
            self.calls.borrow_mut().push("create");
            if self.fail_prompt {
                Err(SetupError::Cancelled)
            } else {
                Ok(())
            }
        }
        fn unlock(&self) -> Result<(), SetupError> {
            self.calls.borrow_mut().push("unlock");
            if self.fail_prompt {
                Err(SetupError::Cancelled)
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn only_a_missing_keyring_is_created_and_only_a_locked_one_is_unlocked() {
        for (state, expected) in [
            (State::Missing, vec!["create"]),
            (State::Locked, vec!["unlock"]),
            (State::Ready, vec![]),
        ] {
            let mock = Mock {
                state: Ok(state),
                fail_prompt: false,
                calls: RefCell::default(),
            };
            assert!(ensure_ready(&mock, Preparation::Login).is_ok());
            assert_eq!(*mock.calls.borrow(), expected);
        }
    }

    #[test]
    fn access_failure_does_not_create_a_replacement_keyring() {
        let mock = Mock {
            state: Err(SetupError::Access),
            fail_prompt: false,
            calls: RefCell::default(),
        };
        assert_eq!(
            ensure_ready(&mock, Preparation::Login),
            Err(SetupError::Access)
        );
        assert!(mock.calls.borrow().is_empty());
    }

    #[test]
    fn dismissed_prompt_stops_setup() {
        for state in [State::Missing, State::Locked] {
            let mock = Mock {
                state: Ok(state),
                fail_prompt: true,
                calls: RefCell::default(),
            };
            assert_eq!(
                ensure_ready(&mock, Preparation::Login),
                Err(SetupError::Cancelled)
            );
            assert_eq!(mock.calls.borrow().len(), 1);
        }
    }

    #[test]
    fn service_unlocks_existing_keyring_without_creating_a_missing_one() {
        for (state, expected, calls) in [
            (Ok(State::Locked), Ok(()), vec!["unlock"]),
            (Ok(State::Ready), Ok(()), vec![]),
            (Ok(State::Missing), Err(SetupError::Missing), vec![]),
            (Err(SetupError::Access), Err(SetupError::Access), vec![]),
        ] {
            let mock = Mock {
                state,
                fail_prompt: false,
                calls: RefCell::default(),
            };
            assert_eq!(ensure_ready(&mock, Preparation::Service), expected);
            assert_eq!(*mock.calls.borrow(), calls);
        }
    }

    #[test]
    fn cancelled_service_unlock_does_not_retry_or_create_a_keyring() {
        let mock = Mock {
            state: Ok(State::Locked),
            fail_prompt: true,
            calls: RefCell::default(),
        };
        assert_eq!(
            ensure_ready(&mock, Preparation::Service),
            Err(SetupError::Cancelled)
        );
        assert_eq!(*mock.calls.borrow(), vec!["unlock"]);
    }
}
