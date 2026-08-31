//! Bounded, named feature values for provider-independent inference inputs.

use std::collections::BTreeMap;

use crate::limits::FeatureLimits;
use crate::vector::Vector;
use crate::{MlError, Result};

/// Stable validated feature name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeatureName(String);

impl FeatureName {
    /// Creates a non-empty bounded name.
    pub fn new(value: impl Into<String>, limits: FeatureLimits) -> Result<Self> {
        let value = value.into();
        if value.is_empty() || value.len() > limits.max_name_bytes() {
            return Err(MlError::new(
                "ML-FEATURE-001",
                "feature name is empty or exceeds the configured byte limit",
            ));
        }
        Ok(Self(value))
    }

    /// Name text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Portable ML feature value.
#[derive(Debug, Clone, PartialEq)]
pub enum FeatureValue {
    /// Finite numeric feature.
    Numeric(f64),
    /// Boolean feature.
    Boolean(bool),
    /// Bounded categorical text.
    Categorical(String),
    /// Validated dense vector feature.
    Vector(Vector),
}

impl FeatureValue {
    /// Creates a finite numeric feature.
    pub fn numeric(value: f64) -> Result<Self> {
        if value.is_finite() {
            Ok(Self::Numeric(value))
        } else {
            Err(MlError::new(
                "ML-FEATURE-002",
                "numeric features must be finite",
            ))
        }
    }

    /// Creates a bounded categorical feature.
    pub fn categorical(value: impl Into<String>, limits: FeatureLimits) -> Result<Self> {
        let value = value.into();
        if value.len() > limits.max_category_bytes() {
            return Err(MlError::new(
                "ML-FEATURE-003",
                "categorical feature exceeds the configured byte limit",
            ));
        }
        Ok(Self::Categorical(value))
    }
}

/// Canonically ordered unique feature set.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FeatureSet {
    values: BTreeMap<FeatureName, FeatureValue>,
}

impl FeatureSet {
    /// Creates an empty feature set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            values: BTreeMap::new(),
        }
    }

    /// Adds one unique feature under hard count limits.
    pub fn insert(
        &mut self,
        name: FeatureName,
        value: FeatureValue,
        limits: FeatureLimits,
    ) -> Result<()> {
        if self.values.contains_key(&name) {
            return Err(MlError::new(
                "ML-FEATURE-004",
                "feature set contains a duplicate name",
            ));
        }
        if self.values.len() >= limits.max_features() {
            return Err(MlError::new(
                "ML-FEATURE-005",
                "feature count exceeds the configured limit",
            ));
        }
        self.values.insert(name, value);
        Ok(())
    }

    /// Number of features.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether no features are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Looks up a feature.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&FeatureValue> {
        self.values
            .iter()
            .find_map(|(key, value)| (key.as_str() == name).then_some(value))
    }

    /// Canonical name/value iteration.
    pub fn iter(&self) -> impl Iterator<Item = (&FeatureName, &FeatureValue)> {
        self.values.iter()
    }
}

#[cfg(test)]
mod tests {
    use crate::limits::FeatureLimits;

    use super::{FeatureName, FeatureSet, FeatureValue};

    #[test]
    fn features_are_bounded_and_unique() {
        let limits = FeatureLimits::default();
        let name = FeatureName::new("age", limits).unwrap();
        let mut set = FeatureSet::new();
        set.insert(name.clone(), FeatureValue::numeric(42.0).unwrap(), limits)
            .unwrap();
        assert!(
            set.insert(name, FeatureValue::Boolean(true), limits)
                .is_err()
        );
        assert!(FeatureValue::numeric(f64::INFINITY).is_err());
    }
}
