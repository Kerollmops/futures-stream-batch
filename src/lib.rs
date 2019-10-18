// This supplements the `futures-preview` provided `chunks` operator[1] with a
// an additional delay to flush the inner vector.
//
// [1]: https://github.com/rust-lang-nursery/futures-rs/blob/4613193023dd4071bbd32b666e3b85efede3a725/futures-util/src/stream/chunks.rs

use core::mem;
use core::pin::Pin;
use futures::stream::{Fuse, FusedStream, Stream};
use futures::task::{Context, Poll};
use futures::{StreamExt, FutureExt};
use futures::future::OptionFuture;
use pin_utils::{unsafe_pinned, unsafe_unpinned};

use std::time::Duration;
use futures_timer::Delay;

impl<T: ?Sized> ChunksTimeoutStreamExt for T where T: Stream {}

pub trait ChunksTimeoutStreamExt: Stream {
    fn chunks_timeout(self, capacity: usize, duration: Duration) -> ChunksTimeout<Self>
    where
        Self: Sized,
    {
        ChunksTimeout::new(self, capacity, duration)
    }
}

#[derive(Debug)]
#[must_use = "streams do nothing unless polled"]
pub struct ChunksTimeout<St: Stream> {
    stream: Fuse<St>,
    items: Vec<St::Item>,
    cap: usize,
    // https://github.com/rust-lang-nursery/futures-rs/issues/1475
    clock: OptionFuture<Delay>,
    duration: Duration,
}

impl<St: Unpin + Stream> Unpin for ChunksTimeout<St> {}

impl<St: Stream> ChunksTimeout<St>
where
    St: Stream,
{
    unsafe_unpinned!(items: Vec<St::Item>);
    unsafe_unpinned!(clock: OptionFuture<Delay>);
    unsafe_pinned!(stream: Fuse<St>);

    pub fn new(stream: St, capacity: usize, duration: Duration) -> ChunksTimeout<St> {
        assert!(capacity > 0);

        ChunksTimeout {
            stream: stream.fuse(),
            items: Vec::with_capacity(capacity),
            cap: capacity,
            clock: OptionFuture::from(None),
            duration,
        }
    }

    fn take(mut self: Pin<&mut Self>) -> Vec<St::Item> {
        let cap = self.cap;
        mem::replace(self.as_mut().items(), Vec::with_capacity(cap))
    }

    /// Acquires a reference to the underlying stream that this combinator is
    /// pulling from.
    pub fn get_ref(&self) -> &St {
        self.stream.get_ref()
    }

    /// Acquires a mutable reference to the underlying stream that this
    /// combinator is pulling from.
    ///
    /// Note that care must be taken to avoid tampering with the state of the
    /// stream which may otherwise confuse this combinator.
    pub fn get_mut(&mut self) -> &mut St {
        self.stream.get_mut()
    }

    /// Acquires a pinned mutable reference to the underlying stream that this
    /// combinator is pulling from.
    ///
    /// Note that care must be taken to avoid tampering with the state of the
    /// stream which may otherwise confuse this combinator.
    pub fn get_pin_mut(self: Pin<&mut Self>) -> Pin<&mut St> {
        self.stream().get_pin_mut()
    }

    /// Consumes this combinator, returning the underlying stream.
    ///
    /// Note that this may discard intermediate state of this combinator, so
    /// care should be taken to avoid losing resources when this is called.
    pub fn into_inner(self) -> St {
        self.stream.into_inner()
    }
}

impl<St: Stream> Stream for ChunksTimeout<St> {
    type Item = Vec<St::Item>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match self.as_mut().stream().poll_next(cx) {
                Poll::Ready(item) => match item {
                    // Push the item into the buffer and check whether it is full.
                    // If so, replace our buffer with a new and empty one and return
                    // the full one.
                    Some(item) => {
                        if self.items.is_empty() {
                            *self.as_mut().clock() = Some(Delay::new(self.duration)).into();
                        }
                        self.as_mut().items().push(item);
                        if self.items.len() >= self.cap {
                            *self.as_mut().clock() = None.into();
                            return Poll::Ready(Some(self.as_mut().take()));
                        } else {
                            // Continue the loop
                            continue;
                        }
                    }

                    // Since the underlying stream ran out of values, return what we
                    // have buffered, if we have anything.
                    None => {
                        let last = if self.items.is_empty() {
                            None
                        } else {
                            Some(mem::replace(self.as_mut().items(), Vec::new()))
                        };

                        return Poll::Ready(last);
                    }
                },
                // Don't return here, as we need to check the clock.
                Poll::Pending => (),
            }

            match self.as_mut().clock().poll_unpin(cx) {
                Poll::Ready(Some(())) => {
                    *self.as_mut().clock() = None.into();
                    return Poll::Ready(Some(self.as_mut().take()));
                }
                Poll::Ready(None) => {
                    debug_assert!(
                        self.as_mut().items().is_empty(),
                        "Inner buffer is empty, but clock is available."
                    );
                }
                Poll::Pending => (),
            }

            return Poll::Pending;
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let chunk_len = if self.items.is_empty() { 0 } else { 1 };
        let (lower, upper) = self.stream.size_hint();
        let lower = lower.saturating_add(chunk_len);
        let upper = match upper {
            Some(x) => x.checked_add(chunk_len),
            None => None,
        };
        (lower, upper)
    }
}

impl<St: FusedStream> FusedStream for ChunksTimeout<St> {
    fn is_terminated(&self) -> bool {
        self.stream.is_terminated() & self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::iter;
    use std::time::{Duration, Instant};
    use futures::executor::ThreadPool;
    use futures::future;
    use futures::{stream, FutureExt, StreamExt};

    #[test]
    fn messages_pass_through() {
        let v = stream::iter(iter::once(5))
            .chunks_timeout(5, Duration::new(1, 0))
            .collect::<Vec<_>>();

        ThreadPool::new().unwrap().run(
            v.then(|x| {
                assert_eq!(vec![vec![5]], x);
                future::ready(())
            })
            .unit_error()
            .boxed()
        )
        .unwrap()
    }

    #[test]
    fn message_chunks() {
        let iter = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9].into_iter();
        let stream = stream::iter(iter);

        let chunk_stream = ChunksTimeout::new(stream, 5, Duration::new(1, 0));

        let v = chunk_stream.collect::<Vec<_>>();
        ThreadPool::new().unwrap().run(
            v.then(|res| {
                assert_eq!(vec![vec![0, 1, 2, 3, 4], vec![5, 6, 7, 8, 9]], res);
                future::ready(())
            })
            .unit_error()
            .boxed()
        )
        .unwrap()
    }

    #[test]
    fn message_early_exit() {
        let iter = vec![1, 2, 3, 4].into_iter();
        let stream = stream::iter(iter);

        let chunk_stream = ChunksTimeout::new(stream, 5, Duration::new(1, 0));

        let v = chunk_stream.collect::<Vec<_>>();
        ThreadPool::new().unwrap().run(
            v.then(|res| {
                assert_eq!(vec![vec![1, 2, 3, 4]], res);
                future::ready(())
            })
            .unit_error()
            .boxed()
        )
        .unwrap()
    }

    // TODO: use the `tokio-test` and `futures-test-preview` crates
    #[test]
    fn message_timeout() {
        let iter = vec![1, 2, 3, 4].into_iter();
        let stream0 = stream::iter(iter);

        let iter = vec![5].into_iter();
        let stream1 = stream::iter(iter).then(move |n| {
            Delay::new(Duration::from_millis(300)).map(move |_| n)
        });

        let iter = vec![6, 7, 8].into_iter();
        let stream2 = stream::iter(iter);

        let stream = stream0.chain(stream1).chain(stream2);
        let chunk_stream = ChunksTimeout::new(stream, 5, Duration::from_millis(100));

        let now = Instant::now();
        let min_times = [Duration::from_millis(80), Duration::from_millis(150)];
        let max_times = [Duration::from_millis(280), Duration::from_millis(350)];
        let results = vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]];
        let mut i = 0;

        let v = chunk_stream
            .map(move |s| {
                let now2 = Instant::now();
                println!("{}: {:?} {:?}", i, now2 - now, s);
                assert!((now2 - now) < max_times[i]);
                assert!((now2 - now) > min_times[i]);
                i += 1;
                s
            })
            .collect::<Vec<_>>();

        ThreadPool::new().unwrap().run(
            v.then(move |res| {
                assert_eq!(res, results);
                future::ready(())
            })
            .unit_error()
            .boxed()
        )
        .unwrap()
    }
}
