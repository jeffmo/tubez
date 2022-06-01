use super::event::TubeEvent;
use super::event::TubeEventTag;

pub enum StateMachineTransitionResult {
  Valid,
  Invalid(TubeEventTag, TubeEventTag), 
}

pub struct StateMachine {
  prev_event_tag: TubeEventTag,
}
impl StateMachine {
  pub fn new() -> Self {
    StateMachine {
      prev_event_tag: TubeEventTag::Uninitialized,
    }
  }

  pub fn transition_to(&mut self, next_event: &TubeEvent) 
    -> StateMachineTransitionResult {
    let next_event_tag = TubeEventTag::from(next_event);

    // TODO: Mayhaps a fancy StateMachine crate lib could be created with a 
    //       little less verbose macro syntax to describe the state machine?
    {
      use TubeEventTag::*;
      match (&self.prev_event_tag, &next_event_tag) {
        (Uninitialized, AuthenticatedAndReady) => {
          self.prev_event_tag = next_event_tag;
          StateMachineTransitionResult::Valid
        },

        (AuthenticatedAndReady, 
          Payload | ClientHasFinishedSending | StreamError | ServerMustDrain) => {
          self.prev_event_tag = next_event_tag;
          StateMachineTransitionResult::Valid
        },

        (Payload,
          Payload | ClientHasFinishedSending | StreamError | ServerMustDrain) => {
          self.prev_event_tag = next_event_tag;
          StateMachineTransitionResult::Valid
        },

        (ClientHasFinishedSending,
          StreamError | ServerMustDrain) => {
          self.prev_event_tag = next_event_tag;
          StateMachineTransitionResult::Valid
        },

        (ServerMustDrain, 
          Payload | ClientHasFinishedSending | StreamError) => {
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