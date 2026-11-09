//! Where one block connection's input lease stands, and its taking over
//! of an owner-only block (handoff D102, D115, D151; gui-design D64).
//!
//! No I/O: [`Lease`] takes what the window and the server say and answers
//! with the requests to send, through a `send` that sends one and returns
//! its request id. A lease follows the focus (D102); on a block whose
//! input is its owner's (C77) it is asked for only after the user
//! confirmed taking it over and the owner granted it (D151).

use rusty_term::gui::source::{Refusal, Take};

/// A request the lease needs sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// `AcquireLease`.
    Acquire,
    /// `ReleaseLease`.
    Release,
    /// `RequestLease`: ask the owner for a grant.
    Request,
}

/// What the user said about taking an owner-only block over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Consent {
    /// Nothing, or they cancelled: a grant is not used.
    #[default]
    None,
    /// Confirmed, waiting for the owner's grant.
    Asking,
    /// Confirmed and granted: the lease follows the focus.
    Taking,
    /// Gave the input back; the grant may still be live.
    GivenBack,
}

/// One block connection's lease.
#[derive(Debug, Default)]
pub struct Lease {
    /// The block has the focus, so its lease is wanted.
    pub wanted: bool,
    pub held: bool,
    /// The pending `AcquireLease`.
    asking: Option<u64>,
    /// The pending `RequestLease`.
    requesting: Option<u64>,
    /// Its input is its owner's: learned from a refusal or a grant.
    gated: bool,
    /// This client holds a live grant on it.
    grant: bool,
    consent: Consent,
    refused: Option<Refusal>,
    /// The grant ended while it was in use.
    ended: bool,
    /// The place the window last gave the block.
    pub place: Option<(u16, u16)>,
    /// The size last asked for.
    pub sizing: Option<(u16, u16)>,
}

impl Lease {
    /// A lease for a block placed at `place`.
    pub fn at(place: (u16, u16)) -> Self {
        Self {
            place: Some(place),
            ..Self::default()
        }
    }

    /// What the window should show of taking the block over.
    pub fn take(&self) -> Take {
        if self.held && self.gated {
            return Take::Held;
        }
        // Granted, the lease asked for: still on its way.
        let taking = self.consent == Consent::Taking && self.asking.is_some();
        if self.consent == Consent::Asking || taking {
            return Take::Asking;
        }
        if let Some(refused) = self.refused {
            return Take::Refused(refused);
        }
        match self.ended {
            true => Take::Ended,
            false => Take::Rest,
        }
    }

    /// Whether the lease may be asked for without the user: always on an
    /// open block, and on an owner-only one only while taking it over.
    fn may_ask(&self) -> bool {
        !self.gated || (self.grant && self.consent == Consent::Taking)
    }

    /// The focus came (`wanted`) or went: ask for the lease, or give it
    /// back.
    pub fn want(&mut self, wanted: bool, send: &mut dyn FnMut(Ask) -> u64) {
        self.wanted = wanted;
        self.settle(send);
    }

    /// Ask for the lease when wanted, allowed, and neither held nor asked
    /// for; give it back when not wanted.
    fn settle(&mut self, send: &mut dyn FnMut(Ask) -> u64) {
        if self.wanted && !self.held && self.asking.is_none() && self.may_ask() {
            self.asking = Some(send(Ask::Acquire));
        } else if !self.wanted && self.held {
            send(Ask::Release);
            self.held = false;
        }
    }

    /// The answer to request `req`: done, or refused for `reason`. A
    /// refused request that isn't the lease's means input after the lease
    /// was lost.
    pub fn answered(&mut self, req: u64, refused: Option<&str>, send: &mut dyn FnMut(Ask) -> u64) {
        if self.asking == Some(req) {
            self.asking = None;
            return self.acquired(refused, send);
        }
        if self.requesting == Some(req) {
            self.requesting = None;
            return self.requested(refused, send);
        }
        if refused.is_some() {
            self.held = false;
        }
    }

    fn acquired(&mut self, refused: Option<&str>, send: &mut dyn FnMut(Ask) -> u64) {
        let Some(reason) = refused else {
            match self.wanted {
                true => self.held = true,
                // Granted after the focus moved on: given back.
                false => {
                    send(Ask::Release);
                }
            }
            return;
        };
        if !reason.starts_with("input_gated") {
            // Someone else holds it: shown as its holder (D64 4).
            if self.consent == Consent::Taking {
                self.consent = Consent::None;
            }
            return;
        }
        self.gated = true;
        self.grant = false;
        // Taking over on a grant that had ended: ask the owner (D151 3).
        if self.consent == Consent::Taking {
            self.consent = Consent::Asking;
            self.request(send);
        }
    }

    fn requested(&mut self, refused: Option<&str>, send: &mut dyn FnMut(Ask) -> u64) {
        let Some(reason) = refused else {
            return; // The owner was told; its grant comes as an event.
        };
        if self.consent != Consent::Asking {
            return;
        }
        self.consent = Consent::None;
        if reason.contains(": acquire its lease") {
            // Not owner-only after all, or this client's own block.
            self.gated = false;
            return self.settle(send);
        }
        self.refused = Some(match reason.starts_with("owner_unreachable") {
            true => Refusal::Unreachable,
            false => Refusal::Other,
        });
    }

    fn request(&mut self, send: &mut dyn FnMut(Ask) -> u64) {
        if self.requesting.is_none() {
            self.requesting = Some(send(Ask::Request));
        }
    }

    /// The lease moved to someone else (`false`) or was freed (`true`):
    /// lost, and asked for again once free while wanted (D115).
    pub fn moved(&mut self, freed: bool, send: &mut dyn FnMut(Ask) -> u64) {
        self.held = false;
        if freed {
            self.settle(send);
        }
    }

    /// This client's grant on the block began (`true`) or ended. A grant
    /// is used only while the user asked for it (D151 3).
    pub fn granted(&mut self, live: bool, send: &mut dyn FnMut(Ask) -> u64) {
        self.gated = true;
        self.grant = live;
        if live {
            if self.consent == Consent::Asking {
                self.consent = Consent::Taking;
                self.settle(send);
            }
            return;
        }
        // The server took the lease back with the grant.
        self.held = false;
        if matches!(self.consent, Consent::Taking | Consent::Asking) {
            self.ended = self.consent == Consent::Taking;
            self.consent = Consent::None;
        }
    }

    /// The user confirmed taking the block over: take it on a live grant,
    /// else ask its owner (D151 3).
    pub fn take_over(&mut self, send: &mut dyn FnMut(Ask) -> u64) {
        self.refused = None;
        self.ended = false;
        match self.grant {
            true => {
                self.consent = Consent::Taking;
                self.settle(send);
            }
            false => {
                self.consent = Consent::Asking;
                self.request(send);
            }
        }
    }

    /// The user stopped waiting for the owner; a later grant is not used.
    pub fn cancel_ask(&mut self) {
        if self.consent == Consent::Asking {
            self.consent = Consent::None;
        }
    }

    /// The user gave the input back; the grant stays the owner's to end.
    pub fn give_back(&mut self, send: &mut dyn FnMut(Ask) -> u64) {
        self.consent = Consent::GivenBack;
        if self.held {
            send(Ask::Release);
            self.held = false;
        }
    }

    /// The connection was taken over (handoff D110): what was asked is
    /// lost, the lease asked for again if wanted; an ask of the owner
    /// stands, as grants survive a reconnect (D119 4).
    pub fn revived(&mut self, send: &mut dyn FnMut(Ask) -> u64) {
        self.held = false;
        self.asking = None;
        self.requesting = None;
        self.sizing = None;
        self.settle(send);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records what was sent, numbering requests from 1.
    #[derive(Default)]
    struct Sent(Vec<Ask>);

    impl Sent {
        fn send(&mut self) -> impl FnMut(Ask) -> u64 + '_ {
            |ask| {
                self.0.push(ask);
                self.0.len() as u64
            }
        }

        fn last(&self) -> (u64, Ask) {
            let n = self.0.len();
            (n as u64, self.0[n - 1])
        }
    }

    const GATED: &str = "input_gated: block 4 takes input only with its owner's grant";

    /// A focused owner-only block whose first ask was refused.
    fn gated(sent: &mut Sent) -> Lease {
        let mut l = Lease::at((80, 24));
        l.want(true, &mut sent.send());
        let (req, ask) = sent.last();
        assert_eq!(ask, Ask::Acquire);
        l.answered(req, Some(GATED), &mut sent.send());
        assert_eq!(l.take(), Take::Rest);
        l
    }

    #[test]
    fn an_open_block_follows_the_focus() {
        let mut sent = Sent::default();
        let mut l = Lease::at((80, 24));
        l.want(true, &mut sent.send());
        l.answered(1, None, &mut sent.send());
        assert!(l.held);
        l.want(false, &mut sent.send());
        assert_eq!(sent.0, [Ask::Acquire, Ask::Release]);
        assert!(!l.held);
    }

    #[test]
    fn an_owner_only_block_is_not_asked_for_again_without_a_take_over() {
        let mut sent = Sent::default();
        let mut l = gated(&mut sent);
        l.moved(true, &mut sent.send());
        l.want(false, &mut sent.send());
        l.want(true, &mut sent.send());
        assert_eq!(sent.0, [Ask::Acquire], "no more asks");
    }

    #[test]
    fn take_over_asks_the_owner_and_takes_the_lease_once_granted() {
        let mut sent = Sent::default();
        let mut l = gated(&mut sent);
        l.take_over(&mut sent.send());
        assert_eq!(sent.last().1, Ask::Request);
        assert_eq!(l.take(), Take::Asking);
        l.answered(sent.last().0, None, &mut sent.send());
        assert_eq!(l.take(), Take::Asking, "told is not granted");
        l.granted(true, &mut sent.send());
        let (req, ask) = sent.last();
        assert_eq!(ask, Ask::Acquire);
        l.answered(req, None, &mut sent.send());
        assert_eq!(l.take(), Take::Held);
    }

    #[test]
    fn give_back_releases_and_holds_off_until_the_next_take_over() {
        let mut sent = Sent::default();
        let mut l = gated(&mut sent);
        l.take_over(&mut sent.send());
        l.granted(true, &mut sent.send());
        l.answered(sent.last().0, None, &mut sent.send());
        l.give_back(&mut sent.send());
        assert_eq!(sent.last().1, Ask::Release);
        assert_eq!(l.take(), Take::Rest);
        let before = sent.0.len();
        l.moved(true, &mut sent.send());
        l.want(false, &mut sent.send());
        l.want(true, &mut sent.send());
        assert_eq!(sent.0.len(), before, "given back stays given back");
        // The grant is still live: taking over again takes at once.
        l.take_over(&mut sent.send());
        assert_eq!(sent.last().1, Ask::Acquire);
    }

    #[test]
    fn a_take_over_on_an_ended_grant_falls_back_to_asking() {
        let mut sent = Sent::default();
        let mut l = gated(&mut sent);
        l.take_over(&mut sent.send());
        l.answered(sent.last().0, None, &mut sent.send());
        l.granted(true, &mut sent.send());
        l.answered(sent.last().0, None, &mut sent.send());
        l.give_back(&mut sent.send());
        l.take_over(&mut sent.send());
        let (req, _) = sent.last();
        l.answered(req, Some(GATED), &mut sent.send());
        assert_eq!(sent.last().1, Ask::Request);
        assert_eq!(l.take(), Take::Asking);
    }

    #[test]
    fn a_cancelled_ask_ignores_a_later_grant() {
        let mut sent = Sent::default();
        let mut l = gated(&mut sent);
        l.take_over(&mut sent.send());
        l.cancel_ask();
        assert_eq!(l.take(), Take::Rest);
        let before = sent.0.len();
        l.granted(true, &mut sent.send());
        l.moved(true, &mut sent.send());
        assert_eq!(
            sent.0.len(),
            before,
            "input is never taken without the confirm"
        );
    }

    #[test]
    fn a_refused_ask_says_why_until_the_next_take_over() {
        let mut sent = Sent::default();
        let mut l = gated(&mut sent);
        l.take_over(&mut sent.send());
        let unreachable = "owner_unreachable: block 4's owner can't be asked now";
        l.answered(sent.last().0, Some(unreachable), &mut sent.send());
        assert_eq!(l.take(), Take::Refused(Refusal::Unreachable));
        l.take_over(&mut sent.send());
        assert_eq!(l.take(), Take::Asking);
        l.answered(
            sent.last().0,
            Some("block 4 is orphaned: its owner is gone"),
            &mut sent.send(),
        );
        assert_eq!(l.take(), Take::Refused(Refusal::Other));
    }

    #[test]
    fn an_ended_grant_takes_the_lease_and_says_so() {
        let mut sent = Sent::default();
        let mut l = gated(&mut sent);
        l.take_over(&mut sent.send());
        l.granted(true, &mut sent.send());
        l.answered(sent.last().0, None, &mut sent.send());
        l.granted(false, &mut sent.send());
        assert!(!l.held);
        assert_eq!(l.take(), Take::Ended);
        l.want(true, &mut sent.send());
        assert_eq!(sent.last().1, Ask::Acquire, "the first ask only");
    }

    #[test]
    fn an_open_block_found_on_ask_is_just_leased() {
        let mut sent = Sent::default();
        let mut l = gated(&mut sent);
        l.take_over(&mut sent.send());
        let open = "block 4 takes input from anyone: acquire its lease";
        l.answered(sent.last().0, Some(open), &mut sent.send());
        assert_eq!(sent.last().1, Ask::Acquire);
        assert_eq!(l.take(), Take::Rest);
    }

    #[test]
    fn a_takeover_keeps_waiting_for_the_owner() {
        let mut sent = Sent::default();
        let mut l = gated(&mut sent);
        l.take_over(&mut sent.send());
        l.revived(&mut sent.send());
        assert_eq!(l.take(), Take::Asking);
        l.granted(true, &mut sent.send());
        assert_eq!(sent.last().1, Ask::Acquire);
    }
}
