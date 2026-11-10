---
name: observations-api
description: Use the SunObservations client API for typed production evidence. Use when declaring or emitting Observations, configuring selection/scopes/policies, routing/sinks/spans, capture, query/decode, or provenance.
---

# Observation evidence language

Use sun_observations::* for runtime and sun_observations::evidence::* for capture/query/decode. Read certifications-methodology to decide which facts need observing; certifications-api owns acceptance and proof aggregation. Domain generation belongs to domain-api.

## DSL

Authoring structure, not literal Rust. Selection controls admission; routing controls delivery; capture observes the real operation without replacing its behavior. NAMESPACE/NS, TAG/TAGS, NAME and ID denote namespace strings, tag string(s), diagnostic/scope strings and persisted identity strings. PAYLOAD is a concrete Observation type; VALUE/START_PAYLOAD are implementation values of that type; FIELDS are its Rust fields. Declaration LEVEL is INFO/WARN/ERROR, query LEVEL is ObservationLevel::INFO/WARN/ERROR. POLICY is Policy::Always, Policy::Never, Policy::first_n(N)? or Policy::every_n(N)? (positive u64 N). OPERATION means arbitrary real production code or a closure executed under the surrounding scope/span/capture, not a framework type. CHANNEL_NAME is the logical routing-channel name/string accepted by routing APIs; PATH is a filesystem path.

~~~text
DECLARATION :=
    #[derive(Serialize, Deserialize, Observation)]
    #[observation(namespace = NAMESPACE, level = LEVEL, tags(TAG, ...))]
    struct PAYLOAD { FIELDS }
EMISSION := observe!(VALUE) | try_observe!(VALUE)
SELECTION := Selection::new()
    [ .activate_all() | .activate_namespace(NS)? | .activate_tags(TAGS)? ]*
    [ .exclude_namespace(NS)? | .exclude_tags(TAGS)? ]*
SCOPE := guard = start_scope(ScopeConfig::new(NAME)
    [ .selection(SELECTION) ]
    [ .activate_all() | .activate_namespace(NS)? | .activate_tags(TAGS)? | .exclude_namespace(NS)? | .exclude_tags(TAGS)? ]*
    [ .override_policy::<PAYLOAD>(POLICY) ])
    [ Context::capture().run(OPERATION) | context::thread::spawn(OPERATION) ]
ROUTING := engine.add_channel_sink(CHANNEL_NAME, SINK)
    engine.register_namespace_channel(NAMESPACE, CHANNEL_NAME)?
SINK := Arc::new(MemorySink::new() | MemorySink::with_config(MEMORY_CONFIG)
    | JsonlSink::open_append(PATH)? | JsonlSink::with_config(PATH, JSONL_CONFIG)?
    | ConsoleSink::stdout() | ConsoleSink::stderr() | ConsoleSink::new(WRITER) | CUSTOM_SINK_IMPL)
SPAN := span = observe_span!(START_PAYLOAD)
OPERATION
span.finish()?
CAPTURE := run = capture_observations(CaptureConfig::new()
    [ .selection(SELECTION) ]
    [ .activate_all() | .activate_namespace(NS)? | .activate_tags(TAGS)? | .exclude_namespace(NS)? | .exclude_tags(TAGS)? ]*
    [ .retention(MEMORY_CONFIG) ], OPERATION)
QUERY := dataset.query(ObservationsQuery::new()
    [ .namespace(NS) | .namespace_subtree(NS) ]
    [ .scope(NAME) | .without_scope() ] [ .level(LEVEL) ]
    [ .points() | .span_starts() | .span_ends() ] [ .span(ID) ] [ .process(ID) ])
TYPED_EVIDENCE := typed = (run | dataset | QUERY).to_typed::<PAYLOAD>()?; typed.source(NAME)
SPAN_EVIDENCE := spans = (run | dataset | QUERY).to_spans::<PAYLOAD>()?; spans.source(NAME)
~~~

## DECLARATION

Derive Observation on Serialize payloads; add Deserialize for typed decoding. namespace is required stable schema identity; level is INFO (default), WARN, ERROR; tags are optional selection labels. Manual Observation: serde::Serialize + 'static; require const NAMESPACE: &'static str. Defaults are const LEVEL: ObservationLevel = ObservationLevel::INFO and const TAGS: &'static [&'static str] = &[]. Optional message(&self) -> Option<&str> borrows persisted semantic text; render_text(&self) -> Option<String> owns transient human output; both default to None.

FORMAT v1 persists ts, proc, proc_ns, namespace, level, data plus optional scope, message, phase, span. Tags, channels, render_text are not persisted. Point records have no phase/span; span starts carry data/message and ends have null data without message. proc_ns is monotonic only within one process.

~~~rust
use sun_observations::*;
#[derive(serde::Serialize, serde::Deserialize, Observation)]
#[observation(namespace = "orders::accepted", level = INFO, tags("evidence"))]
struct Accepted { order_id: u64, quantity: u32 }
#[test]
fn derived_identity() {
    assert_eq!(Accepted::NAMESPACE, "orders::accepted");
}
~~~

~~~rust
use sun_observations::*;
#[derive(serde::Serialize)]
struct Accepted { order_id: u64, quantity: u32 }
impl Observation for Accepted {
    const NAMESPACE: &'static str = "orders::accepted";
    const LEVEL: ObservationLevel = ObservationLevel::INFO;
    const TAGS: &'static [&'static str] = &["evidence"];
    fn message(&self) -> Option<&str> { Some("accepted") }
    fn render_text(&self) -> Option<String> { Some(format!("order {}", self.order_id)) }
}
#[test]
fn stable_identity() {
    assert_eq!(Accepted::NAMESPACE, "orders::accepted");
}
~~~

## EMISSION

For ordinary production side effects, observe!(value) returns () and emits best-effort through the global engine, without coupling application control flow to emission failures. For caller-visible admission/error handling, try_observe!(value) returns Result<bool, ObservationError>: Ok(false) means rejected before construction/serialization; Ok(true) means admitted/dispatched, not durable persistence. ObservationError exposes invalid static namespace/tag metadata, serialization, or sink failures. Both macros use the same canonical runtime path and admit lazily: expensive payload construction stays inside the argument and is skipped on rejection. Discarding the result at the observe! caller boundary preserves capture emission_errors, incomplete capture status, and runtime sink_error_count() accounting.

Use global() to configure the process engine; engine.set_selection(selection) replaces global admission, while process_id(), sink_error_count(), flush_channels() inspect effects. For isolated runtime/routing proofs only, use ObservationEngine::isolated() and engine.observe_if_active(|| value), not dependency injection into ordinary production functions.

~~~rust
use sun_observations::*;
#[derive(serde::Serialize, Observation)]
#[observation(namespace = "orders::validated")]
struct Validated { order_id: u64 }
fn validate(order_id: u64) -> bool {
    if order_id == 0 { return false; }
    observe!(Validated { order_id });
    true
}
#[test]
fn production_validation() {
    assert!(validate(42));
    let _scope = start_scope(ScopeConfig::new("explicit admission").activate_all());
    assert!(try_observe!(Validated { order_id: 42 }).unwrap());
}
~~~

## SELECTION

Selection::new() starts disabled. Combine .activate_all(), .activate_namespace(ns)?, .activate_tags(tags)?; .exclude_namespace(ns)? and .exclude_tags(tags)? veto positives. Namespace matching respects segment boundaries; invalid dynamic namespaces/tags return errors. Inspect .activates(ns, tags), .excludes(ns, tags), .admits(ns, tags). Lower-level validation helpers are validate_tag(value), is_normalized_segment(value), validate_namespace(value), and matches_subtree(root, namespace).

~~~rust
use sun_observations::*;
#[test]
fn select_boundary_not_debug() {
    let selection = Selection::new().activate_namespace("orders").unwrap()
        .exclude_namespace("orders::debug").unwrap();
    assert!(selection.admits("orders::accepted", &[]));
    assert!(!selection.admits("orders::debug::row", &[]));
}
~~~

## SCOPE

Keep the start_scope(config) guard alive through the operation. Prefer ScopeConfig::new(name).activate_all()/activate_namespace(ns)?/activate_tags(tags)? and exclude_namespace(ns)?/exclude_tags(tags)? for scope-local selection; use .selection(selection) when reusing a prebuilt Selection. Add .override_policy::<T>(policy) as needed. Policy variants are Always, Never, FirstN(nonzero), EveryN(nonzero); prefer Policy::first_n(n)? and Policy::every_n(n)? for positive u64 counts. Counters belong to scope/type lifecycle; zero returns None from the helper constructors.

Capture context = Context::capture() in the parent. Use context.enter() when a guard must cover several calls in the current thread, context.run(operation) inside an existing worker, or context::thread::spawn(operation) for a new thread. For Rayon use ContextThreadPool::new()? for default sizing or ContextThreadPool::with_threads(nonzero_thread_count)? for an explicit size, then pool.install(operation). Ordinary threads/unrelated async tasks do not inherit context. Join workers before interpreting capture completeness.

~~~rust
use sun_observations::*;
#[test]
fn propagate_batch_identity() {
 let _guard = start_scope(ScopeConfig::new("batch-42").activate_all());
 let worker = context::thread::spawn(|| Context::capture().scope_name().map(str::to_owned));
 assert_eq!(worker.join().unwrap().as_deref(), Some("batch-42"));
}
~~~

## ROUTING

Attach Arc<dyn Sink> with add_channel_sink(channel,sink), then register_namespace_channel(namespace,channel). Routing does not activate anything. Matching routes deduplicate a sink within one channel; the same sink in two matching channels receives twice. clear_channel_sinks(channel) detaches; flush_channels() waits for delivery barriers.

~~~rust
use std::sync::Arc;
use sun_observations::*;
#[derive(serde::Serialize, Observation)]
#[observation(namespace = "routing::accepted")]
struct Accepted { id: u64 }
#[test]
fn route_to_audit() {
    let engine = ObservationEngine::isolated();
    let memory = Arc::new(MemorySink::new());
    engine.add_channel_sink("audit", memory.clone());
    engine.register_namespace_channel("routing", "audit").unwrap();
    engine.set_selection(Selection::new().activate_all());
    engine.observe_if_active(|| Accepted { id: 42 }).unwrap();
    engine.flush_channels().unwrap();
    assert_eq!(memory.len(), 1);
}
~~~

## SINK

MEMORY_CONFIG means MemorySinkConfig::new(MAX_ITEMS, COMPACT_FRACTION)?; MAX_ITEMS is positive usize and COMPACT_FRACTION is finite in (0,1]. JSONL_CONFIG means JsonlConfig::new() with optional flush_interval_ms(MS)?, max_file_size_bytes(BYTES)? or max_file_size_mb(MIB)?, and trim_fraction(FRACTION)?. WRITER is an owned `Write + Send + 'static` value. CUSTOM_SINK_IMPL is a concrete Sink implementation value, not an Arc<dyn Sink>; Arc::new(...) is supplied by the DSL. Sink: Send + Sync + 'static; only write(&self, record: &ObservationRecord) -> std::io::Result<()> and flush(&self) -> std::io::Result<()> are required. The example also shows all optional hooks: write_shared owns an Arc and defaults to write; wants_render_text defaults to false; write_delivery borrows the delivery and defaults to write_shared. Serialize concurrent writes; keep wants_render_text immutable while attached (sampled once at attachment). Async flush must be a real delivery barrier.

- MemorySink::new() uses 1,000,000 records and 0.10 compaction; MemorySink::with_config(MEMORY_CONFIG) bounds retention. MemorySinkConfig exposes max_items() and compact_fraction(). Inspect sink len(), is_empty(), config(), stats(), records()/shared_records(); drain()/drain_shared() remove retained records. Eviction makes capture incomplete.
- JsonlSink::open_append(path)? / from_writer(writer) use defaults; with_config(path, JSONL_CONFIG)? / writer_with_config(writer, JSONL_CONFIG)? customize persistence. JsonlConfig defaults to 1000 ms flush, no size bound, 0.10 trim. Inspect stats(), is_started(), last_error(); flush()? is the read barrier.
- ConsoleSink::stdout() / stderr() select human output; ConsoleSink::new(writer) uses an owned writer. Inspect ConsoleSink::is_started() and ConsoleSink::last_error(). ObservationDelivery exposes record(), shared_record(), render_text(), shared_render_text().

~~~rust
use sun_observations::*;
#[test]
fn bounded_memory_sink() {
let memory = MemorySink::with_config(MemorySinkConfig::new(1000, 0.1).unwrap());
assert_eq!(memory.config().max_items(), 1000);
let console = ConsoleSink::new(Vec::<u8>::new());
assert!(!console.is_started());
assert!(console.last_error().is_none());
}
~~~

~~~rust
use std::{io, sync::Arc};
use sun_observations::*;
struct AuditSink(MemorySink);
impl Sink for AuditSink {
    fn write(&self, record: &ObservationRecord) -> io::Result<()> { self.0.write(record) }
    fn flush(&self) -> io::Result<()> { self.0.flush() }
    fn write_shared(&self, record: Arc<ObservationRecord>) -> io::Result<()> {
        self.0.write_shared(record)
    }
    fn wants_render_text(&self) -> bool { true }
    fn write_delivery(&self, delivery: &ObservationDelivery) -> io::Result<()> {
        self.write_shared(delivery.shared_record().clone())
    }
}
#[test]
fn custom_sink_contract() {
    let sink = AuditSink(MemorySink::new());
    assert!(sink.wants_render_text());
    sink.flush().unwrap();
}
~~~

## SPAN

Use observe_span!(START_PAYLOAD), which lazily admits before payload construction. START_PAYLOAD describes the operation and attribution dimensions known at span start and is serialized on the start record. finish() records completion/end timing and carries no additional payload: the end has data: null and no message. An incomplete operation still leaves its attributable start so evidence can fail closed on the incomplete lifecycle. Inspect span.is_active(); call span.finish()? when end-delivery failure matters. Drop attempts end delivery. Start admission freezes scope/routing/sinks/policy through end; do not re-check current selection at finish. Match persisted spans by proc, scope, namespace, span, never adjacency. Lower-level isolated code may use span_if_active(|| payload) or engine.span_if_active(|| payload).

~~~rust
use sun_observations::*;
#[derive(serde::Serialize, Observation)]
#[observation(namespace = "batch::sort")]
struct Sort { rows: usize }
#[test]
fn measure_sort() {
    let mut rows = [3, 1, 2];
    let span = observe_span!(Sort { rows: rows.len() }).unwrap();
    rows.sort_unstable();
    span.finish().unwrap();
    assert_eq!(rows, [1, 2, 3]);
}
~~~

## CAPTURE

capture_observations(config, operation) wraps real production. CaptureConfig::new() starts disabled with default memory retention. Selection builders are activate_all(), activate_namespace(ns)? / activate_namespaces(iter)?, exclude_namespace(ns)? / exclude_namespaces(iter)?, activate_tag(tag)? / activate_tags(iter)?, exclude_tag(tag)? / exclude_tags(iter)?. Use selection(selection) when reusing a prebuilt Selection and retention(MEMORY_CONFIG) for storage bounds.
Read run.result(), dataset(), status().is_complete(), to_typed::<T>()?. Typed decoding alone does not assert capture health; use run.certification(name), the planned prerequisite, or an explicit completeness check before proof. CapturedRun completeness requires no eviction, emission error, or escaped context/worker/span. Capture selection replaces global admission/delivery while the enclosing scope identity, exclusions, policy and counters remain in force. Nested captures are exclusive; parallel captures have separate storage. run.certification(name) starts a proof with completeness already registered. A planned executor can return captured Result; case.observations::<T>() then accesses typed evidence and capture completeness is mandatory.

~~~rust
use sun_observations::*;
use sun_observations::evidence::*;
#[derive(serde::Serialize, serde::Deserialize, Observation)]
#[observation(namespace = "capture::normalized", tags("evidence"))]
struct Normalized { original_bytes: usize, output_bytes: usize }
fn normalize(raw: &str) -> String {
    let output = raw.trim().to_lowercase();
    observe!(Normalized { original_bytes: raw.len(), output_bytes: output.len() });
    output
}
#[test]
fn capture_normalization() {
 let run = capture_observations(CaptureConfig::new().activate_tags(["evidence"]).unwrap(),
 || normalize(" ALPHA "));
    assert_eq!(run.result(), "alpha");
    assert!(run.status().is_complete());
    assert_eq!(run.to_typed::<Normalized>().unwrap().values().len(), 1);
}
~~~

## QUERY

Build fixed evidence with ObservationDataset::try_from_records(Vec<Arc<ObservationRecord>>) or try_from_memory(&sink); records() borrows the snapshot. dataset.query(ObservationsQuery::new()...) filters without decoding. Independent selector dimensions combine with AND; later namespace/subtree, scope/without_scope, or point/span builders replace conflicting intent. Variants: namespace(ns) / namespace_subtree(ns), scope(name) / without_scope(), level(ObservationLevel::INFO), points() / span_starts() / span_ends(), span(id), process(id). Queries are unrestricted by default; iter() inspects selected records.

~~~rust
use sun_observations::evidence::*;
#[test]
fn select_one_execution() {
    let dataset = ObservationDataset::try_from_records(vec![]).unwrap();
    let view = dataset.query(ObservationsQuery::new()
        .namespace_subtree("orders").scope("batch-42").points());
    assert_eq!(view.iter().count(), 0);
}
~~~

## TYPED_EVIDENCE

Use run.to_typed::<Payload>()? for captured payloads, dataset.to_typed::<Payload>()? for a full persisted snapshot, or query.to_typed::<Payload>()? for selected persisted evidence. Read typed.values(), typed.sources(), typed.iter() (or typed.iter().nth(index)); TypedEntry exposes value, dataset_index, source record. typed.source(name) prepares generic certification evidence without re-decoding and preserves SourceProvenance. DecodeError carries SourceContext (dataset_index, namespace, proc, scope, span, phase, ts, proc_ns). Source-local and dataset indexes differ.

Filter by process/scope first when domain keys repeat across executions, or use typed execution identity in the comparison key. Acceptance and correspondence rules belong to certifications-api.

~~~rust
use sun_observations::*;
use sun_observations::evidence::*;
#[derive(serde::Serialize, serde::Deserialize, Observation)]
#[observation(namespace = "evidence::accepted")]
struct Accepted { id: u64 }
#[test]
fn retain_provenance() {
 let run = capture_observations(CaptureConfig::new().activate_all(),
 || observe!(Accepted { id: 42 }));
    let typed = run.to_typed::<Accepted>().unwrap();
    let prepared = typed.source("accepted orders");
    assert_eq!(prepared.values()[0].id, 42);
    assert_eq!(typed.iter().next().unwrap().dataset_index, 0);
}
~~~

## SPAN_EVIDENCE

Use run.to_spans::<Payload>()? for a complete capture, dataset.to_spans::<Payload>()? for a full persisted snapshot, or query.to_spans::<Payload>()? for selected persisted traces; then spans.source(name) exposes PreparedSource<SpanMeasurement<Payload>>. Evidence pairs by SpanIdentity = (proc, scope-or-absence, namespace, span), independent of record order. Exactly one start/end is required: missing, duplicate (including fan-out), reversed-time, mismatched-level, or undecodable lifecycles return SpanError; no partial sample is returned. Capture health is also mandatory for run.to_spans::<Payload>().

measurement.identity() exposes SpanIdentity; use identity.process(), identity.scope(), identity.namespace(), identity.span(). measurement.value() is the decoded start payload, measurement.duration_ns() is canonical elapsed nanoseconds, and measurement.duration() is the same value as Duration. spans.values() returns measurements; spans.sources() retains both original records and start_index/end_index; spans.source(name) reuses generic PreparedSource analysis with provenance. Empty evidence is not proof of execution. Nanoseconds preserve available clock precision, not hardware resolution or freedom from instrumentation noise; certifications-api owns analysis and acceptance.
~~~rust
use sun_observations::{certifications::*, evidence::*, *};
#[derive(serde::Serialize, serde::Deserialize, Observation)]
#[observation(namespace = "profile::sort")]
struct Sort { rows: usize }
#[test]
fn prepare_elapsed_evidence() {
 let run = capture_observations(CaptureConfig::new().activate_all(), || {
 let mut rows = [3, 1, 2];
 let span = observe_span!(Sort { rows: rows.len() }).unwrap();
 rows.sort_unstable();
 span.finish().unwrap();
 });
 let spans = run.to_spans::<Sort>().unwrap();
 let mut c = run.certification("sort profiling evidence");
 c.check("one completed sort", spans.source("sort spans").count().expect(1usize));
 c.check("input size", spans.source("sort spans").single_map(|m| m.value().rows).expect(3usize));
 c.assert_certify();
}
~~~
