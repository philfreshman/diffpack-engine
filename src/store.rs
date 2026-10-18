use crate::types::FileMapEntry;
use futures::future::{FutureExt, LocalBoxFuture, Shared};
use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

pub type PackageFiles = Rc<HashMap<String, FileMapEntry>>;
type SharedFetch<E> = Shared<LocalBoxFuture<'static, Result<PackageFiles, E>>>;

pub struct PackageStore<E> {
    extracted: RefCell<HashMap<String, PackageFiles>>,
    in_flight: RefCell<HashMap<String, SharedFetch<E>>>,
}

impl<E: Clone + 'static> PackageStore<E> {
    pub fn new() -> Rc<Self> {
        Rc::new(PackageStore {
            extracted: RefCell::new(HashMap::new()),
            in_flight: RefCell::new(HashMap::new()),
        })
    }

    pub fn extracted(&self) -> Ref<'_, HashMap<String, PackageFiles>> {
        self.extracted.borrow()
    }

    pub async fn get_or_fetch<F>(self: &Rc<Self>, key: String, fetch: F) -> Result<PackageFiles, E>
    where
        F: Future<Output = Result<HashMap<String, FileMapEntry>, E>> + 'static,
    {
        if let Some(files) = self.extracted_package(&key) {
            return Ok(files);
        }
        self.join_or_start_fetch(key, fetch).await
    }

    fn extracted_package(&self, key: &str) -> Option<PackageFiles> {
        self.extracted.borrow().get(key).cloned()
    }

    fn join_or_start_fetch<F>(self: &Rc<Self>, key: String, fetch: F) -> SharedFetch<E>
    where
        F: Future<Output = Result<HashMap<String, FileMapEntry>, E>> + 'static,
    {
        self.in_flight
            .borrow_mut()
            .entry(key.clone())
            .or_insert_with(|| {
                Rc::clone(self)
                    .fetch_and_settle(key, fetch)
                    .boxed_local()
                    .shared()
            })
            .clone()
    }

    async fn fetch_and_settle<F>(self: Rc<Self>, key: String, fetch: F) -> Result<PackageFiles, E>
    where
        F: Future<Output = Result<HashMap<String, FileMapEntry>, E>>,
    {
        let result = fetch.await.map(Rc::new);
        self.settle(key, &result);
        result
    }

    fn settle(&self, key: String, result: &Result<PackageFiles, E>) {
        self.in_flight.borrow_mut().remove(&key);
        if let Ok(files) = result {
            self.extracted.borrow_mut().insert(key, Rc::clone(files));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileType;
    use futures::channel::oneshot;
    use futures::executor::block_on;
    use std::cell::Cell;

    const KEY: &str = "crates:itoa:1.0.18";
    const OTHER_KEY: &str = "crates:itoa:1.0.11";

    type FetchResult = Result<HashMap<String, FileMapEntry>, String>;

    fn extracted_files() -> HashMap<String, FileMapEntry> {
        HashMap::from([(
            "src/lib.rs".to_string(),
            FileMapEntry {
                file_type: FileType::File,
                content: "pub fn itoa() {}\n".to_string(),
            },
        )])
    }

    fn counted_fetch(
        fetches: &Rc<Cell<usize>>,
        result: FetchResult,
    ) -> impl Future<Output = FetchResult> + 'static {
        let fetches = Rc::clone(fetches);
        async move {
            fetches.set(fetches.get() + 1);
            result
        }
    }

    fn counted_fetch_awaiting(
        fetches: &Rc<Cell<usize>>,
        release: oneshot::Receiver<FetchResult>,
    ) -> impl Future<Output = FetchResult> + 'static {
        let fetches = Rc::clone(fetches);
        async move {
            fetches.set(fetches.get() + 1);
            release.await.expect("a fetch the test never releases ran")
        }
    }

    #[test]
    fn concurrent_loads_of_one_version_share_a_single_fetch() {
        let store = PackageStore::<String>::new();
        let fetches = Rc::new(Cell::new(0));
        let (release, released) = oneshot::channel();
        let (_, never_released) = oneshot::channel();

        let (first, second, ()) = block_on(async {
            futures::join!(
                store.get_or_fetch(KEY.to_string(), counted_fetch_awaiting(&fetches, released)),
                store.get_or_fetch(
                    KEY.to_string(),
                    counted_fetch_awaiting(&fetches, never_released)
                ),
                async { release.send(Ok(extracted_files())).unwrap() },
            )
        });

        assert_eq!(fetches.get(), 1);
        assert!(Rc::ptr_eq(&first.unwrap(), &second.unwrap()));
    }

    #[test]
    fn concurrent_loads_of_one_version_share_its_failure() {
        let store = PackageStore::<String>::new();
        let fetches = Rc::new(Cell::new(0));
        let (release, released) = oneshot::channel();
        let (_, never_released) = oneshot::channel();

        let (first, second, ()) = block_on(async {
            futures::join!(
                store.get_or_fetch(KEY.to_string(), counted_fetch_awaiting(&fetches, released)),
                store.get_or_fetch(
                    KEY.to_string(),
                    counted_fetch_awaiting(&fetches, never_released)
                ),
                async { release.send(Err("404".to_string())).unwrap() },
            )
        });

        assert_eq!(fetches.get(), 1);
        assert_eq!(first.unwrap_err(), "404");
        assert_eq!(second.unwrap_err(), "404");
    }

    #[test]
    fn concurrent_loads_of_different_versions_fetch_each_one() {
        let store = PackageStore::<String>::new();
        let fetches = Rc::new(Cell::new(0));
        let (release_first, first_released) = oneshot::channel();
        let (release_second, second_released) = oneshot::channel();

        let (first, second, (), ()) = block_on(async {
            futures::join!(
                store.get_or_fetch(
                    KEY.to_string(),
                    counted_fetch_awaiting(&fetches, first_released)
                ),
                store.get_or_fetch(
                    OTHER_KEY.to_string(),
                    counted_fetch_awaiting(&fetches, second_released)
                ),
                async { release_first.send(Ok(extracted_files())).unwrap() },
                async { release_second.send(Ok(extracted_files())).unwrap() },
            )
        });

        assert_eq!(fetches.get(), 2);
        assert!(!Rc::ptr_eq(&first.unwrap(), &second.unwrap()));
        assert!(store.extracted().contains_key(KEY));
        assert!(store.extracted().contains_key(OTHER_KEY));
    }

    #[test]
    fn a_fetch_abandoned_by_its_caller_is_finished_by_the_next_one() {
        let store = PackageStore::<String>::new();
        let fetches = Rc::new(Cell::new(0));
        let (release, released) = oneshot::channel();
        let (_, never_released) = oneshot::channel();

        let abandoned = store
            .get_or_fetch(KEY.to_string(), counted_fetch_awaiting(&fetches, released))
            .now_or_never();
        assert!(abandoned.is_none());

        let (joined, ()) = block_on(async {
            futures::join!(
                store.get_or_fetch(
                    KEY.to_string(),
                    counted_fetch_awaiting(&fetches, never_released)
                ),
                async { release.send(Ok(extracted_files())).unwrap() },
            )
        });

        assert_eq!(fetches.get(), 1);
        assert!(joined.is_ok());
        assert!(store.extracted().contains_key(KEY));
    }

    #[test]
    fn a_settled_fetch_is_no_longer_in_flight() {
        let store = PackageStore::<String>::new();
        let fetches = Rc::new(Cell::new(0));

        block_on(store.get_or_fetch(
            KEY.to_string(),
            counted_fetch(&fetches, Ok(extracted_files())),
        ))
        .unwrap();

        assert!(store.in_flight.borrow().is_empty());
    }

    #[test]
    fn a_loaded_version_is_not_fetched_again() {
        let store = PackageStore::<String>::new();
        let fetches = Rc::new(Cell::new(0));

        let first = block_on(store.get_or_fetch(
            KEY.to_string(),
            counted_fetch(&fetches, Ok(extracted_files())),
        ))
        .unwrap();
        let second = block_on(store.get_or_fetch(
            KEY.to_string(),
            counted_fetch(&fetches, Ok(extracted_files())),
        ))
        .unwrap();

        assert_eq!(fetches.get(), 1);
        assert!(Rc::ptr_eq(&first, &second));
        assert!(store.extracted().contains_key(KEY));
    }

    #[test]
    fn a_failed_fetch_is_not_cached_and_a_retry_fetches_again() {
        let store = PackageStore::<String>::new();
        let fetches = Rc::new(Cell::new(0));

        let failure = block_on(store.get_or_fetch(
            KEY.to_string(),
            counted_fetch(&fetches, Err("network down".to_string())),
        ));
        assert_eq!(failure.unwrap_err(), "network down");
        assert!(!store.extracted().contains_key(KEY));
        assert!(store.in_flight.borrow().is_empty());

        let retry = block_on(store.get_or_fetch(
            KEY.to_string(),
            counted_fetch(&fetches, Ok(extracted_files())),
        ));

        assert_eq!(fetches.get(), 2);
        assert!(retry.is_ok());
        assert!(store.extracted().contains_key(KEY));
    }
}
