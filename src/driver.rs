use io_uring::{cqueue, squeue, IoUring, Submitter};
use slab::Slab;
use std::io;
use tokio::sync;

pub struct Driver {
    uring: IoUring,
    reactor: Slab<Responder>,
}

pub enum Responder {
    OneShot(sync::oneshot::Sender<cqueue::Entry>),
    MultiShot(sync::broadcast::Sender<cqueue::Entry>),
}

impl Responder {
    fn one_shot(self) -> Option<sync::oneshot::Sender<cqueue::Entry>> {
        if let Self::OneShot(x) = self {
            Some(x)
        } else {
            None
        }
    }
}

impl Driver {
    pub(crate) unsafe fn submit_oneshot(
        &mut self,
        mut sqe: squeue::Entry,
        responder: Responder,
    ) -> io::Result<()> {
        let vacant = self.reactor.vacant_entry();

        sqe = sqe.user_data(vacant.key() as u64);

        let mut events = 0;

        while self.uring.submission().push(&sqe).is_err() {
            events = self.uring.submit()?;
        }

        vacant.insert(responder);

        if events > 0 {
            self.complete_events();
        }

        Ok(())
    }

    pub(crate) unsafe fn complete_events(&mut self) {
        let comp = self.uring.completion();

        for c in comp {
            let key = c.user_data();

            let entry = self
                .reactor
                .get_mut(key as usize)
                .expect("You must have put the wrong responder in!");

            match entry {
                Responder::OneShot(_) => {
                    let _ = self
                        .reactor
                        .remove(key as usize)
                        .one_shot()
                        .unwrap()
                        .send(c);
                }
                Responder::MultiShot(sender) => {
                    let _ = sender.send(c);
                }
            }
        }
    }
}
