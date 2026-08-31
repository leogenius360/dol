use super::super::author::{Expr, ExprSource};
use super::super::call::builtin_call1;
use super::super::function::BuiltinFunction;

/// Symbolic counterpart of portable `time::Date` value methods.
pub trait DateExprExt: ExprSource<Value = time::Date> + Sized {
    /// Mirrors `Date::year`.
    #[must_use]
    fn year(self) -> Expr<i32> {
        builtin_call1(BuiltinFunction::DateYear, self)
    }

    /// Mirrors `Date::month` and preserves the `time::Month` domain type.
    #[must_use]
    fn month(self) -> Expr<time::Month> {
        builtin_call1(BuiltinFunction::DateMonth, self)
    }

    /// Mirrors `Date::day`.
    #[must_use]
    fn day(self) -> Expr<u8> {
        builtin_call1(BuiltinFunction::DateDay, self)
    }

    /// Mirrors `Date::ordinal`.
    #[must_use]
    fn ordinal(self) -> Expr<u16> {
        builtin_call1(BuiltinFunction::DateOrdinal, self)
    }

    /// Mirrors `Date::weekday` and preserves the `time::Weekday` domain type.
    #[must_use]
    fn weekday(self) -> Expr<time::Weekday> {
        builtin_call1(BuiltinFunction::DateWeekday, self)
    }
}

impl<S> DateExprExt for S where S: ExprSource<Value = time::Date> + Sized {}

/// Nullable symbolic counterpart of portable `time::Date` value methods.
pub trait NullableDateExprExt: ExprSource<Value = Option<time::Date>> + Sized {
    /// Extracts year while preserving null/missing state.
    #[must_use]
    fn year(self) -> Expr<Option<i32>> {
        builtin_call1(BuiltinFunction::DateYear, self)
    }

    /// Extracts month while preserving null/missing state.
    #[must_use]
    fn month(self) -> Expr<Option<time::Month>> {
        builtin_call1(BuiltinFunction::DateMonth, self)
    }

    /// Extracts day while preserving null/missing state.
    #[must_use]
    fn day(self) -> Expr<Option<u8>> {
        builtin_call1(BuiltinFunction::DateDay, self)
    }

    /// Extracts ordinal day while preserving null/missing state.
    #[must_use]
    fn ordinal(self) -> Expr<Option<u16>> {
        builtin_call1(BuiltinFunction::DateOrdinal, self)
    }

    /// Extracts weekday while preserving null/missing state.
    #[must_use]
    fn weekday(self) -> Expr<Option<time::Weekday>> {
        builtin_call1(BuiltinFunction::DateWeekday, self)
    }
}

impl<S> NullableDateExprExt for S where S: ExprSource<Value = Option<time::Date>> + Sized {}

/// Symbolic counterpart of portable `time::PrimitiveDateTime` methods.
pub trait LocalDateTimeExprExt: ExprSource<Value = time::PrimitiveDateTime> + Sized {
    /// Mirrors `PrimitiveDateTime::date` without inventing a timezone.
    #[must_use]
    fn date(self) -> Expr<time::Date> {
        builtin_call1(BuiltinFunction::LocalDateTimeDate, self)
    }

    /// Mirrors `PrimitiveDateTime::time` without inventing a timezone.
    #[must_use]
    fn time(self) -> Expr<time::Time> {
        builtin_call1(BuiltinFunction::LocalDateTimeTime, self)
    }
}

impl<S> LocalDateTimeExprExt for S where S: ExprSource<Value = time::PrimitiveDateTime> + Sized {}

/// Nullable symbolic counterpart of `time::PrimitiveDateTime` methods.
pub trait NullableLocalDateTimeExprExt:
    ExprSource<Value = Option<time::PrimitiveDateTime>> + Sized
{
    /// Extracts local date while preserving null/missing state.
    #[must_use]
    fn date(self) -> Expr<Option<time::Date>> {
        builtin_call1(BuiltinFunction::LocalDateTimeDate, self)
    }

    /// Extracts local time while preserving null/missing state.
    #[must_use]
    fn time(self) -> Expr<Option<time::Time>> {
        builtin_call1(BuiltinFunction::LocalDateTimeTime, self)
    }
}

impl<S> NullableLocalDateTimeExprExt for S where
    S: ExprSource<Value = Option<time::PrimitiveDateTime>> + Sized
{
}

/// Symbolic counterpart of portable `time::OffsetDateTime` instant methods.
pub trait InstantExprExt: ExprSource<Value = time::OffsetDateTime> + Sized {
    /// Mirrors `OffsetDateTime::unix_timestamp`.
    #[must_use]
    fn unix_timestamp(self) -> Expr<i64> {
        builtin_call1(BuiltinFunction::InstantUnixTimestamp, self)
    }
}

impl<S> InstantExprExt for S where S: ExprSource<Value = time::OffsetDateTime> + Sized {}

/// Nullable symbolic counterpart of portable instant methods.
pub trait NullableInstantExprExt: ExprSource<Value = Option<time::OffsetDateTime>> + Sized {
    /// Returns Unix-epoch seconds while preserving null/missing state.
    #[must_use]
    fn unix_timestamp(self) -> Expr<Option<i64>> {
        builtin_call1(BuiltinFunction::InstantUnixTimestamp, self)
    }
}

impl<S> NullableInstantExprExt for S where
    S: ExprSource<Value = Option<time::OffsetDateTime>> + Sized
{
}

/// Symbolic counterpart of portable `time::Duration` methods.
pub trait DurationExprExt: ExprSource<Value = time::Duration> + Sized {
    /// Mirrors `Duration::whole_seconds`.
    #[must_use]
    fn whole_seconds(self) -> Expr<i64> {
        builtin_call1(BuiltinFunction::DurationWholeSeconds, self)
    }
}

impl<S> DurationExprExt for S where S: ExprSource<Value = time::Duration> + Sized {}

/// Nullable symbolic counterpart of portable duration methods.
pub trait NullableDurationExprExt: ExprSource<Value = Option<time::Duration>> + Sized {
    /// Returns complete seconds while preserving null/missing state.
    #[must_use]
    fn whole_seconds(self) -> Expr<Option<i64>> {
        builtin_call1(BuiltinFunction::DurationWholeSeconds, self)
    }
}

impl<S> NullableDurationExprExt for S where S: ExprSource<Value = Option<time::Duration>> + Sized {}
