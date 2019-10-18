# tokio-batch

An adaptor that chunks up elements and flushes them after a timeout or when the buffer is full.

## Description

An adaptor that chunks up elements in a vector.

This adaptor will buffer up a list of items in the stream and pass on the
vector used for buffering when a specified capacity has been reached
or a predefined timeout was triggered.

## Usage

Either as a standalone Stream operator:
```rust
extern crate futures;

use std::time::Duration;

use futures::executor::ThreadPool;
use futures::future;
use futures::stream;
use futures::{FutureExt, StreamExt};
use futures_stream_batch::*;

fn main() {
    let iter = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9].into_iter();
    let stream = stream::iter(iter);

    let chunk_stream = ChunksTimeout::new(stream, 5, Duration::new(10, 0));

    let v = chunk_stream.collect::<Vec<_>>();
    ThreadPool::new().unwrap().run(
        v.then(|res| {
            assert_eq!(vec![vec![0, 1, 2, 3, 4], vec![5, 6, 7, 8, 9]], res);
            future::ready(())
        })
    );
}
```

Or as a Stream combinator using `futures-preview`:
```rust
extern crate futures;

use std::time::Duration;

use futures::executor::ThreadPool;
use futures::future;
use futures::stream;
use futures::{FutureExt, StreamExt};
use futures_stream_batch::ChunksTimeoutStreamExt;

fn main() {
    let iter = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9].into_iter();
    let v = stream::iter(iter)
        .chunks_timeout(5, Duration::new(10, 0))
        .collect::<Vec<_>>();

    ThreadPool::new().unwrap().run(
        v.then(|res| {
            assert_eq!(vec![vec![0, 1, 2, 3, 4], vec![5, 6, 7, 8, 9]], res);
            future::ready(())
        })
    );
}
```


## Credits

This was taken and adjusted from
https://github.com/rust-lang-nursery/futures-rs/blob/4613193023dd4071bbd32b666e3b85efede3a725/futures-util/src/stream/chunks.rs
and moved into a separate crate for usability.

Thanks to [@arielb1](https://github.com/arielb1) and [@alexcrichton](https://github.com/alexcrichton/) for their support!
