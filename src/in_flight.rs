use futures::future::{FutureExt, LocalBoxFuture, Shared};
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

type SharedRequest<T> = Shared<LocalBoxFuture<'static, T>>;

pub struct InFlight<T: Clone> {
    pending: RefCell<HashMap<String, SharedRequest<T>>>,
}

impl<T: Clone + 'static> InFlight<T> {
    pub fn new() -> Rc<Self> {
        Rc::new(InFlight {
            pending: RefCell::new(HashMap::new()),
        })
    }

    pub fn join_or_start<F>(
        self: &Rc<Self>,
        key: &str,
        start: impl FnOnce() -> F,
    ) -> SharedRequest<T>
    where
        F: Future<Output = T> + 'static,
    {
        if let Some(pending) = self.pending.borrow().get(key) {
            return pending.clone();
        }
        let request = Rc::clone(self).settle_when_done(key.to_owned(), start());
        self.pending
            .borrow_mut()
            .insert(key.to_owned(), request.clone());
        request
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.pending.borrow().is_empty()
    }

    fn settle_when_done<F>(self: Rc<Self>, key: String, request: F) -> SharedRequest<T>
    where
        F: Future<Output = T> + 'static,
    {
        async move {
            let answer = request.await;
            self.pending.borrow_mut().remove(&key);
            answer
        }
        .boxed_local()
        .shared()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::oneshot;
    use futures::executor::block_on;
    use std::cell::Cell;

    const LISTING: &str = "https://proxy.test/v1/composer/p2/o/r.json";
    const OTHER_LISTING: &str = "https://proxy.test/v1/composer/p2/o/other.json";

    fn counted_fetch(
        fetches: &Rc<Cell<usize>>,
        release: oneshot::Receiver<String>,
    ) -> impl FnOnce() -> LocalBoxFuture<'static, String> {
        let fetches = Rc::clone(fetches);
        move || {
            async move {
                fetches.set(fetches.get() + 1);
                release
                    .await
                    .expect("the test releases every fetch it starts")
            }
            .boxed_local()
        }
    }

    #[test]
    fn two_requests_for_the_same_key_while_one_is_pending_share_one_fetch() {
        let fetches = Rc::new(Cell::new(0));
        let in_flight = InFlight::new();
        let (release, pending) = oneshot::channel();
        let (_unused, never_used) = oneshot::channel();

        let first = in_flight.join_or_start(LISTING, counted_fetch(&fetches, pending));
        let second = in_flight.join_or_start(LISTING, counted_fetch(&fetches, never_used));
        release.send("listing".to_string()).unwrap();

        let (first, second) = block_on(futures::future::join(first, second));
        assert_eq!(first, "listing");
        assert_eq!(second, "listing");
        assert_eq!(fetches.get(), 1);
    }

    #[test]
    fn a_settled_fetch_is_not_kept_so_the_next_request_fetches_again() {
        let fetches = Rc::new(Cell::new(0));
        let in_flight = InFlight::new();

        for answer in ["first", "second"] {
            let (release, pending) = oneshot::channel();
            let request = in_flight.join_or_start(LISTING, counted_fetch(&fetches, pending));
            release.send(answer.to_string()).unwrap();
            assert_eq!(block_on(request), answer);
        }
        assert_eq!(fetches.get(), 2);
    }

    #[test]
    fn requests_for_different_keys_fetch_separately() {
        let fetches = Rc::new(Cell::new(0));
        let in_flight = InFlight::new();
        let (release_one, one) = oneshot::channel();
        let (release_other, other) = oneshot::channel();

        let first = in_flight.join_or_start(LISTING, counted_fetch(&fetches, one));
        let second = in_flight.join_or_start(OTHER_LISTING, counted_fetch(&fetches, other));
        release_one.send("one".to_string()).unwrap();
        release_other.send("other".to_string()).unwrap();

        let (first, second) = block_on(futures::future::join(first, second));
        assert_eq!((first.as_str(), second.as_str()), ("one", "other"));
        assert_eq!(fetches.get(), 2);
    }
}
