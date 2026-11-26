//! Truncation and budget splitting for a status's output and a failed status's call arguments.

use std::borrow::Cow;
use std::mem;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::host;

use super::Limits;

/// `call` with its arguments truncated by the rule on [`crate::Status::Failed`]'s `calls`.
pub(super) fn truncate_args(mut call: host::Call, limits: &Limits) -> host::Call {
    let mut chars_left = limits.output_chars / limits.calls_per_execute.max(1);
    let mut kept = Vec::new();
    let mut dropped = Map::new();
    for (arg, value) in mem::take(&mut call.args) {
        let arg_chars = arg.chars().count();
        if dropped.is_empty()
            && let Some(value_share) = chars_left.checked_sub(arg_chars)
        {
            let text = to_text(&value);
            let text_chars = text.chars().count();
            let truncated = truncate_text(&text, text_chars, value_share);
            drop(text);
            let value_chars = truncated
                .as_ref()
                .map_or(text_chars, |text| text.chars().count());
            if value_chars <= value_share {
                chars_left = value_share - value_chars;
                kept.push((arg, value, truncated, arg_chars + value_chars));
                continue;
            }
        }
        dropped.insert(arg, value);
    }
    if !dropped.is_empty() {
        let mut text_chars = count_json_chars(&dropped);
        while text_chars > chars_left
            && chars_left
                < format_truncation_marker(text_chars, text_chars)
                    .chars()
                    .count()
            && let Some((arg, value, _, used_chars)) = kept.pop()
        {
            text_chars += count_json_chars(&arg) + ":".len() + count_json_chars(&value) + ",".len();
            chars_left += used_chars;
            dropped.insert(arg, value);
        }
        let text = serde_json::to_string(&dropped).expect("a Map always serializes");
        call.truncated_args = Some(truncate_text(&text, text_chars, chars_left).unwrap_or(text));
    }
    call.args = kept
        .into_iter()
        .map(|(arg, value, truncated, _)| (arg, truncated.map_or(value, Value::String)))
        .collect();
    call
}

/// The text that `value` is measured by against [`Limits::output_chars`], and truncated when it
/// doesn't fit: a string's own text, or any other value's JSON text.
pub(super) fn to_text(value: &Value) -> Cow<'_, str> {
    match value {
        Value::String(text) => Cow::Borrowed(text),
        other => Cow::Owned(serde_json::to_string(other).expect("a Value always serializes")),
    }
}

/// `text`, which is `text_chars` characters long, truncated to its first and last characters
/// around a marker such as `[… 60,000 of 100,000 characters truncated …]`, up to `max_chars` in
/// all with the marker, or to the marker alone when `max_chars` is smaller than it. `None` if
/// `text` already fits in `max_chars`.
pub(super) fn truncate_text(text: &str, text_chars: usize, max_chars: usize) -> Option<String> {
    if text_chars <= max_chars {
        return None;
    }
    let longest_marker = format_truncation_marker(text_chars, text_chars);
    let kept_chars = max_chars.saturating_sub(longest_marker.chars().count());
    let tail_chars = kept_chars / 2;
    let head_end = text
        .char_indices()
        .nth(kept_chars - tail_chars)
        .map_or(text.len(), |(byte, _)| byte);
    let tail_start = text
        .char_indices()
        .rev()
        .take(tail_chars)
        .last()
        .map_or(text.len(), |(byte, _)| byte);
    let head = &text[..head_end];
    let tail = &text[tail_start..];
    let marker = format_truncation_marker(text_chars - kept_chars, text_chars);
    Some(format!("{head}{marker}{tail}"))
}

/// The marker that stands in for `truncated_chars` characters truncated from a text of
/// `text_chars` characters, such as `[… 60,000 of 100,000 characters truncated …]`.
fn format_truncation_marker(truncated_chars: usize, text_chars: usize) -> String {
    let truncated_count = group_thousands(truncated_chars);
    let total_count = group_thousands(text_chars);
    format!("[… {truncated_count} of {total_count} characters truncated …]")
}

/// How many characters `value`'s JSON text has.
fn count_json_chars(value: &impl Serialize) -> usize {
    serde_json::to_string(value)
        .expect("a Map, Value or String always serializes")
        .chars()
        .count()
}

/// `number` in digits with a comma between each group of three, such as `100,000`.
fn group_thousands(number: usize) -> String {
    let digits = number.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// How many of `budget` characters two texts of `first_chars` and `second_chars` each get, as
/// `(first_share, second_share)`. Each gets what it needs, and when both need more, they split
/// the budget evenly, the first taking any odd character.
pub(super) fn split_budget(
    budget: usize,
    first_chars: usize,
    second_chars: usize,
) -> (usize, usize) {
    let second_share = budget
        .saturating_sub(first_chars)
        .max(budget / 2)
        .min(second_chars);
    (budget - second_share, second_share)
}
