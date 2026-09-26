//! Helpers shared by the typed value serde implementations.

use std::cell::Cell;
use std::fmt;
use std::thread::LocalKey;

use ::serde::Deserialize;
use ::serde::de::{self, Deserializer, SeqAccess, Visitor};
use ::serde::ser;

use crate::value::MAX_VALUE_DEPTH;

thread_local! {
    /// Containers (lists, maps, objects, variants) currently being
    /// deserialized on this thread.
    static DESERIALIZE_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// Containers currently being serialized on this thread.
    static SERIALIZE_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Tracks the container nesting of typed value (de)serialization.
///
/// Serde recurses once per nesting level, so unbounded input (for example a
/// corrupt or malicious MessagePack payload) could overflow the stack.
/// Every container visit holds a guard; entering a container deeper than
/// [`MAX_VALUE_DEPTH`] fails. Serialization is limited too, so everything
/// serialized can be deserialized again. The counters are per thread
/// because serde runs synchronously on the calling thread.
pub(crate) struct DepthGuard(&'static LocalKey<Cell<usize>>);

impl DepthGuard {
    /// Enter a container while deserializing.
    pub(crate) fn enter<E: de::Error>() -> Result<Self, E> {
        Self::enter_with(&DESERIALIZE_DEPTH).map_err(E::custom)
    }

    /// Enter a container while serializing.
    pub(crate) fn enter_serialize<E: ser::Error>() -> Result<Self, E> {
        Self::enter_with(&SERIALIZE_DEPTH).map_err(E::custom)
    }

    fn enter_with(counter: &'static LocalKey<Cell<usize>>) -> Result<Self, String> {
        counter.with(|depth| {
            let next = depth.get() + 1;
            if next > MAX_VALUE_DEPTH {
                return Err(format!(
                    "value nesting exceeds the maximum depth of {MAX_VALUE_DEPTH}"
                ));
            }
            depth.set(next);
            Ok(Self(counter))
        })
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        self.0.with(|depth| depth.set(depth.get() - 1));
    }
}

/// The wire form of a duration: `(whole seconds, subsecond nanoseconds)`.
///
/// Durations were originally encoded as whole milliseconds (`i64`), which
/// lost sub-millisecond precision and wrapped for durations beyond
/// `i64::MAX` milliseconds; [`DurationWire`] still decodes that form.
pub(crate) fn duration_parts(duration: time::Duration) -> (i64, i32) {
    (duration.whole_seconds(), duration.subsec_nanoseconds())
}

/// A duration payload: a `(seconds, nanoseconds)` sequence (see
/// [`duration_parts`]) or, in the legacy form, an integer number of
/// milliseconds.
pub(crate) struct DurationWire(pub(crate) time::Duration);

impl<'de> Deserialize<'de> for DurationWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(DurationVisitor).map(Self)
    }
}

struct DurationVisitor;

impl<'de> Visitor<'de> for DurationVisitor {
    type Value = time::Duration;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a (seconds, nanoseconds) pair or an integer number of milliseconds")
    }

    fn visit_i64<E: de::Error>(self, milliseconds: i64) -> Result<Self::Value, E> {
        Ok(time::Duration::milliseconds(milliseconds))
    }

    fn visit_u64<E: de::Error>(self, milliseconds: u64) -> Result<Self::Value, E> {
        let milliseconds = i64::try_from(milliseconds)
            .map_err(|_| E::custom(format_args!("duration of {milliseconds} ms out of range")))?;
        Ok(time::Duration::milliseconds(milliseconds))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let seconds: i64 = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let nanoseconds: i32 = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        if seq.next_element::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::invalid_length(3, &self));
        }
        let consistent_sign =
            seconds == 0 || nanoseconds == 0 || (seconds < 0) == (nanoseconds < 0);
        if nanoseconds.unsigned_abs() >= 1_000_000_000 || !consistent_sign {
            return Err(de::Error::custom(format_args!(
                "invalid duration nanoseconds {nanoseconds} for {seconds} seconds"
            )));
        }
        Ok(time::Duration::new(seconds, nanoseconds))
    }
}
