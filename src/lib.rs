#![feature(trait_alias)]
use core::fmt::Debug;
use crossterm::event::{self, EventStream};
use futures::{future::join_all, StreamExt};
use std::io;
use std::marker::Send;
use std::ops::*;
use std::time::Duration;
use tokio::{
    select,
    sync::broadcast::{self, error::*},
};

pub trait EventKind = Debug + Clone + Send;
pub trait ConvertFn<Event: EventKind> = Fn(event::Event) -> Option<Event> + Send + Sync + 'static;
pub trait StateFn<Event: EventKind> =
    FnMut(Event) -> Result<bool, io::Error> + Sync + Send + 'static;
pub trait RenderFn = Fn() -> Result<(), io::Error> + Sync + Send + 'static;

#[derive(Debug)]
enum Error<Event: EventKind> {
    RecvError(broadcast::error::RecvError),
    SendError(broadcast::error::SendError<Event>),
    IoError(io::Error),
}

impl<T: EventKind> From<broadcast::error::RecvError> for Error<T> {
    fn from(e: RecvError) -> Error<T> {
        Error::RecvError(e)
    }
}

impl<T: EventKind> From<broadcast::error::SendError<T>> for Error<T> {
    fn from(e: SendError<T>) -> Error<T> {
        Error::SendError(e)
    }
}

impl<T: EventKind> From<io::Error> for Error<T> {
    fn from(e: io::Error) -> Error<T> {
        Error::IoError(e)
    }
}

pub async fn jk_tui_main<Event: EventKind + 'static>(
    convert: impl ConvertFn<Event>,
    mut statefn: impl StateFn<Event>,
) {
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);
    let (tx, mut rx) = broadcast::channel(16);
    let renderfn: Option<Box<dyn RenderFn>> = None;

    let threads = vec![
        tokio::spawn(async move {
            let mut es = EventStream::new().map(|x| convert(x.ok()?));
            loop {
                let shutdown_init = shutdown_rx.recv();
                let event_stream = es.next();
                select! {
                    _ = shutdown_init => { break; },
                    e = event_stream => {
                        match e {
                            Some(event) => {
                                   tx.send(event).expect("failure during send!");
                            },
                            _ => {}
                        }
                    },
                }
            }
            Ok::<(), Error<Event>>(())
        }),
        tokio::spawn(async move {
            loop {
                if let Some(cte) = rx.recv().await? {
                    println!("received: {:?}", cte);
                    if statefn(cte)? {
                        shutdown_tx
                            .send(true)
                            .expect("failure during shutdown initiation!");
                        break;
                    }
                }
            }
            Ok::<(), Error<Event>>(())
        }),
        tokio::spawn(async move {
            if let Some(render) = renderfn {
                let delay = Duration::from_millis(100);
                loop {
                    render()?;
                    tokio::time::sleep(delay).await;
                }
            }
            Ok::<(), Error<Event>>(())
        }),
    ];
    join_all(threads).await;
}
