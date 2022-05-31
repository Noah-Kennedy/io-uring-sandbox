use io_uring::{cqueue, squeue, IoUring};
use slab::Slab;
use std::any::Any;
use std::io;
use std::task::Waker;

pub(crate) use ops::InFlight;

mod ops;

pub struct Driver {
    uring: IoUring,
    reactor: Slab<LifeCycle>,
}

enum LifeCycle {
    Submitted,
    Waiting(Waker),
    Completed(cqueue::Entry),
    Cancelled(Box<dyn Any>),
}

impl Driver {
    unsafe fn submit_event(&mut self, mut sqe: squeue::Entry) -> io::Result<u64> {
        let vacant = self.reactor.vacant_entry();

        let key = vacant.key() as u64;

        sqe = sqe.user_data(key);

        if self.uring.submission().push(&sqe).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Submission queue is full",
            ));
        }

        vacant.insert(LifeCycle::Submitted);

        Ok(key)
    }

    pub(crate) fn complete_events(&mut self) {
        let comp = self.uring.completion();

        for c in comp {
            let key = c.user_data();

            let entry = self
                .reactor
                .get_mut(key as usize)
                .expect("You must have put the wrong responder in!");

            todo!()
        }
    }
}
