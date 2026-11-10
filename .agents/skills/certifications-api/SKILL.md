---
name: certifications-api
description: Write client-side semantic certifications with sun_observations::certifications. Use when building facts, expectations, planned production execution, prepared sources, comparisons, statistics, anomaly checks, lag checks, or final assertions.
---

# Certifications client language

Use `sun_observations::certifications::*`. Read certifications-methodology for proof design, domain-api for generated inputs, and observations-api for capture/query/decode.

## DSL

Authoring structure, not literal Rust. PROJECT/LEFT/RIGHT are value projections; KEY and LEFT_KEY/RIGHT_KEY are group/join key projections; PREDICATE is a predicate. EXECUTOR is |&input| -> execution and SEEDED_EXECUTOR is |&input, u64| -> execution. NAME/SOURCE/TEXT are diagnostic strings; VALUE/OBSERVED are typed values, VALUES/INPUTS typed collections, RESULT a Rust Result, OTHER another PreparedSource. EXPRESSION is eager Rust; CASE_EXPRESSION/AGGREGATE_EXPRESSION and CASE_EXPECTATION/AGGREGATE_EXPECTATION are Rust expressions using their symbolic root; EAGER_CHECK_RESULT is an already evaluated check. N/MIN_COUNT/TOP are counts; MIN_MEAN/MAX_MEAN, THRESHOLD, ABSOLUTE_TOLERANCE, DENOMINATOR_FLOOR, ABS and REL are f64; ABSOLUTE_THRESHOLD/RELATIVE_THRESHOLD are Option<f64>. LAG_START/LAG_END/OFFSET are i64, SEQUENCE_START/SEQUENCE_END are u64, Q is an f64 quantile in [0,1], BOOL is bool, MIN/MAX/BOUND use the observed ordered type, CLASS is ViolationClass::Hard or ::Statistical. NUMERIC_RULE, GROUP_EXPECTATION, CORRELATION_EXPECTATION, LINEAR_FIT_EXPECTATION, DIFFERENCE_EXPECTATION and RELATION_EXPECTATION are concrete types defined below; DOMAIN belongs to domain-api.

~~~text
CERTIFICATION :=
    c = certification(NAME)
    [ SOURCE_BINDING ]*
    [ EAGER_CHECK | SECTION ]*
    ( ASSERT_CERTIFY
    | runner = c.executor(EXECUTOR); PLANNED_BODY
    | runner = c.executor_with_seed(SEEDED_EXECUTOR); PLANNED_BODY )
SOURCE_BINDING :=
    value = source(NAME, &VALUE) | rows = collection(NAME, &VALUES)
EAGER_CHECK :=
    c.check_fact_expect(NAME, OBSERVED, EXPECTATION)
    | c.check(NAME, FACT.expect(EXPECTATION))
    | c.check(NAME, PREPARED_SOURCE_CHECK | NUMERIC_CHECK | GROUP_CHECK
    | CORRESPONDENCE_CHECK | NUMERIC_COMPARISON_CHECK)
SECTION :=
    c.section(NAME, |s| { [ EAGER_CHECK | SECTION ]* })
    | runner.section(NAME, |s| { [ PLANNED_CHECK | AGGREGATE_CHECK | SECTION ]* })
PLANNED_BODY :=
    [ runner.data_source(INPUTS) ] [ runner.domain(DOMAIN) ]
    [ runner.cases_budget(N) ] [ runner.failure_budget(N) ]
    [ runner.seed(N) ] [ runner.report_mode(BOOL) ]
    [ runner.require_pairwise_coverage() ]
    [ CASE ] [ CASE_AGGREGATE ] [ PLANNED_CHECK | AGGREGATE_CHECK | SECTION ]* ASSERT_CERTIFY
CASE := case = runner.case()
CASE_AGGREGATE := cases = runner.case_aggregate()
PLANNED_CHECK :=
    runner.check(fact!(CASE_EXPRESSION).expect(EXPECTATION | fact!(CASE_EXPECTATION))
      [ .description(TEXT) ])
    | runner.check(EAGER_CHECK_RESULT)
AGGREGATE_CHECK :=
    runner.check_aggregate(fact!(AGGREGATE_EXPRESSION)
        .expect(EXPECTATION | fact!(AGGREGATE_EXPECTATION)) [ .description(TEXT) ])
FACT := fact(SOURCE, VALUE) | try_fact(SOURCE, RESULT) | fact!(EXPRESSION)
EXPECTATION :=
    VALUE | NotEqual(VALUE)
    | Criterion::greater_than(BOUND)? | Criterion::at_least(BOUND)?
    | Criterion::less_than(BOUND)? | Criterion::at_most(BOUND)? | Criterion::between(MIN, MAX)?
    | criterion(TEXT, CLASS, PREDICATE)
    | close_to(VALUE)? [ .absolute_tolerance(ABS)? ] [ .relative_tolerance(REL)? ]
    | CountRule::NonEmpty | NumericRule::Finite | NumericRule::Positive | NumericRule::NonNegative
    | NumericRule::between(MIN, MAX)? | AtMostAnomalies(N) | LagExpectation { best_lag: OFFSET }
    | GroupExpectation::new(MIN_COUNT, MIN_MEAN, MAX_MEAN)?
    | CORRELATION_EXPECTATION | LINEAR_FIT_EXPECTATION | DIFFERENCE_EXPECTATION | RELATION_EXPECTATION
PREPARED_SOURCE_CHECK :=
    rows.count().expect(EXPECTATION) | value.single_map(PROJECT).expect(EXPECTATION)
    | rows.matching_count(PREDICATE).expect(EXPECTATION)
    | rows.unique(KEY).expect_valid() | rows.contiguous(KEY, SEQUENCE_START, SEQUENCE_END).expect_valid()
NUMERIC_CHECK :=
    rows.numeric(PROJECT).expect(NUMERIC_RULE)
    | rows.numeric(PROJECT).(summary() | robust_summary() | quantile(Q)).expect(EXPECTATION)
    | rows.numeric(PROJECT).outliers(OutlierConfig::default()
    | OutlierConfig::new(THRESHOLD, ZeroMadPolicy::ExactMedian
    | ZeroMadPolicy::absolute_tolerance(ABSOLUTE_TOLERANCE)?)?).expect(AtMostAnomalies(N))
    | rows.numeric(PROJECT).jumps(JumpConfig::new(ABSOLUTE_THRESHOLD, RELATIVE_THRESHOLD, DENOMINATOR_FLOOR)?)
        .expect(AtMostAnomalies(N))
| rows.(lag_self(PROJECT, LagConfig { range: LagRange::new(LAG_START, LAG_END)?,
        measure: LagMeasure::Pearson | LagMeasure::Spearman })
    | lag_with(&OTHER, LEFT, RIGHT, LagConfig { range: LagRange::new(LAG_START, LAG_END)?,
    measure: LagMeasure::Pearson | LagMeasure::Spearman }))
    .expect(LagExpectation { best_lag: OFFSET })
GROUP_CHECK :=
    fact(NAME, rows.group_by(KEY).numeric(PROJECT)
        .(summaries() | summaries_with_robust())).expect(GROUP_EXPECTATION)
CORRESPONDENCE_CHECK :=
    paired = left.compare_with(&right).on(LEFT_KEY, RIGHT_KEY)
    | paired.paired_comparison_fact().expect_valid()
    | paired.equal(LEFT, RIGHT, TOP).expect_valid()
    | paired.check_pairs(PREDICATE, TOP).expect_valid()
NUMERIC_COMPARISON_CHECK :=
    paired.numeric(LEFT, RIGHT).correlation().expect(CORRELATION_EXPECTATION)
    | paired.numeric(LEFT, RIGHT).linear_fit().expect(LINEAR_FIT_EXPECTATION)
    | paired.numeric(LEFT, RIGHT).difference().expect(DIFFERENCE_EXPECTATION)
    | paired.numeric(LEFT, RIGHT).relation().expect(RELATION_EXPECTATION)
ASSERT_CERTIFY := c.assert_certify() | runner.assert_certify()
~~~

## CERTIFICATION

Start one proof with certification(NAME). Register prerequisites and prepare borrowed evidence before optionally consuming c with executor(EXECUTOR) or executor_with_seed(SEEDED_EXECUTOR). Do not create two independent proofs for those phases.

Diagnostic controls are c.limits(DiagnosticLimits { failures, bytes_per_failure }) and c.report(). CertificationReport exposes name(), checks(), failed(), failures(), is_pass(), and assert_certify().

~~~rust
use sun_observations::certifications::*;
#[test]
fn export_contract() {
    let exported = [10u64, 11, 12];
    let mut c = certification("export has stable identity");
    c.check("ids", collection("export", &exported).unique(|id| *id).expect_valid());
    c.assert_certify();
}
~~~

## SOURCE_BINDING

Use source(name, &value) for one value and collection(name, &slice) for many. Names are diagnostics, not join keys. Bindings borrow evidence, not the certification; they survive the transition to planned execution.

Inspect with name(), values(), entry(index), iter(); attach custom provenance with with_provenance(&provider). A custom SourceProvenance provider implements the formatter contract below; write bounded diagnostic coordinates, propagate std::fmt::Result, and do not return an owned description. Cache projections with project(project) or keys(key); PreparedValues exposes values()/source(), and PreparedKeys exposes indices()/compare_with(&other_keys).

~~~rust
use sun_observations::certifications::*;
struct RowCoordinates;
impl SourceProvenance for RowCoordinates {
    fn describe(&self, index: usize, out: &mut dyn std::fmt::Write) -> std::fmt::Result {
        write!(out, "row={index}")
    }
}
#[test]
fn prepared_before_execution() {
    let expected = ["alpha", "beta"];
    let c = certification("normalized catalog");
    let catalog = collection("catalog", &expected).with_provenance(&RowCoordinates);
    let mut runner = c.executor(|s: &String| Ok::<_, ()>(s.trim().to_lowercase()));
    runner.data_source([" ALPHA ".to_owned()]);
    runner.check(catalog.count().expect(2usize));
    let case = runner.case();
    runner.check(fact!(case.result().clone()).expect("alpha".to_owned()));
    runner.assert_certify();
}
~~~

## EAGER_CHECK

Use c.check_fact_expect(name, observed, expectation) for a direct observed/expected check. Use c.check(name, fact.expect(expectation)) when the Fact needs its own source/details/description, and c.check(name, prepared_check) for prepared analysis. CheckResult::description(text) overrides the diagnostic name before registration. These are authoring roles, not competing acceptance engines.

~~~rust
use sun_observations::certifications::*;
#[test]
fn bounded_page() {
 let page = [21, 22, 23];
 let mut c = certification("page capacity");
 c.check_fact_expect("fits", page.len() <= 100, true);
 c.check("not empty", fact!(page.len()).expect(CountRule::NonEmpty));
 c.assert_certify();
}
~~~

## SECTION

Call section(name, |s| { ... }) on either phase; nest sections for meaningful semantic stages. Use the closure's s for registrations. Planned paths are captured at registration, not fact construction. An eager result registered with s.check(result) belongs to that planned section too. Paths remain separate from descriptions in reports; a section does not execute the planned cases.

~~~rust
use sun_observations::certifications::*;
#[test]
fn nested_stages() {
    let mut runner = certification("squares stay nonnegative")
        .executor(|n: &i32| Ok::<_, ()>(i64::from(*n).pow(2)));
    runner.data_source([-3, 0, 4]);
    let case = runner.case();
    runner.section("transform", |s| {
        s.section("range", |s| {
            s.check(fact!(*case.result()).expect(Criterion::at_least(0i64).unwrap()));
        });
    });
    runner.assert_certify();
}
~~~

## PLANNED_BODY

The executor runs real production: |input| -> Result<Output, Error>, or |input, seed| for seeded execution. CapturedRun<Result<Output, Error>> also works; capture belongs to observations-api.

- data_source(inputs): meaningful concrete cases, executed first; no DomainInput required.
- domain(domain): one generated input contract; syntax belongs to domain-api.
- cases_budget(n): total exploration capacity including real-case prefill; real inputs run first even beyond this bound. A registered Domain needs remaining generated capacity.
- failure_budget(n): positive bound on retained representative case failures; stops exploration when full.
- seed(n): reproducible framework/Domain seed; seeded executors additionally receive a deterministic per-case seed.
- report_mode(bool): coverage and registered-check summary.
- require_pairwise_coverage(): require enum values/pairs feasible under Domain relations and active branches within the existing budget, not extra generation capacity. Only positive cases credit positive coverage.
- diagnostic_bytes(bytes): per-failure diagnostic byte budget.

Positive executor errors, accepted generated-negative cases, and executor panics fail. Expected negative rejection passes and skips positive-only checks. Missing inputs, invalid Domain/configuration, incomplete capture, or undefined required analysis fail closed.

~~~rust
use sun_observations::certifications::*;
#[test]
fn magnitude_cases() {
    let mut runner = certification("absolute magnitude")
        .executor(|n: &i32| Ok::<_, ()>(i64::from(*n).abs()));
    runner.data_source([i32::MIN, 0, i32::MAX]);
    runner.seed(71).failure_budget(3).report_mode(true);
    let case = runner.case();
    runner.check(fact!(*case.result() >= 0).expect(true));
    runner.assert_certify();
}
~~~

## CASE

Bind let case = runner.case() once. Inside fact!(...), use case.input(), case.result(), case.observations::<T>() for captured payloads, or case.spans::<T>() for paired profiling evidence. result() is the successful positive output, not a Result. The binding does not borrow the runner. Ordinary runner.check(...) deferred checks reevaluate for each successful positive case; aggregate checks use the separate cases root below.

~~~rust
use sun_observations::certifications::*;
#[test]
fn sort_preserves_length() {
    let mut runner = certification("sorted batch cardinality").executor(|rows: &Vec<u64>| {
        let mut sorted = rows.clone();
        sorted.sort_unstable();
        Ok::<_, ()>(sorted)
    });
    runner.data_source([vec![3, 1, 2]]);
    let case = runner.case();
    runner.check(fact!(case.result().len()).expect(fact!(case.input().len())));
    runner.assert_certify();
}
~~~

## CASE_AGGREGATE

Bind let cases = runner.case_aggregate(). runner.check(...) still runs for each successful positive case; runner.check_aggregate(...) runs once after exploration. cases.cases()? exposes retained positive AggregateCase rows with id(), input(), result(), execution(); repeated equal inputs keep distinct CaseId values. Correctly rejected generated negatives are excluded. Local check failures keep otherwise valid evidence, while rejected positives, panics, or invalid prerequisites make aggregate access fail closed.

For captured executors, cases.observations::<T>()? and cases.spans::<T>()? return case-associated AggregateEvidence. Each row exposes case(); Observation rows add value()/observation(), span rows add measurement()/sources(). source(name) is an ordinary PreparedSource, so grouping, numeric, comparison, relation, and anomaly analysis stay generic. Any required case decode/span error fails the whole accessor rather than producing a partial sample; zero matching evidence is valid until a cardinality expectation says otherwise.

~~~rust
use sun_observations::certifications::*;
#[test]
fn cross_case_relation() {
let mut runner = certification("family relation").executor(|n: &usize| Ok::<_, ()>(n * 3 + 7));
runner.data_source([1, 1, 2, 4]);
let cases = runner.case_aggregate();
runner.check_aggregate(fact!({
let rows = collection("executions", cases.cases().unwrap());
let paired = rows.compare_with(&rows).on(|r| r.id(), |r| r.id());
paired.numeric(|r| *r.input() as f64, |r| *r.result() as f64)
.linear_fit().observed.unwrap()
}).expect(LinearFitExpectation::slope_between(2.99, 3.01).unwrap()));
runner.assert_certify();
}
~~~

## AGGREGATE_CHECK

check_aggregate(fact!(aggregate_expression).expect(expectation)) is deferred until all actually explored cases finish, including an existing failure-budget stop. Ordinary section(name, |s| ...) groups both check kinds; aggregate panics or unavailable required evidence become EvaluationError and earlier failures remain failures. Use the canonical case/cases roots and do not mix them in one fact. Aggregate expectations may themselves be fact!(aggregate_expression); .description(text) names the diagnostic. In deferred expressions, unwrap() is safe for the proof boundary because evaluation panics become EvaluationError; use the Result itself when its value is the intended evidence.

~~~rust
use sun_observations::certifications::*;
#[test]
fn aggregate_count() {
let mut runner = certification("family").executor(|n: &usize| Ok::<_, ()>(*n));
runner.data_source([1, 2]);
let cases = runner.case_aggregate();
runner.check_aggregate(fact!(cases.cases().unwrap().len()).expect(2usize)
.description("complete family"));
runner.assert_certify();
}
~~~

## PLANNED_CHECK

runner.check(fact!(expression).expect(expectation)) accepts static expectations or fact!(case-dependent expectation). .description(text) gives semantic diagnostic meaning. runner.check(eager_result) registers a one-time prerequisite, not a per-case check. Independent checks continue after check evaluation errors/panics.

~~~rust
use sun_observations::certifications::*;
#[test]
fn rounded_capacity() {
    let mut runner = certification("capacity rounds upward to a multiple of eight")
        .executor(|n: &usize| Ok::<_, ()>(n.div_ceil(8) * 8));
    runner.data_source([0, 1, 8, 9]);
    let case = runner.case();
    runner.check(fact!(*case.result() % 8).expect(0).description("aligned"));
    runner.check(fact!(*case.result() >= *case.input()).expect(true));
    runner.check(fact!(*case.result() - *case.input() < 8).expect(true));
    runner.assert_certify();
}
~~~

## FACT

Use fact!(expression) for ordinary expressions. Use fact(source_name, value) for a prepared report/value with an explicit diagnostic source, and try_fact(source_name, result) for fallible evidence so analysis errors remain EvaluationError. Add .details(text) for context. Apply .expect(expectation) before registering; facts never assert.

CheckResult inspection is source(), expected(), observed(), violations(), class(), analysis_error(), details(), is_pass(), plus description(text). CheckClass is Pass, HardViolation, StatisticalViolation, or EvaluationError.

~~~rust
use sun_observations::certifications::*;
#[test]
fn parse_persisted_count() {
    let mut c = certification("persisted count is usable");
    c.check("count", try_fact("header count", "24".parse::<u64>())
        .details("after reload").expect(24));
    c.assert_certify();
}
~~~

## EXPECTATION

Choose by evidence semantics:

- VALUE: exact PartialEq equality; NotEqual(value): inequality.
- Criterion::greater_than(bound), Criterion::at_least(bound), Criterion::less_than(bound), Criterion::at_most(bound), Criterion::between(min, max): ordered scalar bounds; constructors validate and return Result.
- criterion(text, ViolationClass::Hard | ViolationClass::Statistical, predicate): custom typed predicate. Implement Expectation<T> only for reusable policy: type Violation must implement std::fmt::Debug; T has no additional trait bounds. Required assess(&self, observed: &T) returns Result<Vec<Self::Violation>, AnalysisError> (empty means pass); describe(&self) returns String. Optional class(&self) -> ViolationClass defaults to Hard. Return AnalysisError when evaluation is impossible.
- close_to(expected)?.absolute_tolerance(abs)?.relative_tolerance(rel)?: validated f64 comparison.
- CountRule::{Exact(n), Minimum(n), NonEmpty}: structural counts.
- .expect_valid(): integrity, correspondence, and pair-predicate reports only, not arbitrary object validation.
- NumericRule::{Finite, Range(NumericRange), Positive, NonNegative}; prefer NumericRule::between(min,max)? to construct Range. NumericRange::new(min,max)? is the validated explicit range.
- AtMostAnomalies(n): outlier or jump count. LagExpectation { best_lag: offset }: keyed lag acceptance.
- GroupExpectation::new(min_count, min_mean, max_mean)?: each group's count and mean.
Use family-specific constructors and and_* combinators when they express the contract; use with_bound(measure, min, max)? and .bound(measure, min, max)? for explicit measures not covered by that sugar. Every constructor/combinator below returns Result. Scalar min/max are f64; generic bounds and correlation and_*_bounds arguments are Option<f64> (None means unbounded; at least one bound is required).

| Expectation parameter | Constructor; chainable combinators |
| --- | --- |
| CORRELATION_EXPECTATION | CorrelationExpectation::pearson_at_least(min), ::spearman_at_least(min), ::spearman_at_most(max); .and_pearson_bounds(min,max), .and_spearman_bounds(min,max) |
| LINEAR_FIT_EXPECTATION | LinearFitExpectation::slope_between(min,max), ::intercept_between(min,max), ::r_squared_at_least(min), ::residual_mean_absolute_at_most(max), ::residual_max_absolute_at_most(max); .and_slope_between(min,max), .and_intercept_between(min,max), .and_r_squared_at_least(min), .and_residual_mean_absolute_at_most(max), .and_residual_max_absolute_at_most(max) |
| DIFFERENCE_EXPECTATION | DifferenceExpectation::mean_absolute_at_most(max), ::p95_absolute_at_most(max), ::max_absolute_at_most(max); .and_mean_absolute_at_most(max), .and_p95_absolute_at_most(max), .and_max_absolute_at_most(max) |
| RELATION_EXPECTATION | RelationExpectation::spearman_at_least(min), ::spearman_at_most(max); .bound(measure,min,max) |

All four families provide CorrelationExpectation::with_bound(measure, min, max)?, LinearFitExpectation::with_bound(measure, min, max)?, DifferenceExpectation::with_bound(measure, min, max)?, or RelationExpectation::with_bound(measure, min, max)?, followed by .bound(measure, min, max)?. Use the measure enum belonging to that family:

- CorrelationMeasure::{Pearson, Spearman, AbsolutePearson, AbsoluteSpearman}.
- LinearFitMeasure::{Slope, Intercept, RSquared, ResidualMeanAbsolute, ResidualMaxAbsolute}.
- DifferenceMeasure::{MeanAbsolute, P95Absolute, MaxAbsolute}.
- RelationMeasure::{Pearson, Spearman, AbsolutePearson, AbsoluteSpearman, Slope, Intercept, RSquared, MeanAbsoluteDifference, P95AbsoluteDifference, MaxAbsoluteDifference, ResidualMeanAbsolute, ResidualMaxAbsolute}.

Bounds must be finite, ordered, and within the measure domain: signed correlations [-1,1], absolute correlations/R-squared [0,1], absolute differences/residuals nonnegative. MetricConstraint::new(measure, min, max)? is the common validated representation; inspect measure(), min(), max(). A required undefined metric is EvaluationError. For lag, no defined candidate or interior-incomplete best alignment is EvaluationError; a complete defined alignment at the wrong best lag is StatisticalViolation.

~~~rust
use sun_observations::certifications::*;
#[test]
fn tolerance_and_identity() {
    let mut c = certification("normalized reading");
    c.check("unit magnitude", fact("gain", 1.0f64)
        .expect(close_to(1.0).unwrap().absolute_tolerance(1e-6).unwrap()));
    c.check("assigned id", fact("id", 42u64).expect(NotEqual(0)));
    c.assert_certify();
}
~~~

~~~rust
use sun_observations::certifications::*;
struct Nonzero;
impl Expectation<u32> for Nonzero {
    type Violation = &'static str;
    fn assess(&self, observed: &u32) -> Result<Vec<Self::Violation>, AnalysisError> {
        Ok(if *observed == 0 { vec!["zero"] } else { vec![] })
    }
    fn describe(&self) -> String { "nonzero".to_owned() }
}
#[test]
fn reusable_expectation() {
    assert!(fact("count", 1u32).expect(Nonzero).is_pass());
    assert!(!fact("count", 0u32).expect(Nonzero).is_pass());
}
~~~

## PREPARED_SOURCE_CHECK

Use count().expect(count_rule), matching_count(predicate).expect(count_rule), single_map(project).expect(expectation), unique(key).expect_valid(), contiguous(key, start, end).expect_valid(). single_map requires exactly one value. contiguous uses inclusive u64 endpoints. unique/contiguous end with expect_valid(); names never substitute for typed keys. Lower-level direct helpers are check_count(actual, rule), check_unique(values, key), check_same_keys(left, right, left_key, right_key), and check_contiguous(values, key, start, end); their IntegrityReport supports is_valid().

~~~rust
use sun_observations::certifications::*;
#[test]
fn exported_sequence() {
    let ids = [42u64, 43, 44];
    let mut c = certification("complete export range");
    let rows = collection("export", &ids);
    c.check("unique", rows.unique(|id| *id).expect_valid());
    c.check("range", rows.contiguous(|id| *id, 42, 44).expect_valid());
    c.assert_certify();
}
~~~

## NUMERIC_CHECK
Profiling bridge: obtain SpanMeasurements from observations-api and use spans.source(name) as an ordinary PreparedSource. Project duration_ns() as f64 for generic numeric analysis or duration() for exact typed bounds; then reuse group_by, compare_with, relation, outliers, and jumps. summary() is optional; preserve execution identity when generated inputs repeat.

Use numeric(project) once when reused. PreparedNumeric exposes values(), series(), summary(), robust_summary(), quantile(q), outliers(config), jumps(config), and expect(numeric_rule). NumericSeries::new(iter) constructs a reusable series; it exposes iter(), finite_count(), integrity(rule), and the same fallible statistical/anomaly analyses. q is in [0,1]; for custom summary acceptance use criterion over the report.

- OutlierConfig::default() uses threshold 3.5 and ZeroMadPolicy::ExactMedian. OutlierConfig::new(threshold, zero_mad)? accepts a finite nonnegative threshold and ZeroMadPolicy::ExactMedian or the AbsoluteTolerance variant constructed by ZeroMadPolicy::absolute_tolerance(tolerance)?; inspect threshold() and zero_mad(). The underlying validated wrapper is NonNegativeFinite::new(value)? with get().
- JumpConfig::new(absolute_threshold, relative_threshold, denominator_floor)?: thresholds are Option<f64> (None disables that threshold), at least one must be Some. Enabled thresholds are finite/nonnegative; floor is finite/positive. Inspect absolute_threshold(), relative_threshold(), denominator_floor().
- lag_self(project, LagConfig { range: LagRange::new(start,end)?, measure }) or lag_with(&other,left,right,config): projections return (i64 key, f64 value); measure is LagMeasure::Pearson or ::Spearman. start/end are inclusive i64 offsets with start <= end; inspect range.start(), range.end(), zero_lag(), and alignment.has_interior_gaps(). The lower-level equivalent is lag_relation_by(left,right,left_project,right_project,config).

~~~rust
use sun_observations::certifications::*;
#[test]
fn stable_latency() {
    let milliseconds = [10.0, 10.0, 10.0, 10.0];
    let mut c = certification("latency integrity");
    let numeric = collection("latency ms", &milliseconds).numeric(|v| *v);
    c.check("finite", numeric.expect(NumericRule::Finite));
    c.check("outliers", numeric.outliers(OutlierConfig::default()).expect(AtMostAnomalies(0)));
    c.check("tolerant outliers", numeric.outliers(OutlierConfig::new(3.5,
        ZeroMadPolicy::absolute_tolerance(0.01).unwrap()).unwrap()).expect(AtMostAnomalies(0)));
    c.check("jumps", numeric.jumps(JumpConfig::new(Some(1.0), None, 0.01).unwrap())
        .expect(AtMostAnomalies(0)));
    c.assert_certify();
}
~~~

~~~rust
use sun_observations::certifications::*;
#[test]
fn keyed_alignment() {
    let samples = [(0i64, 1.0), (1, 2.0), (2, 1.0), (3, 4.0)];
    let rows = collection("signal", &samples);
    let mut c = certification("same signal aligns at zero");
    let start: i64 = -1; let end: i64 = 1; let offset: i64 = 0;
    c.check("lag", rows.lag_self(|r| *r, LagConfig {
        range: LagRange::new(start, end).unwrap(), measure: LagMeasure::Pearson,
    }).expect(LagExpectation { best_lag: offset }));
    c.assert_certify();
}
~~~

## GROUP_CHECK

Group by semantic KEY, prepare numeric PROJECT, then wrap summaries() or summaries_with_robust() with fact(name, report) before GroupExpectation. Certify required group identities separately; summaries cannot detect an entirely absent group. For custom checks use groups.iter(), then group.len(), group.is_empty(), group.values(), group.iter(), group.numeric(project). The last returns a NumericSeries: wrap its fallible analyses with try_fact(name, result) before .expect(expectation).

~~~rust
use sun_observations::certifications::*;
#[test]
fn per_region_latency() {
    let samples = [("east", 10.0), ("east", 12.0), ("west", 9.0), ("west", 11.0)];
    let mut c = certification("regional latency");
    let rows = collection("requests", &samples);
    let groups = rows.group_by(|r| r.0);
    c.check("regions", fact("region count", groups.iter().count()).expect(2usize));
    c.check("latency", fact("regional ms", groups.numeric(|r| r.1).summaries())
        .expect(GroupExpectation::new(2, 8.0, 13.0).unwrap()));
    c.assert_certify();
}
~~~

## CORRESPONDENCE_CHECK

Bind paired = left.compare_with(&right).on(left_key, right_key). Both keys must have the same ordered type. Always certify paired_comparison_fact().expect_valid() before interpreting matched subsets. equal(left,right,top) and check_pairs(predicate,top) fail closed on incomplete correspondence; top bounds examples (capped at 32).

Inspect comparison(), matched_pairs(), name() for diagnostics. Comparison exposes left_source(), right_source(), left_count(), right_count(), matched(), left_only(), right_only(), duplicate_left(), duplicate_right(), is_complete(). Join exposes set_source_names(left,right), comparison(), matched_pairs(), require_complete(). Lower-level join_by(left,right,left_key,right_key) and join_presorted_by(left,right,left_key,right_key) use the same correspondence semantics; ComparisonBuilder::on_presorted(left_key,right_key) validates pre-sorted inputs.

~~~rust
use sun_observations::certifications::*;
#[test]
fn persistence_preserves_rows() {
    let sent = [(1u64, "alpha"), (2, "beta")];
    let loaded = [(2u64, "beta"), (1, "alpha")];
    let mut c = certification("persistence by identity");
    let paired = collection("sent", &sent).compare_with(&collection("loaded", &loaded))
        .on(|r| r.0, |r| r.0);
    c.check("complete", paired.paired_comparison_fact().expect_valid());
    c.check("labels", paired.equal(|r| r.1, |r| r.1, 5).expect_valid());
    c.assert_certify();
}
~~~

## NUMERIC_COMPARISON_CHECK

Use paired.numeric(left_value,right_value) once. PreparedNumericComparison exposes values(); correlation(), linear_fit(), relation(), difference(), linear(), pearson(), spearman() are parameterless evidence methods, while largest(top) returns bounded diagnostic pairs. .linear() returns Fact<LinearMetrics>; .linear_fit() returns Fact<LinearFitMetrics>, whose slope(), intercept(), r_squared() accessors provide metric Facts; Metric::value() returns Option<f64>. Match correlation() with CorrelationExpectation, linear_fit() with LinearFitExpectation, difference() with DifferenceExpectation, and relation() with RelationExpectation. difference is right minus left, not regression residual; high correlation alone does not prove agreement.

Lower-level numeric functions take pair iterators: linear_metrics(pairs), pearson(pairs), spearman(pairs), correlation(pairs), linear_fit(pairs), difference_summary(pairs), numeric_relation(pairs).

~~~rust
use sun_observations::certifications::*;
#[test]
fn calibrated_sensor() {
    let reference = [(1u64, 1.0), (2, 2.0), (3, 3.0)];
    let measured = [(3u64, 3.001), (1, 1.001), (2, 2.001)];
    let mut c = certification("sensor calibration");
    let paired = collection("reference", &reference).compare_with(&collection("sensor", &measured))
        .on(|r| r.0, |r| r.0);
    c.check("identity", paired.paired_comparison_fact().expect_valid());
    let numeric = paired.numeric(|r| r.1, |r| r.1);
    let slope = numeric.linear_fit().slope();
    assert!(slope.observed.is_ok());
    let start: u64 = 1; let end: u64 = 3;
    c.check("sequence", collection("reference", &reference).contiguous(|r| r.0, start, end).expect_valid());
    c.check("agreement", numeric.difference().expect(DifferenceExpectation::max_absolute_at_most(0.01).unwrap()));
    c.check("association", numeric.correlation().expect(CorrelationExpectation::pearson_at_least(0.99).unwrap()));
    c.assert_certify();
}
~~~

## ASSERT_CERTIFY

End normal proofs with c.assert_certify() or runner.assert_certify(). It aggregates hard/statistical violations and evaluation errors. No checks is an eager failure; no executed cases is a planned failure. c.report() yields CertificationReport; runner.report() yields PlannedCertificationReport with is_pass(), cases_executed(), and assert_certify(). Read-only planned report inspection includes name(), seed(), budget(), executed(), categories(), failure_budget(), failures(), aggregate_failures(), evaluation_error(), stopped(), coverage(), and checks(). Reports are for programmatic diagnostics, not a replacement for final assertion in normal tests.

~~~rust
use sun_observations::certifications::*;
#[test]
fn no_forbidden_ids() {
    let ids = [1u64, 2, 3];
    let mut c = certification("ids are assigned");
    c.check("unassigned", collection("ids", &ids).matching_count(|id| *id == 0).expect(0usize));
    c.assert_certify();
}
~~~
