use crate::common::frame;

#[derive(Clone, Debug, PartialEq)]
pub enum DrainReason {}

#[derive(Clone, Debug, PartialEq)]
#[allow(non_camel_case_types)]
pub enum TubeEvent_StreamError {
  InvalidTubeEventTransition(TubeEventTag, TubeEventTag),
  ServerError(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TubeEvent {
    Abort(frame::AbortReason),
    AuthenticatedAndReady,
    ClientHasFinishedSending,
    Payload(Vec<u8>),
    StreamError(TubeEvent_StreamError),
    ServerHasFinishedSending,
    ServerMustDrain(DrainReason),
}

// TODO: Is there a way to macro-ize this so TubeEvent and 
//       TubeEventTag aren't so copypasta?
#[derive(Clone, Debug, PartialEq)]
pub enum TubeEventTag {
    Abort,
    Uninitialized,
    AuthenticatedAndReady,
    Payload,
    ClientHasFinishedSending,
    StreamError,
    ServerHasFinishedSending,
    ServerMustDrain,
}
impl From<&TubeEvent> for TubeEventTag {
    fn from(event: &TubeEvent) -> Self {
        match event {
            TubeEvent::Abort(_) => TubeEventTag::Abort,
            TubeEvent::AuthenticatedAndReady => TubeEventTag::AuthenticatedAndReady,
            TubeEvent::Payload(_) => TubeEventTag::Payload,
            TubeEvent::ClientHasFinishedSending => TubeEventTag::ClientHasFinishedSending,
            TubeEvent::StreamError(_) => TubeEventTag::StreamError,
            TubeEvent::ServerMustDrain(_) => TubeEventTag::ServerMustDrain,
            TubeEvent::ServerHasFinishedSending => TubeEventTag::ServerHasFinishedSending,
        }
    }
}

pub(in crate) enum StateMachineTransitionResult {
  Valid,
  Invalid(TubeEventTag, TubeEventTag), 
}

// TODO: Decide if this is really necessary? If not...just delete it...
#[derive(Debug)]
pub(in crate) struct StateMachine {
  prev_event_tag: TubeEventTag,
}
impl StateMachine {
  pub fn new() -> Self {
    StateMachine {
      prev_event_tag: TubeEventTag::Uninitialized,
    }
  }

    pub fn transition_to(
        &mut self, 
        next_event: &TubeEvent,
    ) -> StateMachineTransitionResult {
        let next_event_tag = TubeEventTag::from(next_event);

        // TODO: Mayhaps a fancy StateMachine crate lib could be created with a 
        //       little less verbose macro syntax to describe the state machine?
        {
            use TubeEventTag::*;
            match (&self.prev_event_tag, &next_event_tag) {
                (Abort, _) =>
                    StateMachineTransitionResult::Invalid(
                        self.prev_event_tag.clone(), 
                        next_event_tag,
                    ),

                (_, Abort) => {
                    self.prev_event_tag = next_event_tag;
                    StateMachineTransitionResult::Valid
                },

                (Uninitialized, AuthenticatedAndReady) => {
                    self.prev_event_tag = next_event_tag;
                    StateMachineTransitionResult::Valid
                },

                (AuthenticatedAndReady, 
                  Payload | ClientHasFinishedSending | StreamError | 
                  ServerHasFinishedSending | ServerMustDrain) => {
                    self.prev_event_tag = next_event_tag;
                    StateMachineTransitionResult::Valid
                },

                (Payload,
                  Payload | ClientHasFinishedSending | StreamError | 
                  ServerHasFinishedSending | ServerMustDrain) => {
                    self.prev_event_tag = next_event_tag;
                    StateMachineTransitionResult::Valid
                },

                (ClientHasFinishedSending,
                  StreamError | ServerHasFinishedSending | ServerMustDrain) => {
                    self.prev_event_tag = next_event_tag;
                    StateMachineTransitionResult::Valid
                },

                (ServerMustDrain, 
                  Payload | ClientHasFinishedSending | ServerHasFinishedSending | 
                  StreamError) => {
                    self.prev_event_tag = next_event_tag;
                    StateMachineTransitionResult::Valid
                },

                // TODO: Fill this out to cover exhaustive list of transitions

                // InvalidFatal fallthrough
                (_, _) => {
                  StateMachineTransitionResult::Invalid(
                    self.prev_event_tag.clone(), next_event_tag)
                },
      }
    }
  }
}
