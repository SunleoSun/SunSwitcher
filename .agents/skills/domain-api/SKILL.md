---
name: domain-api
description: Use the typed Domain DSL for generated certification inputs. Use when defining DomainInput types, ranges, choices, nested fields, collections, conditions, relations, or pairwise coverage inputs.
---

# Domain client language

Use sun_observations::certifications::*. Read certifications-methodology for input-proof strategy and certifications-api for execution, budgets, seeds, pairwise coverage, and assertions. Domain owns the input family, not acceptance policy or Observation capture.

## DSL

Authoring structure, not literal Rust; the actual macro is domain!(TYPE { ... }). TYPE is a concrete DomainInput type; SCALAR_TYPE is a built-in scalar DomainInput such as i32 or usize; FIELD/ENUM_FIELD/COLLECTION are typed field paths. VALUE is one compatible value; VALUES/LENGTHS/VARIANTS are explicit compatible collections. RANGE is a supported Rust range and RANGES is a collection of them. OPERAND is a compatible scalar constant or typed field operand.

~~~text
DOMAIN := domain!(TYPE { RULE, ... }) | domain!(SCALAR_TYPE { CONSTRUCTIVE })
RULE := FIELD.CONSTRUCTIVE | FIELD { RULE, ... }
      | ENUM_RULE | COLLECTION_RULE | WHEN | RELATION
CONSTRUCTIVE := fixed_value(VALUE) | selected_values(VALUES)
              | in_range(RANGE) | one_of_ranges(RANGES)
              | all_enum_values() | selected_enum_values(VALUES)
ENUM_RULE := ENUM_FIELD {
    all_enum_values() | selected_enum_values(VARIANTS),
    EnumType::Variant { RULE, ... }, ...
}
COLLECTION_RULE := COLLECTION {
    length().(fixed_value(N) | selected_values(LENGTHS) | in_range(RANGE) | one_of_ranges(RANGES)),
    each(CONSTRUCTIVE) | each { RULE, ... }
}
WHEN := when(RELATION) { RULE, ... }
RELATION := FIELD.(equal(OPERAND) | not_equal(OPERAND)
                 | less_than(OPERAND) | less_equal(OPERAND)
                 | greater_than(OPERAND) | greater_equal(OPERAND))
~~~

## DOMAIN

Scalar roots use the constructive rule directly, for example `domain!(i32 { in_range(1..=3) })` or `domain!(usize { in_range(1..=9) })`; there is no field receiver. Derive DomainInput on concrete named-field structs and enums; derive Clone when production fixtures need it. Built-ins include integers, f32/f64, bool, String, Vec<T>. No generic input derives, unions, or unnamed struct fields. Register the Domain with a planned runner; real data_source cases can prefill generated coverage.

Generation chooses deterministic representatives and meaningful constraint-breaking negative inputs. Declare only states at the validation owner; downstream owners should receive reachable valid inputs. No shrinking, regex/arbitrary-string generation, map/set, uniqueness, or collection-order rule language exists. For family-wide profiling use the same runner.domain(domain) and real captured executor; observations-api owns paired measurements and certifications-api owns analysis/acceptance.

~~~rust
use sun_observations::certifications::*;
#[derive(Clone, DomainInput)]
struct Reading { raw: i32 }
#[test]
fn reading_family() {
    let _ = domain!(i32 { in_range(1..=3) });
    let _ = domain!(usize { in_range(1..=9) });
    let mut runner = certification("magnitude is nonnegative")
        .executor(|r: &Reading| Ok::<_, ()>(i64::from(r.raw).abs()));
    runner.domain(domain!(Reading { raw.in_range(i32::MIN..=i32::MAX) }));
    let case = runner.case();
    runner.check(fact!(*case.result() >= 0).expect(true));
    runner.assert_certify();
}
~~~

## RULE

FIELD.CONSTRUCTIVE selects a field's value space. FIELD { RULE, ... } descends into a nested struct, enum, or collection. DomainField::fields() is the macro backing for that nested descent and normally is not handwritten. Every active scalar/length field needs exactly one constructive source. Nested blocks, enum payloads, each blocks, and when bodies contain ordinary rules; they are not opaque predicates.

~~~rust
use sun_observations::certifications::*;
#[derive(Clone, DomainInput)]
struct Config { window: usize }
#[derive(Clone, DomainInput)]
struct Request { config: Config }
#[test]
fn nested_config() {
    let _domain = domain!(Request { config { window.in_range(1..=32) } });
}
~~~

## CONSTRUCTIVE

- fixed_value(value): one typed value.
- selected_values(values): explicit compatible scalar choices; use this for strings.
- in_range(range): ordered Rust range supported by the scalar type.
- one_of_ranges(ranges): disconnected ordered ranges.
- all_enum_values() / selected_enum_values(values): choose enum variants; payload rules belong to ENUM_RULE.

Incomplete, overlapping/ambiguous, contradictory, or unmaterializable families fail closed when registered. Do not replace constraints with arbitrary predicates or baseline templates.

~~~rust
use sun_observations::certifications::*;
#[derive(Clone, DomainInput)]
struct Batch { region: String, delta: i32, retry: bool }
#[test]
fn explicit_spaces() {
    let _domain = domain!(Batch {
        region.selected_values(["east".to_owned(), "west".to_owned()]),
        delta.one_of_ranges([-10..=-1, 1..=10]),
        retry.fixed_value(false),
    });
}
~~~

## ENUM_RULE

For unit enums, field.all_enum_values() or field.selected_enum_values([Enum::Variant, ...]) suffices. For payload enums, choose variants inside the enum field block, then specify each selected payload with EnumType::Variant { RULE, ... }. The runner can cover individual enum choices and feasible pairwise combinations; enforcement belongs to certifications-api.

~~~rust
use sun_observations::certifications::*;
#[derive(Clone, DomainInput)]
enum Strategy { Direct, Window { size: usize } }
#[derive(Clone, DomainInput)]
struct Request { strategy: Strategy }
#[test]
fn payload_variants() {
    let _domain = domain!(Request {
        strategy {
            all_enum_values(),
            Strategy::Window { size.in_range(1..=8) },
        },
    });
}
~~~

Unnamed enum payload positions use `_0`, `_1`, ... in declaration order, inside the same brace-form variant block (not tuple parentheses).

~~~rust
use sun_observations::certifications::*;
#[derive(Clone, DomainInput)]
enum Item { Number(i32), Pair(i32, bool) }
#[derive(Clone, DomainInput)]
struct Input { item: Item }
#[test]
fn tuple_payload_positions() {
    let family = domain!(Input {
        item {
            all_enum_values(),
            Item::Number { _0.in_range(1..=3) },
            Item::Pair { _0.in_range(1..=3), _1.fixed_value(true) },
        },
    });
    let mut runner = certification("tuple payload constraints").executor(|input: &Input| {
        match input.item {
            Item::Number(n) | Item::Pair(n, true) if (1..=3).contains(&n) => Ok(n),
            _ => Err(()),
        }
    });
    runner.domain(family);
    let case = runner.case();
    runner.check(fact!((1..=3).contains(case.result())).expect(true));
    runner.assert_certify();
}
~~~

## COLLECTION_RULE

COLLECTION { length().SPACE, each(SPACE) } generates scalar elements. Use each { RULE, ... } for structured elements. length() is usize; choose fixed_value(n), selected_values(lengths), in_range(range), or one_of_ranges(ranges), plus an element source. SPACE is a constructive rule; enum selection is not a collection-length rule.

~~~rust
use sun_observations::certifications::*;
#[derive(Clone, DomainInput)]
struct Sample { value: f64 }
#[derive(Clone, DomainInput)]
struct Batch { ids: Vec<u64>, samples: Vec<Sample> }
#[test]
fn collection_shapes() {
    let _domain = domain!(Batch {
        ids { length().in_range(1..=4), each(in_range(1..=100)) },
        samples { length().fixed_value(2), each { value.in_range(-1.0..=1.0) } },
    });
}
~~~

## WHEN

when(relation) { RULE, ... } activates a rule set under a typed condition. Conditional constructive sources must be complete and unambiguous for every reachable branch. This controls input generation, not planned-check execution.

~~~rust
use sun_observations::certifications::*;
#[derive(Clone, DomainInput)]
enum Mode { Fast, Accurate }
#[derive(Clone, DomainInput)]
struct Request { mode: Mode, window: usize }
#[test]
fn conditional_windows() {
    let _domain = domain!(Request {
        mode.all_enum_values(),
        when(mode.equal(Mode::Fast)) { window.in_range(1..=3) },
        when(mode.equal(Mode::Accurate)) { window.in_range(4..=8) },
    });
}
~~~

## RELATION

Use equal(operand), not_equal(operand), less_than(operand), less_equal(operand), greater_than(operand), greater_equal(operand) with a compatible scalar constant or typed field operand. Nested field paths and collection.length() are supported. Constructive rules choose candidates; relations constrain their legal combinations.

~~~rust
use sun_observations::certifications::*;
#[derive(Clone, DomainInput)]
struct Window { start: usize, end: usize, values: Vec<f64> }
#[test]
fn bounded_window() {
    let _domain = domain!(Window {
        start.in_range(0..=3), end.in_range(1..=4),
        values { length().fixed_value(4), each(in_range(-1.0..=1.0)) },
        start.less_than(end), end.less_equal(values.length()),
    });
}
~~~
