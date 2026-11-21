use std::fmt::{self, Write as _};
use std::sync::{Arc, Mutex};

use cucumber::{given, then};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Dispatch, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

use crate::World;

/// A tracing layer that keeps every span it sees, shared between the subscriber and the steps.
#[derive(Clone, Default)]
pub(crate) struct SpanCapture(Arc<Mutex<Vec<CapturedSpan>>>);

/// One span: its id, its name, whether it has a parent, and each field it recorded.
#[derive(Clone, Debug)]
pub(crate) struct CapturedSpan {
    pub(crate) id: Id,
    pub(crate) name: String,
    pub(crate) has_parent: bool,
    pub(crate) fields: Vec<(String, String)>,
}

impl SpanCapture {
    fn spans(&self) -> Vec<CapturedSpan> {
        self.0.lock().expect("captured spans").clone()
    }
}

struct FieldVisitor<'a>(&'a mut Vec<(String, String)>);

impl Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let mut text = String::new();
        write!(text, "{value:?}").expect("format a field");
        self.0.push((field.name().to_string(), text));
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SpanCapture {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let has_parent = if attrs.is_contextual() {
            ctx.current_span().id().is_some()
        } else {
            attrs.parent().is_some()
        };
        let mut span = CapturedSpan {
            id: id.clone(),
            name: attrs.metadata().name().to_string(),
            has_parent,
            fields: Vec::new(),
        };
        attrs.record(&mut FieldVisitor(&mut span.fields));
        self.0.lock().expect("captured spans").push(span);
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut spans = self.0.lock().expect("captured spans");
        let span = spans.iter_mut().rev().find(|span| span.id == *id);
        values.record(&mut FieldVisitor(&mut span.expect("recorded span").fields));
    }
}

#[given("a subscriber that captures every span")]
fn capture_every_span(world: &mut World) {
    let subscriber = tracing_subscriber::registry().with(world.span_capture.clone());
    world.dispatch = Dispatch::new(subscriber);
}

#[then(regex = r"^the captured spans are (\w+) and (\w+)$")]
fn captured_spans_are(world: &mut World, first: String, second: String) {
    let mut names: Vec<String> = world
        .span_capture
        .spans()
        .into_iter()
        .map(|span| span.name)
        .collect();
    names.sort();
    names.dedup();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(names, expected);
}

#[then(regex = r"^the captured span fields are only (.+)$")]
fn captured_fields_are_only(world: &mut World, allowed: String) {
    let allowed: Vec<&str> = allowed
        .split([',', ' '])
        .filter(|word| !word.is_empty() && *word != "and")
        .collect();
    for span in world.span_capture.spans() {
        for (field, _) in &span.fields {
            assert!(
                allowed.contains(&field.as_str()),
                "{} has field {field}",
                span.name
            );
        }
    }
}

#[then(regex = r"^the (\w+) span has no parent$")]
fn span_has_no_parent(world: &mut World, name: String) {
    let spans = world.span_capture.spans();
    let mut opened = spans.iter().filter(|span| span.name == name).peekable();
    assert!(opened.peek().is_some(), "no {name} span");
    assert!(opened.all(|span| !span.has_parent), "{spans:?}");
}

#[then(regex = r"^the (\w+) span's `(\w+)` equals the (\w+) span's$")]
fn span_field_equals(world: &mut World, name: String, field: String, other: String) {
    let value = span_field(world, &name, &field);
    let value = value.unwrap_or_else(|| panic!("{name} has no {field}"));
    assert_eq!(Some(value), span_field(world, &other, &field));
}

#[then(regex = r#"^no captured span field holds "(.+)"$"#)]
fn no_field_holds(world: &mut World, text: String) {
    for span in world.span_capture.spans() {
        for (field, value) in &span.fields {
            assert!(!value.contains(&text), "{}.{field} = {value}", span.name);
        }
    }
}

/// The last value that the last captured span named `name` recorded for `field`, if any.
/// Panics if no span has that name.
fn span_field(world: &World, name: &str, field: &str) -> Option<String> {
    let spans = world.span_capture.spans();
    let span = spans.into_iter().rev().find(|span| span.name == name);
    let fields = span.unwrap_or_else(|| panic!("no {name} span")).fields;
    let value = fields.into_iter().rev().find(|(key, _)| key == field);
    value.map(|(_, value)| value)
}

#[then(regex = r"^the (\w+) span's `(\w+)` is (.+)$")]
fn span_field_is(world: &mut World, name: String, field: String, expected: String) {
    assert_eq!(span_field(world, &name, &field), Some(expected));
}

#[then(regex = r"^the (\w+) span records a `(\w+)`$")]
fn span_records(world: &mut World, name: String, field: String) {
    let value = span_field(world, &name, &field);
    assert!(value.is_some(), "{name} has no {field}");
}
