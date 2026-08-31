use core::fmt;
use std::collections::BTreeMap;

use crate::diagnostic::{Diagnostic, Result};
use crate::fingerprint::{CanonicalHasher, Fingerprint};
use crate::types::TypeDef;
use crate::value::Datum;

/// Stable semantic identity of a DOL function family.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FunctionKey(String);

impl FunctionKey {
    /// Creates a namespaced semantic function key.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Stable namespaced function key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether repeated calls may depend on anything other than their arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FunctionDeterminism {
    /// The same semantic arguments always produce the same semantic result.
    Deterministic,
    /// The result may depend on explicit evaluation context such as a pinned clock.
    Contextual,
    /// Repeated evaluation may produce different results even in one context.
    Volatile,
}

/// Portable semantic definition of a function used by DOL expressions.
///
/// This definition is portable semantic data. It intentionally contains no Rust
/// closure, function pointer, backend function name, or process-local type identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionDef {
    key: FunctionKey,
    version: u32,
    determinism: FunctionDeterminism,
    semantic_dependencies: BTreeMap<String, String>,
}

impl FunctionDef {
    /// Creates a semantic function definition.
    #[must_use]
    pub fn new(key: impl Into<String>, version: u32, determinism: FunctionDeterminism) -> Self {
        Self {
            key: FunctionKey::new(key),
            version,
            determinism,
            semantic_dependencies: BTreeMap::new(),
        }
    }

    pub(crate) fn builtin(
        key: &'static str,
        version: u32,
        determinism: FunctionDeterminism,
    ) -> Self {
        Self::new(key, version, determinism)
    }

    /// Pins an external semantic dependency such as a Unicode data version.
    #[must_use]
    pub fn with_dependency(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.semantic_dependencies
            .insert(name.into(), version.into());
        self
    }

    /// Stable function-family identity.
    #[must_use]
    pub fn key(&self) -> &FunctionKey {
        &self.key
    }

    /// Semantic function version.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Determinism classification used by normalization and future placement.
    #[must_use]
    pub fn determinism(&self) -> FunctionDeterminism {
        self.determinism
    }

    /// Canonically ordered external semantic dependencies.
    #[must_use]
    pub fn semantic_dependencies(&self) -> &BTreeMap<String, String> {
        &self.semantic_dependencies
    }

    /// Exact semantic fingerprint of this function definition.
    #[must_use]
    pub fn fingerprint(&self) -> Fingerprint {
        let mut hasher = CanonicalHasher::new(b"function/v1");
        hash_function_def(&mut hasher, self);
        hasher.finish()
    }
}

/// Local implementation contract for a primitive portable semantic function.
///
/// Type authors normally prefer methods that lower into existing DOL expressions.
/// This trait is for operations that are semantically primitive. `definition` is
/// the portable identity; `validate` and `evaluate` are local conformance hooks and
/// are never included in expression fingerprints or wire identity.
pub trait SemanticFunction: 'static {
    /// Portable semantic function definition.
    fn definition() -> FunctionDef;

    /// Validates one call's exact argument and result semantic types.
    fn validate(arguments: &[TypeDef], result: &TypeDef) -> Result<()>;

    /// Normative local evaluation for one already-validated call.
    fn evaluate(arguments: Vec<Datum>, result: &TypeDef) -> Result<Datum>;
}

pub(crate) fn validate_function_def(function: &FunctionDef) -> Result<()> {
    let key = function.key().as_str();
    let valid_key = !key.is_empty()
        && !key.starts_with('/')
        && !key.ends_with('/')
        && key.split('/').count() >= 2
        && key.split('/').all(|segment| !segment.is_empty());
    if !valid_key {
        return Err(Diagnostic::error(
            "EXPR-FUNC-001",
            "function key must be a non-empty namespaced path such as `org.example/function`",
        ));
    }
    if function.version() == 0 {
        return Err(Diagnostic::error(
            "EXPR-FUNC-002",
            "function semantic version must be greater than zero",
        ));
    }
    for (name, version) in function.semantic_dependencies() {
        if name.is_empty() || version.is_empty() {
            return Err(Diagnostic::error(
                "EXPR-FUNC-003",
                "function semantic dependencies require non-empty names and versions",
            ));
        }
    }
    Ok(())
}

pub(crate) fn hash_function_def(hasher: &mut CanonicalHasher, function: &FunctionDef) {
    hasher.str(function.key.as_str());
    hasher.u32(function.version);
    hasher.u8(match function.determinism {
        FunctionDeterminism::Deterministic => 0,
        FunctionDeterminism::Contextual => 1,
        FunctionDeterminism::Volatile => 2,
    });
    hasher.u64(function.semantic_dependencies.len() as u64);
    for (name, version) in &function.semantic_dependencies {
        hasher.str(name);
        hasher.str(version);
    }
}

type ValidateFunction = fn(&FunctionDef, &[TypeDef], &TypeDef) -> Result<()>;
type EvaluateFunction = fn(&FunctionDef, Vec<Datum>, &TypeDef) -> Result<Datum>;

/// Private executable witness paired with a portable [`FunctionDef`].
#[derive(Clone)]
pub(crate) struct FunctionRef {
    definition: FunctionDef,
    validate: ValidateFunction,
    evaluate: EvaluateFunction,
}

impl FunctionRef {
    pub(crate) fn new(
        definition: FunctionDef,
        validate: ValidateFunction,
        evaluate: EvaluateFunction,
    ) -> Self {
        Self {
            definition,
            validate,
            evaluate,
        }
    }

    pub(crate) fn semantic<F: SemanticFunction>() -> Self {
        Self::new(
            F::definition(),
            validate_semantic_function::<F>,
            evaluate_semantic_function::<F>,
        )
    }

    pub(crate) fn definition(&self) -> &FunctionDef {
        &self.definition
    }

    pub(crate) fn validate(&self, arguments: &[TypeDef], result: &TypeDef) -> Result<()> {
        validate_function_def(&self.definition)?;
        (self.validate)(&self.definition, arguments, result)
    }

    pub(crate) fn evaluate(&self, arguments: Vec<Datum>, result: &TypeDef) -> Result<Datum> {
        (self.evaluate)(&self.definition, arguments, result)
    }
}

fn validate_semantic_function<F: SemanticFunction>(
    definition: &FunctionDef,
    arguments: &[TypeDef],
    result: &TypeDef,
) -> Result<()> {
    if definition.key().as_str().starts_with("dol/") {
        return Err(Diagnostic::error(
            "EXPR-FUNC-005",
            "custom semantic functions may not claim the reserved `dol/` namespace",
        ));
    }
    if definition.determinism() != FunctionDeterminism::Deterministic {
        return Err(Diagnostic::error(
            "EXPR-FUNC-006",
            "Phase 2 custom semantic functions must be deterministic; contextual and volatile functions require a future explicit evaluation-context contract",
        ));
    }
    F::validate(arguments, result)
}

fn evaluate_semantic_function<F: SemanticFunction>(
    _definition: &FunctionDef,
    arguments: Vec<Datum>,
    result: &TypeDef,
) -> Result<Datum> {
    F::evaluate(arguments, result)
}

impl fmt::Debug for FunctionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FunctionRef")
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BuiltinFunction {
    TextLowercase,
    TextUppercase,
    TextTrim,
    TextTrimStart,
    TextTrimEnd,
    TextContains,
    TextStartsWith,
    TextEndsWith,
    TextByteLength,
    TextScalarLength,
    DateYear,
    DateMonth,
    DateDay,
    DateOrdinal,
    DateWeekday,
    LocalDateTimeDate,
    LocalDateTimeTime,
    InstantUnixTimestamp,
    DurationWholeSeconds,
}

impl BuiltinFunction {
    pub(crate) fn definition(self) -> FunctionDef {
        use FunctionDeterminism::Deterministic;

        match self {
            Self::TextLowercase => unicode_function("dol/text/to-lowercase"),
            Self::TextUppercase => unicode_function("dol/text/to-uppercase"),
            Self::TextTrim => unicode_function("dol/text/trim"),
            Self::TextTrimStart => unicode_function("dol/text/trim-start"),
            Self::TextTrimEnd => unicode_function("dol/text/trim-end"),
            Self::TextContains => FunctionDef::builtin("dol/text/contains", 1, Deterministic),
            Self::TextStartsWith => FunctionDef::builtin("dol/text/starts-with", 1, Deterministic),
            Self::TextEndsWith => FunctionDef::builtin("dol/text/ends-with", 1, Deterministic),
            Self::TextByteLength => FunctionDef::builtin("dol/text/byte-length", 1, Deterministic),
            Self::TextScalarLength => {
                FunctionDef::builtin("dol/text/scalar-length", 1, Deterministic)
            }
            Self::DateYear => FunctionDef::builtin("dol/date/year", 1, Deterministic),
            Self::DateMonth => FunctionDef::builtin("dol/date/month", 1, Deterministic),
            Self::DateDay => FunctionDef::builtin("dol/date/day", 1, Deterministic),
            Self::DateOrdinal => FunctionDef::builtin("dol/date/ordinal", 1, Deterministic),
            Self::DateWeekday => FunctionDef::builtin("dol/date/weekday", 1, Deterministic),
            Self::LocalDateTimeDate => {
                FunctionDef::builtin("dol/local-datetime/date", 1, Deterministic)
            }
            Self::LocalDateTimeTime => {
                FunctionDef::builtin("dol/local-datetime/time", 1, Deterministic)
            }
            Self::InstantUnixTimestamp => {
                FunctionDef::builtin("dol/instant/unix-timestamp", 1, Deterministic)
            }
            Self::DurationWholeSeconds => {
                FunctionDef::builtin("dol/duration/whole-seconds", 1, Deterministic)
            }
        }
    }
}

fn unicode_function(key: &'static str) -> FunctionDef {
    let (major, minor, patch) = char::UNICODE_VERSION;
    FunctionDef::builtin(key, 1, FunctionDeterminism::Deterministic)
        .with_dependency("unicode", format!("{major}.{minor}.{patch}"))
}
