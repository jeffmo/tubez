use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use crate::common::PeerType;
use crate::common::tube;
use crate::common::tube::TubeCompletionState;
use crate::common::UniqueId;
use super::encode;
use super::frame;

#[derive(Debug)]
pub enum FrameHandlerError {
    AbortAckFrameEncodingError(encode::FrameEncodeError),
    AbortAckTransmitError(hyper::Error),
    DuplicateAbortFrame { tube_id: u16 },
    DuplicateHasFinishedSendingFrame { tube_id: u16 },
    InappropriateHasFinishedSendingFrameFromPeer,
    PayloadAckFrameEncodingError(encode::FrameEncodeError),
    PayloadAckTransmitError(hyper::Error),
    ReceivedHasFinishedSendingAfterRemoteAbort { tube_id: u16 },
    ServerInitiatedTubesNotImplemented,
    TubeManagerInsertionError { tube_id: u16 },
    UntrackedAckId {
        tube_id: u16,
        ack_id: u16,
    },
    UntrackedTubeId(frame::Frame),
}

// TODO: When server-initiated tubes are implemented, can we generalize 
//       server_ctx into channel_ctx, pass in channel_ctx from both server and 
//       client code, and then handle NewTube event-publishing entirely here? If
//       so we could eliminate FrameHandlerResult whose sole purpose is to host 
//       FrameHandlerResult::NewTube...
pub enum FrameHandlerResult {
    FullyHandled,
    NewTube(tube::Tube),
}

pub struct FrameHandler<'a> {
    peer_type: PeerType,
    tube_managers: &'a mut Arc<Mutex<HashMap<u16, Arc<Mutex<tube::TubeManager>>>>>,
}
impl<'a> FrameHandler<'a> {
    pub fn new(
        peer_type: PeerType,
        tube_managers: &'a mut Arc<Mutex<HashMap<u16, Arc<Mutex<tube::TubeManager>>>>>,
    ) -> Self {
        FrameHandler {
            peer_type,
            tube_managers,
        }
    }

    fn get_tube_mgr(&mut self, tube_id: &u16) -> Option<Arc<Mutex<tube::TubeManager>>> {
        let tube_mgrs = self.tube_managers.lock().unwrap();
        match tube_mgrs.get(tube_id) {
            Some(tm) => Some(tm.clone()),
            None => None,
        }
    }

    pub async fn handle_frame(
        &mut self, 
        frame: frame::Frame,
        data_sender: &mut Arc<tokio::sync::Mutex<hyper::body::Sender>>,
    ) -> Result<FrameHandlerResult, FrameHandlerError> {
        match frame {
            frame::Frame::ClientHasFinishedSending { tube_id } => {
                if let PeerType::Client = self.peer_type {
                    return Err(FrameHandlerError::InappropriateHasFinishedSendingFrameFromPeer);
                }

                let tube_mgr = match self.get_tube_mgr(&tube_id) {
                    Some(tm) => tm,
                    None => return Err(FrameHandlerError::UntrackedTubeId(frame)),
                };

                let should_remove_tube_mgr = {
                    let mut tube_mgr = tube_mgr.lock().unwrap();
                    let new_state = {
                        use tube::TubeCompletionState::*;
                        match tube_mgr.completion_state {
                            Open => 
                                ClientHasFinishedSending,
                            ServerHasFinishedSending => 
                                Closed,
                            ClientHasFinishedSending | Closed => 
                                return Err(FrameHandlerError::DuplicateHasFinishedSendingFrame {
                                    tube_id,
                                }),
                            AbortedFromRemote(_) => 
                                return Err(FrameHandlerError::ReceivedHasFinishedSendingAfterRemoteAbort {
                                    tube_id,
                                }),
                            AbortedFromLocal(_) =>
                                return Ok(FrameHandlerResult::FullyHandled),
                        }
                    };

                    if tube_mgr.completion_state != new_state {
                        tube_mgr.completion_state = new_state.clone();
                        if tube_mgr.completion_state == tube::TubeCompletionState::ClientHasFinishedSending {
                            tube_mgr.pending_events.push_back(tube::TubeEvent::ClientHasFinishedSending);
                        }
                        if let Some(waker) = tube_mgr.waker.take() {
                          waker.wake();
                        }
                    }

                    tube::TubeCompletionState::Closed == new_state
                };

                if should_remove_tube_mgr {
                    self.tube_managers.lock().unwrap().remove(&tube_id);
                }
            },

            frame::Frame::Drain => {
                // TODO
            },

            // TODO: Handle NewTube headers
            frame::Frame::NewTube { tube_id, headers: _ } => {
                if let PeerType::Client = self.peer_type {
                    return Err(FrameHandlerError::ServerInitiatedTubesNotImplemented);
                }

                let tube_mgr = Arc::new(Mutex::new(tube::TubeManager::new()));
                if let Err(_) = self.tube_managers.lock().unwrap().try_insert(tube_id, tube_mgr.clone()) {
                    return Err(FrameHandlerError::TubeManagerInsertionError {
                        tube_id,
                    });
                }

                log::trace!("Emitting tube...");
                let tube_id = UniqueId::new(tube_id, None);
                let tube = tube::Tube::new(
                    self.peer_type,
                    tube_id,
                    data_sender.clone(),
                    tube_mgr,
                );

                // TODO: When server-initiated tubes are implemented, can we 
                //       generalize server_ctx into channel_ctx, pass in 
                //       channel_ctx from both server and client code, and then
                //       handle NewTube event-publishing entirely here? If so 
                //       we could eliminate FrameHandlerResult whose sole 
                //       purpose is to host FrameHandlerResult::NewTube...
                return Ok(FrameHandlerResult::NewTube(tube))
                /*
                let mut server_ctx = server_ctx.lock().unwrap();
                server_ctx.pending_events.push_back(ServerEvent::NewTube(tube));
                if let Some(waker) = server_ctx.waker.take() {
                    waker.wake();
                }
                */
            },

            frame::Frame::Payload { tube_id, ack_id, ref data } => {
                let tube_mgr = match self.get_tube_mgr(&tube_id) {
                    Some(tm) => tm,
                    None => return Err(FrameHandlerError::UntrackedTubeId(frame)),
                };

                // If an ack was requested, send one...
                if let Some(ack_id) = ack_id {
                    let frame_data = match encode::payload_ack_frame(tube_id, ack_id) {
                        Ok(data) => data,
                        Err(e) => return Err(FrameHandlerError::PayloadAckFrameEncodingError(e)),
                    };
                    let mut sender = data_sender.lock().await;
                    match sender.send_data(frame_data.into()).await {
                        Ok(_) => (),
                        Err(e) => return Err(FrameHandlerError::PayloadAckTransmitError(e)),
                    }
                }

                let mut tube_mgr = tube_mgr.lock().unwrap();
                tube_mgr.pending_events.push_back(tube::TubeEvent::Payload(data.to_vec()));
                if let Some(waker) = tube_mgr.waker.take() {
                    waker.wake();
                }
            },

            frame::Frame::PayloadAck { tube_id, ack_id } => {
                let tube_mgr = match self.get_tube_mgr(&tube_id) {
                    Some(tm) => tm,
                    None => return Err(FrameHandlerError::UntrackedTubeId(frame)),
                };

                let mut tube_mgr = tube_mgr.lock().unwrap();
                match tube_mgr.sendacks.get_mut(&ack_id) {
                    Some(res) => res.resolve(()),
                    None => return Err(FrameHandlerError::UntrackedAckId {
                        tube_id,
                        ack_id
                    }),
                };
            },

            frame::Frame::ServerHasFinishedSending { tube_id } => {
                if let PeerType::Server = self.peer_type {
                    return Err(FrameHandlerError::InappropriateHasFinishedSendingFrameFromPeer);
                }

                let tube_mgr = match self.get_tube_mgr(&tube_id) {
                    Some(tm) => tm,
                    None => return Err(FrameHandlerError::UntrackedTubeId(frame)),
                };

                let should_remove_tube_mgr = {
                    let mut tube_mgr = tube_mgr.lock().unwrap();
                    let new_state = {
                        use tube::TubeCompletionState::*;
                        match tube_mgr.completion_state {
                            Open => 
                                ServerHasFinishedSending,
                            ClientHasFinishedSending => 
                                Closed,
                            ServerHasFinishedSending | Closed =>
                                return Err(FrameHandlerError::DuplicateHasFinishedSendingFrame {
                                    tube_id,
                                }),
                            AbortedFromRemote(_) => 
                                return Err(FrameHandlerError::ReceivedHasFinishedSendingAfterRemoteAbort {
                                    tube_id,
                                }),
                            AbortedFromLocal(_) =>
                                return Ok(FrameHandlerResult::FullyHandled),
                        }
                    };

                    if tube_mgr.completion_state != new_state {
                        tube_mgr.completion_state = new_state.clone();
                        if tube_mgr.completion_state == tube::TubeCompletionState::ServerHasFinishedSending {
                            tube_mgr.pending_events.push_back(tube::TubeEvent::ServerHasFinishedSending);
                        }
                        if let Some(waker) = tube_mgr.waker.take() {
                            waker.wake();
                        }
                    }

                    tube::TubeCompletionState::Closed == new_state
                };

                if should_remove_tube_mgr {
                    self.tube_managers.lock().unwrap().remove(&tube_id);
                }
            },

            frame::Frame::Abort { tube_id, ref reason } => {
                let tube_mgr = match self.get_tube_mgr(&tube_id) {
                    Some(tm) => tm,
                    None => return Err(FrameHandlerError::UntrackedTubeId(frame)),
                };

                {
                    let mut tube_mgr = tube_mgr.lock().unwrap();
                    match tube_mgr.completion_state {
                        TubeCompletionState::AbortedFromRemote(_) =>
                            return Err(FrameHandlerError::DuplicateAbortFrame {
                                tube_id,
                            }),

                        TubeCompletionState::AbortedFromLocal(_) => (),

                        _ => {
                            tube_mgr.completion_state = 
                                TubeCompletionState::AbortedFromLocal(reason.clone());
                            tube_mgr.pending_events.push_back(tube::TubeEvent::Abort(reason.clone()));
                            if let Some(waker) = tube_mgr.waker.take() {
                                waker.wake();
                            }
                        },
                    }
                };

                self.tube_managers.lock().unwrap().remove(&tube_id);

                let abortack_frame_data = match encode::abort_ack_frame(tube_id) {
                    Ok(data) => data,
                    Err(e) => return Err(
                        FrameHandlerError::AbortAckFrameEncodingError(e)
                    ),
                };
                let mut sender = data_sender.lock().await;
                log::trace!("Sending AbortAck(tube_id={})...", tube_id);
                if let Err(e) = sender.send_data(abortack_frame_data.into()).await {
                    return Err(FrameHandlerError::AbortAckTransmitError(e));
                }
            },

            frame::Frame::AbortAck { tube_id } => {
                // It is now safe to re-use tube_id for a future new tube!
                let tube_mgr = match self.get_tube_mgr(&tube_id) {
                    Some(tm) => tm,
                    None => return Err(FrameHandlerError::UntrackedTubeId(frame)),
                };
                let mut tube_mgr = tube_mgr.lock().unwrap();
                log::trace!("Removing Tube(id={}) from list of pending Aborts.", &tube_id);
                tube_mgr.abort_pending_id_reservation = None
            },
        };

        Ok(FrameHandlerResult::FullyHandled)
    }
}
