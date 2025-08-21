# rayon-tracing

**`rayon-tracing`** integrates [Rayon](https://github.com/rayon-rs/rayon)’s parallel iterators with [tracing](https://github.com/tokio-rs/tracing).
It ensures that spans are correctly entered in every Rayon worker thread, so log events and structured traces remain consistent across parallel computations.

---

## ✨ Features

* ✅ Works with both **`ParallelIterator`** and **`IndexedParallelIterator`**
* ✅ Provides `.in_span(span)` and `.indexed_in_span(span)` extension methods
* ✅ Preserves your `tracing::Span` across all Rayon worker tasks
* ✅ Minimal overhead (cheap `Span` clones under the hood)
* ✅ Tested against empty iterators and mixed usage

---

## 📦 Installation

```toml
[dependencies]
rayon = "1"
tracing = "0.1"
rayon-tracing = { git = "https://github.com/danylaporte/rayon-tracing" }
```

---

## 🚀 Usage

### Regular parallel iterator

```rust
use rayon::prelude::*;
use tracing::{info_span, info};
use rayon_tracing::TracedParallelIterator;

fn main() {
    tracing_subscriber::fmt::init();

    let span = info_span!("regular_iter");

    (1..=3u32)
        .into_par_iter()
        .in_span(span) // <--
        .for_each(|i| {
            info!(%i, "processing");
        });
}
```

### Indexed parallel iterator

```rust
use rayon::prelude::*;
use tracing::{info_span, info};
use rayon_tracing::TracedIndexedParallelIterator;

fn main() {
    tracing_subscriber::fmt::init();

    let span = info_span!("indexed_iter");

    (1..=3u32)
        .into_par_iter()
        .indexed_in_span(span) // <--
        .enumerate()
        .for_each(|(i, x)| {
            info!(%i, %x, "indexed processing");
        });
}
```

---

## 🔧 How It Works

* Adds two blanket extension traits:

  ```rust
  trait TracedParallelIterator {
      fn in_span(self, span: Span) -> InSpan<Self>;
  }

  trait TracedIndexedParallelIterator {
      fn indexed_in_span(self, span: Span) -> InSpan<Self>;
  }
  ```
* Wraps Rayon’s `Consumer` / `Folder` and re-enters the span around every item.
* Ensures that **all worker threads log inside the correct tracing span**.

---

## 📝 Caveats

* Only affects Rayon iterators; does not automatically propagate spans into `ThreadPool::spawn`.
* Must explicitly call `.in_span(...)` or `.indexed_in_span(...)`.
* Spans are cloned at each fork in the parallel iterator (cheap `Arc` clone).

---

## ✅ Tests

The crate includes unit tests to ensure:

* Events are captured correctly inside spans
* Empty iterators don’t produce spurious logs
* Both `.in_span` and `.indexed_in_span` behave as expected
* Smoke tests confirm tracing works end-to-end

---

## 📜 License

Licensed under either:

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))
* MIT license ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))

at your option.
