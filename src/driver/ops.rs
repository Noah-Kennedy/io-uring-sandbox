use crate::driver::{Driver, LifeCycle};
use futures::FutureExt;
use io_uring::squeue::Flags;
use io_uring::{cqueue, squeue};
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::{io, mem};

pub struct Unsubmitted<T, F> {
    driver: Rc<RefCell<Driver>>,
    sqe: squeue::Entry,
    state: T,
    f: F,
}

impl<F, T, Out> Unsubmitted<T, F>
where
    F: FnOnce((cqueue::Entry, T)) -> io::Result<Out>,
    T: 'static + Unpin,
{
    pub fn submit(self) -> impl Future<Output = io::Result<Out>> {
        let res = self.fire_off();

        async move {
            let op = InFlight {
                id: res?,
                driver: self.driver.clone(),
                state: Some(self.state),
            };

            op.map(self.f).await
        }
    }

    pub fn flags(&mut self, flags: Flags) {
        self.sqe = self.sqe.clone().flags(flags);
    }

    fn fire_off(&self) -> io::Result<u64> {
        unsafe {
            let mut guard = self.driver.borrow_mut();

            let res = guard.submit_event(self.sqe.clone());

            if res.is_err() {
                if guard.uring.submit()? > 0 {
                    guard.complete_events();
                }

                guard.submit_event(self.sqe.clone())
            } else {
                res
            }
        }
    }
}

pub(crate) struct InFlight<T: 'static> {
    id: u64,
    driver: Rc<RefCell<Driver>>,
    state: Option<T>,
}

impl<T> Future for InFlight<T>
where
    T: Unpin,
{
    type Output = (cqueue::Entry, T);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pinned = self.get_mut();

        let mut guard = pinned.driver.borrow_mut();

        let cycle = guard.reactor.get_mut(pinned.id as usize).unwrap();

        match cycle {
            LifeCycle::Submitted => {
                let _ = mem::replace(cycle, LifeCycle::Waiting(cx.waker().clone()));
                Poll::Pending
            }
            LifeCycle::Waiting(waker) => {
                *waker = cx.waker().clone();
                Poll::Pending
            }
            LifeCycle::Completed(_) => {
                let state = pinned.state.take().unwrap();

                if let LifeCycle::Completed(cqe) = guard.reactor.remove(pinned.id as usize) {
                    Poll::Ready((cqe, state))
                } else {
                    unimplemented!()
                }
            }
            LifeCycle::Cancelled(_) => unreachable!(),
        }
    }
}

impl<T: 'static> Drop for InFlight<T> {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let mut guard = self.driver.borrow_mut();

            if let Some(life) = guard.reactor.get_mut(self.id as usize) {
                let _ = mem::replace(life, LifeCycle::Cancelled(Box::new(state)));
            } else {
                unreachable!()
            }
        }
    }
}
