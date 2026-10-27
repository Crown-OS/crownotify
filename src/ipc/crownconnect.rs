//! The outbound half: commands the cards send to the phone companion.
//!
//! crownotify is a *client* of crownconnect, and an unreliable one by nature —
//! crownconnect is a separate daemon that may be restarting, may not be installed
//! and may never come up at all. That shapes the whole module: connecting is
//! lazy, failures are absorbed, and commands issued while the service is down
//! wait in a bounded queue instead of being lost. Answering a call is the
//! motivating case; a hang-up that silently evaporated because the phone daemon
//! was restarting would be a very bad bug to have.
//!
//! Nothing here blocks. [`PhoneLink::send`] appends and returns, and
//! [`PhoneLink::pump`] does the connecting and the writing from the event
//! loop's tick.

use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{Duration, Instant},
};

use crownos_ipc::Pending;

use crate::ipc::crownconnect_protocol as crownconnect;

/// How long to wait before retrying a connection that failed.
///
/// Long enough that a permanently-absent crownconnect costs a syscall a second
/// rather than one per frame, short enough that a restart is picked up before
/// the user notices.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

/// How many commands may wait for a crownconnect that is not there.
///
/// A queue that grew without limit would turn "the phone daemon is not
/// installed" into a slow memory leak in the notification server. Sixty-four
/// is far more than a user can click through in the reconnect window, and the
/// oldest are the ones dropped: a stale seek is worth less than a fresh one.
const MAX_QUEUED: usize = 64;

/// Outbound half: the phone-companion commands the cards trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneCommand {
    Pickup(String),
    Decline(String),
    Play,
    Pause,
    Next,
    Previous,
    Seek(u8),
    Reply {
        conversation_id: String,
        text: String,
    },
}

/// Lazily-connected, non-blocking crownconnect client. Commands issued while the
/// service is down are queued and flushed on the next successful connect, so a
/// crownconnect restart never loses a hang-up.
pub struct PhoneLink {
    client: Option<crownconnect::Client>,
    queue: VecDeque<PhoneCommand>,
    /// Answering and declining are requests rather than notifications, so they
    /// leave a reply outstanding. The handles are kept only so the replies can
    /// be harvested and the client's correlation table stays bounded.
    outstanding: Vec<Pending<()>>,
    /// Set on failure; the connection is not retried before this instant.
    retry_after: Option<Instant>,
    /// Overrides the socket directory. `None` is the runtime directory, which
    /// is what everything but the tests wants.
    directory: Option<PathBuf>,
}

impl Default for PhoneLink {
    fn default() -> Self {
        Self::new()
    }
}

impl PhoneLink {
    pub fn new() -> Self {
        Self {
            client: None,
            queue: VecDeque::new(),
            outstanding: Vec::new(),
            retry_after: None,
            directory: None,
        }
    }

    /// A link that looks for crownconnect's socket in `directory`.
    ///
    /// Only the tests need this; the daemon uses the runtime directory that
    /// crownconnect actually binds in.
    pub fn in_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: Some(directory.into()),
            ..Self::new()
        }
    }

    /// Queues `command` and tries to get it out of the door immediately.
    ///
    /// Infallible on purpose. This is called from a pointer-button handler,
    /// which has nowhere sensible to put an io error and no way to ask the
    /// user about it; the failure mode that matters is losing the command, and
    /// queueing is what prevents that.
    pub fn send(&mut self, command: PhoneCommand) {
        if self.queue.len() >= MAX_QUEUED {
            let dropped = self.queue.pop_front();
            log::warn!("crownconnect queue is full; dropping {dropped:?}");
        }
        self.queue.push_back(command);
        self.pump();
    }

    /// Drives reconnection and flushing; call from the calloop tick.
    pub fn pump(&mut self) {
        self.harvest();

        if self.client.is_none() && !self.queue.is_empty() {
            self.connect();
        }

        while let Some(command) = self.queue.pop_front() {
            let Some(client) = self.client.as_mut() else {
                self.queue.push_front(command);
                return;
            };
            match dispatch(client, &command) {
                Ok(pending) => self.outstanding.extend(pending),
                Err(error) => {
                    // The command goes back to the head of the queue so the
                    // next successful connect replays it.
                    log::warn!("crownconnect command failed ({error}); reconnecting");
                    self.queue.push_front(command);
                    self.disconnect();
                    return;
                }
            }
        }

        let Some(client) = self.client.as_mut() else {
            return;
        };
        if let Err(error) = client.handle_writable() {
            log::warn!("flushing to crownconnect failed ({error}); reconnecting");
            self.disconnect();
        }
    }

    /// Whether a connection to crownconnect is currently up.
    pub fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    /// Commands still waiting for a crownconnect to send them to.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    fn connect(&mut self) {
        if self.retry_after.is_some_and(|at| Instant::now() < at) {
            return;
        }
        let connected = match &self.directory {
            Some(directory) => crownconnect::Client::connect_in(directory),
            None => crownconnect::Client::connect(),
        };
        match connected {
            Ok(client) => {
                log::debug!(
                    "connected to crownconnect; flushing {} commands",
                    self.queue.len()
                );
                self.client = Some(client);
                self.retry_after = None;
            }
            Err(error) => {
                log::debug!(
                    "crownconnect is not answering ({error}); {} commands queued",
                    self.queue.len()
                );
                self.retry_after = Some(Instant::now() + RECONNECT_INTERVAL);
            }
        }
    }

    fn disconnect(&mut self) {
        self.client = None;
        self.outstanding.clear();
        self.retry_after = Some(Instant::now() + RECONNECT_INTERVAL);
    }

    /// Reads whatever crownconnect has replied and retires the handles.
    ///
    /// Nothing here needs the answers, but somebody has to read them: an
    /// unread socket eventually applies backpressure to crownconnect, and an
    /// unresolved handle stays in the client's correlation table forever.
    fn harvest(&mut self) {
        let Some(client) = self.client.as_mut() else {
            return;
        };
        if let Err(error) = client.handle_readable() {
            log::debug!("crownconnect connection lost while reading: {error}");
            self.disconnect();
            return;
        }
        let mut outstanding = std::mem::take(&mut self.outstanding);
        outstanding.retain(|pending| match client.take(pending) {
            Ok(None) => true,
            Ok(Some(())) => false,
            Err(error) => {
                log::debug!("crownconnect rejected a call action: {error}");
                false
            }
        });
        self.outstanding = outstanding;
    }
}

/// Sends one command, returning the reply handle when the command has one.
///
/// Everything but answering and declining a call is fire-and-forget, which is
/// what makes the queue safe to replay: a duplicated `media_pause` is
/// harmless, whereas a duplicated pickup would be answered twice.
fn dispatch(
    client: &mut crownconnect::Client,
    command: &PhoneCommand,
) -> Result<Option<Pending<()>>, crownos_ipc::Error> {
    match command {
        PhoneCommand::Pickup(call_id) => client.pickup_call(call_id.clone()).map(Some),
        PhoneCommand::Decline(call_id) => client.decline_call(call_id.clone()).map(Some),
        PhoneCommand::Play => client.media_play().map(|()| None),
        PhoneCommand::Pause => client.media_pause().map(|()| None),
        PhoneCommand::Next => client.media_next().map(|()| None),
        PhoneCommand::Previous => client.media_previous().map(|()| None),
        PhoneCommand::Seek(percent) => client.media_seek(*percent).map(|()| None),
        PhoneCommand::Reply {
            conversation_id,
            text,
        } => client
            .send_reply(conversation_id.clone(), text.clone())
            .map(|()| None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_sent_with_crownconnect_absent_is_queued_not_lost() {
        let directory = tempfile::tempdir().expect("temp dir");
        let mut link = PhoneLink::in_directory(directory.path());

        link.send(PhoneCommand::Pickup("call-42".into()));

        assert!(
            !link.is_connected(),
            "there is no crownconnect to connect to"
        );
        assert_eq!(link.queued(), 1, "the pickup must survive for the retry");
    }

    #[test]
    fn a_permanently_absent_crownconnect_cannot_grow_the_queue_without_limit() {
        let directory = tempfile::tempdir().expect("temp dir");
        let mut link = PhoneLink::in_directory(directory.path());

        for percent in 0..(MAX_QUEUED as u8 + 10) {
            link.send(PhoneCommand::Seek(percent.min(100)));
        }

        assert_eq!(
            link.queued(),
            MAX_QUEUED,
            "the queue is bounded, got {}",
            link.queued()
        );
        assert_eq!(
            link.queue.front(),
            Some(&PhoneCommand::Seek(10)),
            "the oldest commands are the ones dropped"
        );
    }

    #[test]
    fn pumping_without_a_command_to_send_does_not_reconnect_every_tick() {
        let directory = tempfile::tempdir().expect("temp dir");
        let mut link = PhoneLink::in_directory(directory.path());

        link.pump();
        assert!(
            link.retry_after.is_none(),
            "an empty queue has nothing to connect for"
        );
    }

    #[test]
    fn a_failed_connection_is_not_retried_until_the_interval_has_passed() {
        let directory = tempfile::tempdir().expect("temp dir");
        let mut link = PhoneLink::in_directory(directory.path());

        link.send(PhoneCommand::Play);
        let first = link.retry_after.expect("the failed connect armed a retry");
        link.pump();

        assert_eq!(
            link.retry_after,
            Some(first),
            "pumping again inside the interval must not re-attempt"
        );
    }
}
