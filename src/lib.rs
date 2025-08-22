use rayon::iter::{
    IndexedParallelIterator, ParallelIterator,
    plumbing::{Consumer, ProducerCallback, Reducer, UnindexedConsumer},
};
use std::marker::PhantomData;
use tracing::Span;

pub trait TracedParallelIterator: ParallelIterator + Sized {
    /// Wrap a parallel iterator with tracing span support
    fn in_span(self, span: Span) -> InSpan<Self>;
}

impl<P> TracedParallelIterator for P
where
    P: ParallelIterator,
{
    fn in_span(self, span: Span) -> InSpan<Self> {
        InSpan {
            inner: self,
            span,
            _marker: PhantomData,
        }
    }
}

pub trait TracedIndexedParallelIterator: IndexedParallelIterator + Sized {
    /// Wrap an indexed parallel iterator with tracing span support
    fn indexed_in_span(self, span: Span) -> InSpan<Self>;
}

impl<P> TracedIndexedParallelIterator for P
where
    P: IndexedParallelIterator,
{
    fn indexed_in_span(self, span: Span) -> InSpan<Self> {
        InSpan {
            inner: self,
            span,
            _marker: PhantomData,
        }
    }
}

#[derive(Clone)]
pub struct InSpan<P> {
    inner: P,
    span: Span,
    _marker: PhantomData<fn() -> P>,
}

impl<P> ParallelIterator for InSpan<P>
where
    P: ParallelIterator,
{
    type Item = P::Item;

    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: UnindexedConsumer<Self::Item>,
    {
        self.inner
            .drive_unindexed(InSpanConsumer::new(consumer, self.span.clone()))
    }
}

impl<P> IndexedParallelIterator for InSpan<P>
where
    P: IndexedParallelIterator,
{
    fn drive<C>(self, consumer: C) -> C::Result
    where
        C: Consumer<Self::Item>,
    {
        self.inner
            .drive(InSpanConsumer::new(consumer, self.span.clone()))
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn with_producer<CB>(self, callback: CB) -> CB::Output
    where
        CB: ProducerCallback<Self::Item>,
    {
        self.inner.with_producer(callback)
    }
}

struct InSpanConsumer<C> {
    inner: C,
    span: Span,
}

impl<C> InSpanConsumer<C> {
    fn new(inner: C, span: Span) -> Self {
        Self { inner, span }
    }
}

impl<T, C> Consumer<T> for InSpanConsumer<C>
where
    C: Consumer<T>,
{
    type Folder = InSpanFolder<C::Folder>;
    type Reducer = InSpanReducer<C::Reducer>;
    type Result = C::Result;

    fn split_at(self, index: usize) -> (Self, Self, Self::Reducer) {
        let (left, right, reducer) = self.inner.split_at(index);
        let span = self.span.clone();
        (
            InSpanConsumer::new(left, span.clone()),
            InSpanConsumer::new(right, span.clone()),
            InSpanReducer {
                inner: reducer,
                span,
            },
        )
    }

    fn into_folder(self) -> Self::Folder {
        InSpanFolder {
            inner: self.inner.into_folder(),
            span: self.span.clone(),
        }
    }

    fn full(&self) -> bool {
        self.inner.full()
    }
}

impl<T, C> UnindexedConsumer<T> for InSpanConsumer<C>
where
    C: UnindexedConsumer<T>,
{
    fn split_off_left(&self) -> Self {
        InSpanConsumer::new(self.inner.split_off_left(), self.span.clone())
    }

    fn to_reducer(&self) -> Self::Reducer {
        InSpanReducer {
            inner: self.inner.to_reducer(),
            span: self.span.clone(),
        }
    }
}

struct InSpanFolder<F> {
    inner: F,
    span: Span,
}

impl<T, F> rayon::iter::plumbing::Folder<T> for InSpanFolder<F>
where
    F: rayon::iter::plumbing::Folder<T>,
{
    type Result = F::Result;

    fn consume(self, item: T) -> Self {
        InSpanFolder {
            inner: self.span.in_scope(|| self.inner.consume(item)),
            span: self.span,
        }
    }

    fn complete(self) -> Self::Result {
        self.span.in_scope(|| self.inner.complete())
    }

    fn full(&self) -> bool {
        self.inner.full()
    }
}

struct InSpanReducer<R> {
    inner: R,
    span: Span,
}

impl<R, T> Reducer<T> for InSpanReducer<R>
where
    R: Reducer<T>,
{
    fn reduce(self, left: T, right: T) -> T {
        self.span.in_scope(|| self.inner.reduce(left, right))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rayon::iter::IntoParallelIterator;
    use std::sync::{Arc, Mutex};
    use tracing::{Level, span};
    use tracing_core::Event;
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::prelude::*; // This imports SubscriberExt which provides .with()

    // Test utility: collect events in a thread-safe way
    #[derive(Debug, Clone)]
    struct TestCollector {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl TestCollector {
        fn new() -> Self {
            Self {
                events: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn get_events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }

        fn get_events_string(&self) -> String {
            self.get_events().join("\n")
        }

        fn clear(&self) {
            self.events.lock().unwrap().clear();
        }
    }

    impl<S> Layer<S> for TestCollector
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = EventVisitor::new();
            event.record(&mut visitor);
            let message = format!("{} {}", event.metadata().level(), visitor.message);
            self.events.lock().unwrap().push(message);
        }
    }

    struct EventVisitor {
        message: String,
    }

    impl EventVisitor {
        fn new() -> Self {
            Self {
                message: String::new(),
            }
        }
    }

    impl tracing::field::Visit for EventVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.message = format!("{:?}", value);
            }
        }
    }

    #[test]
    fn test_basic_tracing() {
        let collector = TestCollector::new();
        let subscriber = tracing_subscriber::registry().with(collector.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        let span = span!(Level::INFO, "test_span");

        let data = vec![1, 2];
        let result: Vec<i32> = data
            .into_par_iter()
            .in_span(span)
            .map(|x| {
                tracing::info!("processing {}", x);
                x * 2
            })
            .collect();

        let expected: Vec<i32> = vec![2, 4];
        assert_eq!(result, expected);

        // Give some time for async logging to complete
        std::thread::sleep(std::time::Duration::from_millis(50));

        let events = collector.get_events_string();
        println!("Events: '{}'", events);
    }

    #[test]
    fn test_empty_iterator() {
        let collector = TestCollector::new();
        let subscriber = tracing_subscriber::registry().with(collector.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        let span = span!(Level::INFO, "empty_test");

        let data: Vec<i32> = vec![]; // Fixed the syntax error
        let result: Vec<i32> = data
            .into_par_iter()
            .in_span(span)
            .map(|x| {
                tracing::info!("should not see this");
                x * 2
            })
            .collect();

        assert_eq!(result, vec![]);

        // Give some time for async logging to complete
        std::thread::sleep(std::time::Duration::from_millis(10));

        let events = collector.get_events_string();
        assert!(!events.contains("should not see this"));
    }

    #[test]
    fn test_both_method_names_work() {
        let collector = TestCollector::new();
        let subscriber = tracing_subscriber::registry().with(collector.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        // Test traced_in_span (regular parallel iterator)
        let span1 = span!(Level::INFO, "regular_span");
        let data = vec![1];
        let result1: Vec<i32> = data
            .clone()
            .into_par_iter()
            .in_span(span1)
            .map(|x| {
                tracing::info!("regular: {}", x);
                x * 2
            })
            .collect();

        // Clear events and test traced_indexed_in_span
        std::thread::sleep(std::time::Duration::from_millis(10));
        let events1 = collector.get_events_string();
        collector.clear();

        let span2 = span!(Level::INFO, "indexed_span");
        let result2: Vec<(usize, i32)> = data
            .into_par_iter()
            .indexed_in_span(span2)
            .enumerate()
            .map(|(i, x)| {
                tracing::info!("indexed: {} -> {}", i, x);
                (i, x * 3)
            })
            .collect();

        std::thread::sleep(std::time::Duration::from_millis(10));
        let events2 = collector.get_events_string();

        assert_eq!(result1, vec![2]);
        assert_eq!(result2, vec![(0, 3)]);

        println!("Events1: '{}'", events1);
        println!("Events2: '{}'", events2);
    }

    #[test]
    fn test_simple_smoke_test() {
        // Simple smoke test to verify the basic functionality works
        let collector = TestCollector::new();
        let subscriber = tracing_subscriber::registry().with(collector.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        let span = span!(Level::INFO, "smoke_test");

        let data = vec![42];
        let result: Vec<i32> = data
            .into_par_iter()
            .in_span(span)
            .map(|x| {
                tracing::info!("smoke processing");
                x
            })
            .collect();

        assert_eq!(result, vec![42]);

        // Give some time for async logging to complete
        std::thread::sleep(std::time::Duration::from_millis(50));

        let events = collector.get_events();
        println!("Smoke test events count: {}", events.len());

        for event in &events {
            println!("Event: {}", event);
        }
    }

    #[test]
    fn test_reduction_with_tracing() {
        let collector = TestCollector::new();
        let subscriber = tracing_subscriber::registry().with(collector.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        let span = span!(Level::INFO, "reduction_span");

        let data = vec![1, 2, 3, 4];
        let sum = data.into_par_iter().in_span(span).reduce(
            || 0,
            |a, b| {
                tracing::info!("Combining {} and {}", a, b);
                a + b
            },
        );

        assert_eq!(sum, 10);

        std::thread::sleep(std::time::Duration::from_millis(50));

        let events = collector.get_events_string();
        assert!(events.contains("Combining"));
        println!("Reduction events: {}", events);
    }
}
