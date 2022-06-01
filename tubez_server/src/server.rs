use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Waker;
use std::thread;

use hyper;

use crate::tube::Tube;

enum ServerEvent {
  NewTube(Tube),
  Err(hyper::Error),
}

pub enum ServerError {
  Err(String)
}

struct ServerEventQueue {
  is_complete: bool,
  pending_events: VecDeque<ServerEvent>,
  waker: Option<Waker>,
}

pub struct Server {
    event_queue: Arc<Mutex<ServerEventQueue>>,
}
impl Server {
    #[allow(dead_code)]
    pub async fn new(addr: &SocketAddr) -> Self {
        let event_queue = Arc::new(Mutex::new(ServerEventQueue {
          is_complete: false,
          pending_events: VecDeque::new(),
          waker: None,
        }));
        let tubez_makeservice = hyper::service::make_service_fn({
          // TODO: All these move-clones are silly... 
          //       ...There's gotta be a better way? Maybe just write a custom ServiceFn?
          let event_queue = event_queue.clone(); 
          move |_conn: &hyper::server::conn::AddrStream| { 
            let event_queue = event_queue.clone(); 
            async move {
              // A new http connection has started!
              Ok::<_, std::convert::Infallible>(hyper::service::service_fn({
                let event_queue = event_queue.clone();
                move |req: hyper::Request<hyper::Body>| {
                  let event_queue = event_queue.clone();
                  async move {
                    // A new http request has started!
                    let (body_sender, body) = hyper::Body::channel();
                    let response: hyper::Response<hyper::Body> = hyper::Response::new(body);

                    let tube = Tube::new(body_sender, req);
                    let mut event_queue = event_queue.lock().unwrap();
                    // TODO: Actually authenticate the tube first...
                    event_queue.pending_events.push_back(ServerEvent::NewTube(tube));
                    if let Some(waker) = event_queue.waker.take() {
                      waker.wake();
                    }

                    // Need an explicit type annotation so the compiler 
                    // can infer the type of Error
                    let res: Result<
                      hyper::Response<hyper::Body>, 
                      std::convert::Infallible
                    > = Ok(response);
                    res
                  }
                }
              }))
            }
          }
        });

        let threadid = thread::current().id();
        println!("Starting server (threadid={:?}...", threadid);
        let hyper_server = 
            hyper::Server::bind(&addr)
                .http2_only(true)
                .serve(tubez_makeservice);

        println!("Creating Server object...");
        let server = Server {
          event_queue: event_queue.clone(),
        };

        println!("Awaiting error...");
        tokio::spawn(async move {
          if let Err(e) = hyper_server.await {
              let mut event_queue = event_queue.lock().unwrap();
              let error_msg = format!("Server error: {}", e);
              eprintln!("{}", error_msg);
              event_queue.pending_events.push_back(ServerEvent::Err(e));
              // TODO: Error all tubes here as well
              if let Some(waker) = event_queue.waker.take() {
                waker.wake();
              };
          } else {
              // TODO: Indicate that the http request has EOM'd? Not sure...
              // 
              //         "completes when the server has been shutdown"
              //         https://docs.rs/hyper/latest/hyper/server/struct.Server.html
          }
        });

        println!("Returning Server from ::new()...");
        server
    }
}
impl futures::stream::Stream for Server {
    type Item = Result<Tube, ServerError>;

    fn poll_next(
        self: core::pin::Pin<&mut Self>,
        cx: &mut futures::task::Context,
    ) -> futures::task::Poll<Option<Self::Item>> {
        let mut event_queue = self.event_queue.lock().unwrap();
        event_queue.waker = Some(cx.waker().clone());
        match event_queue.pending_events.pop_front() {
          Some(ServerEvent::NewTube(tube)) => futures::task::Poll::Ready(Some(Ok(tube))),
          Some(ServerEvent::Err(e)) => futures::task::Poll::Ready(Some(Err(
            ServerError::Err(format!("{}", e))
          ))),
          None => 
            if event_queue.is_complete {
              futures::task::Poll::Ready(None)
            } else {
              futures::task::Poll::Pending
            }
        }
    }
}

#[cfg(test)]
mod server_tests {
    use futures::StreamExt;
    use tokio;

    use super::Server;
    use super::Tube;

    // TODO: This test is silly and basically just tests the set_test_tubes 
    //       mechanics. Delete it when there's something more useful to write a 
    //       test around.
    #[tokio::test]
    async fn server_listens() {
        let addr = std::net::SocketAddr::from(([127,0,0,1], 3000));
        let mut server = Server::new(&addr).await;

        println!("Waiting for Tubes...");
        while let Some(Ok(mut tube)) = server.next().await {
          tokio::spawn(async move {
            println!("New Tube! Sending first chunk...");
            tube.send_and_forget("First chunk...\n".into());

            println!("Waiting 3s before sending second chunk...");
            tokio::time::sleep(std::time::Duration::from_millis(3000)).await;

            println!("Sending second chunk...");
            tube.send_and_forget("...second chunk.\n".into());
          });
        }
    }
}