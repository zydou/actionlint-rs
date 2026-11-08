//! Types for values in GitHub Actions expressions.
//!
//! This module is the Rust counterpart of the original `expr_type.go`. It defines the
//! [`ExprType`] trait and the concrete types used by the expression type checker
//! ([`crate::expr_sema`]) to reason about the static type of a value produced by an expression.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Trait shared by all expression types.
///
/// Implementors describe a static type of a value in the GitHub Actions expression language and
/// support assignability checks and merging used by the type checker.
pub trait ExprType: fmt::Display {
    /// Returns whether `other` can be assigned to this type.
    fn assignable(&self, other: &ExprTypeKind) -> bool;

    /// Merges `other` into this type. When the two conflict, the merged result falls back to
    /// [`AnyType`].
    fn merge(&self, other: &ExprTypeKind) -> ExprTypeKind;

    /// Returns a deep copy of this type where all child types are copied recursively.
    fn deep_copy(&self) -> ExprTypeKind;
}

/// An owned, cheaply cloneable handle to a concrete expression type.
///
/// This is the ergonomic wrapper used throughout the checker; the actual data lives in
/// [`ExprTypeKind`] behind an [`Arc`].
#[derive(Clone, PartialEq)]
pub struct ExprTypeKind(pub Arc<ExprTypeKindInner>);

impl fmt::Debug for ExprTypeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl ExprTypeKind {
    /// Creates a handle from a concrete inner type.
    #[must_use]
    pub fn new(inner: ExprTypeKindInner) -> Self {
        Self(Arc::new(inner))
    }

    /// Returns a reference to the inner type.
    #[must_use]
    pub fn inner(&self) -> &ExprTypeKindInner {
        &self.0
    }

    /// Returns a mutable reference to the inner type, cloning it first if it is shared.
    pub fn inner_mut(&mut self) -> &mut ExprTypeKindInner {
        Arc::make_mut(&mut self.0)
    }
}

impl fmt::Display for ExprTypeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl ExprType for ExprTypeKind {
    fn assignable(&self, other: &ExprTypeKind) -> bool {
        self.0.assignable(other)
    }

    fn merge(&self, other: &ExprTypeKind) -> ExprTypeKind {
        self.0.merge(other)
    }

    fn deep_copy(&self) -> ExprTypeKind {
        self.0.deep_copy()
    }
}

/// The concrete variants of an expression type.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprTypeKindInner {
    /// A type whose static value cannot be known; every type is assignable to and from it.
    Any,
    /// The type of a JSON `null` value.
    Null,
    /// The type of a number (integer or float).
    Number,
    /// The type of a boolean.
    Bool,
    /// The type of a string.
    String,
    /// The type of an object holding key-value pairs.
    Object(ObjectType),
    /// The type of an array.
    Array(ArrayType),
}

impl fmt::Display for ExprTypeKindInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => write!(f, "any"),
            Self::Null => write!(f, "null"),
            Self::Number => write!(f, "number"),
            Self::Bool => write!(f, "bool"),
            Self::String => write!(f, "string"),
            Self::Object(o) => o.fmt(f),
            Self::Array(a) => a.fmt(f),
        }
    }
}

impl ExprType for ExprTypeKindInner {
    fn assignable(&self, other: &ExprTypeKind) -> bool {
        match self {
            Self::Any => true,
            Self::Null => matches!(other.inner(), Self::Null | Self::Any),
            Self::Number => matches!(other.inner(), Self::Number | Self::Any),
            // Any type can be converted into bool, e.g. `if: ${{ steps.foo }}`.
            Self::Bool => true,
            // Bool and null can also be coerced into string, but in almost all cases that would
            // be a mistake so it is rejected here.
            Self::String => matches!(other.inner(), Self::String | Self::Number | Self::Any),
            Self::Object(o) => o.assignable(other),
            Self::Array(a) => a.assignable(other),
        }
    }

    fn merge(&self, other: &ExprTypeKind) -> ExprTypeKind {
        match self {
            Self::Any => ExprTypeKind::new(Self::Any),
            Self::Null => {
                if matches!(other.inner(), Self::Null) {
                    ExprTypeKind::new(Self::Null)
                } else {
                    ExprTypeKind::new(Self::Any)
                }
            }
            Self::Number => match other.inner() {
                Self::Number => ExprTypeKind::new(Self::Number),
                Self::String => other.clone(),
                _ => ExprTypeKind::new(Self::Any),
            },
            Self::Bool => match other.inner() {
                Self::Bool => ExprTypeKind::new(Self::Bool),
                Self::String => other.clone(),
                _ => ExprTypeKind::new(Self::Any),
            },
            Self::String => match other.inner() {
                Self::String | Self::Number | Self::Bool => ExprTypeKind::new(Self::String),
                _ => ExprTypeKind::new(Self::Any),
            },
            Self::Object(o) => o.merge(other),
            Self::Array(a) => a.merge(other),
        }
    }

    fn deep_copy(&self) -> ExprTypeKind {
        match self {
            Self::Object(o) => ExprTypeKind::new(Self::Object(o.deep_copy())),
            Self::Array(a) => ExprTypeKind::new(Self::Array(a.deep_copy())),
            other => ExprTypeKind::new(other.clone()),
        }
    }
}

/// Convenience constructors and helpers for [`ExprTypeKind`].
impl ExprTypeKind {
    /// The `any` type.
    #[must_use]
    pub fn any() -> Self {
        Self::new(ExprTypeKindInner::Any)
    }

    /// The `null` type.
    #[must_use]
    pub fn null() -> Self {
        Self::new(ExprTypeKindInner::Null)
    }

    /// The `number` type.
    #[must_use]
    pub fn number() -> Self {
        Self::new(ExprTypeKindInner::Number)
    }

    /// The `bool` type.
    #[must_use]
    pub fn bool() -> Self {
        Self::new(ExprTypeKindInner::Bool)
    }

    /// The `string` type.
    #[must_use]
    pub fn string() -> Self {
        Self::new(ExprTypeKindInner::String)
    }

    /// A loose object type that allows unknown props, mapping them to `any`.
    #[must_use]
    pub fn empty_object() -> Self {
        Self::new(ExprTypeKindInner::Object(ObjectType {
            props: BTreeMap::new(),
            mapped: Some(ExprTypeKind::any()),
        }))
    }

    /// A loose object type with the given props; unknown props map to `any`.
    #[must_use]
    pub fn object(props: BTreeMap<String, ExprTypeKind>) -> Self {
        Self::new(ExprTypeKindInner::Object(ObjectType {
            props,
            mapped: Some(ExprTypeKind::any()),
        }))
    }

    /// A strict object type that does not allow unknown props.
    #[must_use]
    pub fn empty_strict_object() -> Self {
        Self::new(ExprTypeKindInner::Object(ObjectType {
            props: BTreeMap::new(),
            mapped: None,
        }))
    }

    /// A strict object type with the given props; unknown props are rejected.
    #[must_use]
    pub fn strict_object(props: BTreeMap<String, ExprTypeKind>) -> Self {
        Self::new(ExprTypeKindInner::Object(ObjectType {
            props,
            mapped: None,
        }))
    }

    /// An object type that maps every key to `t`.
    #[must_use]
    pub fn map_object(t: ExprTypeKind) -> Self {
        Self::new(ExprTypeKindInner::Object(ObjectType {
            props: BTreeMap::new(),
            mapped: Some(t),
        }))
    }

    /// An array type whose elements have type `elem`.
    #[must_use]
    pub fn array(elem: ExprTypeKind) -> Self {
        Self::new(ExprTypeKindInner::Array(ArrayType { elem, deref: false }))
    }

    /// An array type derived from object filtering syntax (`foo.*`).
    #[must_use]
    pub fn deref_array(elem: ExprTypeKind) -> Self {
        Self::new(ExprTypeKindInner::Array(ArrayType { elem, deref: true }))
    }

    /// Returns `true` if this is an [`ExprTypeKindInner::Object`].
    #[must_use]
    pub fn as_object(&self) -> Option<&ObjectType> {
        match self.inner() {
            ExprTypeKindInner::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Returns `true` if this is an [`ExprTypeKindInner::Array`].
    #[must_use]
    pub fn as_array(&self) -> Option<&ArrayType> {
        match self.inner() {
            ExprTypeKindInner::Array(a) => Some(a),
            _ => None,
        }
    }
}

/// The type of an object, which can hold key-value pairs.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectType {
    /// Map from property name to its type.
    pub props: BTreeMap<String, ExprTypeKind>,
    /// The element type shared by all props. `Some(any)` means props can be any type (loose
    /// object); `None` means unknown props are rejected (strict object).
    ///
    /// Invariant: all types in `props` must be assignable to this type.
    pub mapped: Option<ExprTypeKind>,
}

impl Default for ObjectType {
    fn default() -> Self {
        Self {
            props: BTreeMap::new(),
            // An empty object defaults to a loose object that allows unknown props.
            mapped: Some(ExprTypeKind::any()),
        }
    }
}

impl ObjectType {
    /// Returns whether the object is strict (no unknown props allowed).
    #[must_use]
    pub fn is_strict(&self) -> bool {
        self.mapped.is_none()
    }

    /// Returns whether the object is loose (any unknown prop is allowed).
    pub fn is_loose(&self) -> bool {
        matches!(
            self.mapped.as_ref().map(ExprTypeKind::inner),
            Some(ExprTypeKindInner::Any)
        )
    }

    /// Marks the object as strict.
    pub fn set_strict(&mut self) {
        self.mapped = None;
    }

    /// Marks the object as loose.
    pub fn set_loose(&mut self) {
        self.mapped = Some(ExprTypeKind::any());
    }
}

impl fmt::Display for ObjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.is_strict() {
            if self.is_loose() {
                return write!(f, "object");
            }
            return write!(f, "{{string => {}}}", self.mapped.as_ref().unwrap());
        }

        write!(f, "{{")?;
        for (i, (name, ty)) in self.props.iter().enumerate() {
            if i > 0 {
                write!(f, "; ")?;
            }
            write!(f, "{name}: {ty}")?;
        }
        write!(f, "}}")
    }
}

impl ObjectType {
    fn assignable(&self, other: &ExprTypeKind) -> bool {
        if matches!(other.inner(), ExprTypeKindInner::Any) {
            return true;
        }
        let Some(other_obj) = other.as_object() else {
            return false;
        };

        if !self.is_strict() {
            if !other_obj.is_strict() {
                return self
                    .mapped
                    .as_ref()
                    .unwrap()
                    .assignable(other_obj.mapped.as_ref().unwrap());
            }
            return other_obj
                .props
                .values()
                .all(|t| self.mapped.as_ref().unwrap().assignable(t));
        }

        // self is strict
        if !other_obj.is_strict() {
            return self
                .props
                .values()
                .all(|t| t.assignable(other_obj.mapped.as_ref().unwrap()));
        }

        // both strict
        other_obj
            .props
            .iter()
            .all(|(n, r)| match self.props.get(n) {
                Some(l) => l.assignable(r),
                None => false,
            })
    }

    fn merge(&self, other: &ExprTypeKind) -> ExprTypeKind {
        let Some(other_obj) = other.as_object() else {
            return ExprTypeKind::any();
        };

        // Shortcuts
        if self.props.is_empty() && other_obj.is_loose() {
            return other.clone();
        }
        if other_obj.props.is_empty() && self.is_loose() {
            return ExprTypeKind::new(ExprTypeKindInner::Object(self.clone()));
        }

        let mut mapped = match (&self.mapped, &other_obj.mapped) {
            (None, o) => o.clone(),
            (m, None) => m.clone(),
            (Some(l), Some(r)) => Some(l.merge(r)),
        };

        let mut props = self.props.clone();
        for (n, r) in &other_obj.props {
            if let Some(l) = props.get(n) {
                props.insert(n.clone(), l.merge(r));
            } else {
                if let Some(m) = &mut mapped {
                    *m = m.merge(r);
                }
                props.insert(n.clone(), r.clone());
            }
        }

        ExprTypeKind::new(ExprTypeKindInner::Object(ObjectType { props, mapped }))
    }

    fn deep_copy(&self) -> ObjectType {
        ObjectType {
            props: self
                .props
                .iter()
                .map(|(n, t)| (n.clone(), t.deep_copy()))
                .collect(),
            mapped: self.mapped.as_ref().map(ExprTypeKind::deep_copy),
        }
    }
}

/// The type of an array.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayType {
    /// The type of the array elements.
    pub elem: ExprTypeKind,
    /// `true` when this type was derived from object filtering syntax (`foo.*`).
    pub deref: bool,
}

impl fmt::Display for ArrayType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "array<{}>", self.elem)
    }
}

impl ArrayType {
    fn assignable(&self, other: &ExprTypeKind) -> bool {
        match other.as_array() {
            Some(o) => self.elem.assignable(&o.elem),
            None => matches!(other.inner(), ExprTypeKindInner::Any),
        }
    }

    fn merge(&self, other: &ExprTypeKind) -> ExprTypeKind {
        match other.as_array() {
            Some(o) => {
                if matches!(self.elem.inner(), ExprTypeKindInner::Any) {
                    return ExprTypeKind::new(ExprTypeKindInner::Array(self.clone()));
                }
                if matches!(o.elem.inner(), ExprTypeKindInner::Any) {
                    return other.clone();
                }
                ExprTypeKind::new(ExprTypeKindInner::Array(ArrayType {
                    // Fusing array deref types means the prop deref chain breaks.
                    elem: self.elem.merge(&o.elem),
                    deref: false,
                }))
            }
            None => ExprTypeKind::any(),
        }
    }

    fn deep_copy(&self) -> ArrayType {
        ArrayType {
            elem: self.elem.deep_copy(),
            deref: self.deref,
        }
    }
}

/// Returns whether the two types are equal, in the sense that each is assignable to the other.
#[must_use]
pub fn equal_types(l: &ExprTypeKind, r: &ExprTypeKind) -> bool {
    l.assignable(r) && r.assignable(l)
}

/// Returns the expression type of a decoded JSON value.
///
/// This mirrors `typeOfJSONValue` in the original: JSON booleans map to [`ExprTypeKindInner::Bool`],
/// numbers to [`ExprTypeKindInner::Number`], strings to [`ExprTypeKindInner::String`], arrays to
/// [`ExprTypeKindInner::Array`], objects to a strict [`ExprTypeKindInner::Object`], and `null` to
/// [`ExprTypeKindInner::Null`].
pub fn type_of_json_value(v: &serde_json::Value) -> ExprTypeKind {
    match v {
        serde_json::Value::Bool(_) => ExprTypeKind::bool(),
        serde_json::Value::Number(_) => ExprTypeKind::number(),
        serde_json::Value::String(_) => ExprTypeKind::string(),
        serde_json::Value::Array(elems) => {
            let mut elem: Option<ExprTypeKind> = None;
            for e in elems {
                let t = type_of_json_value(e);
                elem = Some(match elem {
                    None => t,
                    Some(prev) => prev.merge(&t),
                });
            }
            ExprTypeKind::array(elem.unwrap_or_else(ExprTypeKind::any))
        }
        serde_json::Value::Object(map) => {
            let props = map
                .iter()
                .map(|(k, v)| (k.clone(), type_of_json_value(v)))
                .collect();
            ExprTypeKind::strict_object(props)
        }
        serde_json::Value::Null => ExprTypeKind::null(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_types() {
        assert_eq!(ExprTypeKind::any().to_string(), "any");
        assert_eq!(ExprTypeKind::string().to_string(), "string");
        assert_eq!(ExprTypeKind::number().to_string(), "number");
        assert_eq!(ExprTypeKind::bool().to_string(), "bool");
        assert_eq!(ExprTypeKind::null().to_string(), "null");
        assert_eq!(ExprTypeKind::empty_object().to_string(), "object");
        assert_eq!(
            ExprTypeKind::map_object(ExprTypeKind::string()).to_string(),
            "{string => string}"
        );
        assert_eq!(
            ExprTypeKind::array(ExprTypeKind::string()).to_string(),
            "array<string>"
        );
        let mut props = BTreeMap::new();
        props.insert("a".to_string(), ExprTypeKind::string());
        assert_eq!(
            ExprTypeKind::strict_object(props).to_string(),
            "{a: string}"
        );
    }

    #[test]
    fn assignability() {
        assert!(ExprTypeKind::any().assignable(&ExprTypeKind::string()));
        assert!(ExprTypeKind::string().assignable(&ExprTypeKind::any()));
        assert!(ExprTypeKind::string().assignable(&ExprTypeKind::number()));
        assert!(!ExprTypeKind::number().assignable(&ExprTypeKind::string()));
        assert!(ExprTypeKind::bool().assignable(&ExprTypeKind::any()));
        assert!(ExprTypeKind::null().assignable(&ExprTypeKind::null()));
        assert!(!ExprTypeKind::null().assignable(&ExprTypeKind::string()));
    }

    #[test]
    fn merge_types() {
        let merged = ExprTypeKind::number().merge(&ExprTypeKind::string());
        assert_eq!(merged.to_string(), "string");
        let merged = ExprTypeKind::number().merge(&ExprTypeKind::bool());
        assert_eq!(merged.to_string(), "any");
        let merged = ExprTypeKind::null().merge(&ExprTypeKind::null());
        assert_eq!(merged.to_string(), "null");
    }

    #[test]
    fn strict_object_assignable() {
        let mut a = BTreeMap::new();
        a.insert("x".to_string(), ExprTypeKind::number());
        let strict = ExprTypeKind::strict_object(a);
        // A loose object (mapped to any) is assignable: every prop accepts any.
        assert!(strict.assignable(&ExprTypeKind::empty_object()));
        assert!(strict.assignable(&ExprTypeKind::any()));

        let mut b = BTreeMap::new();
        b.insert("x".to_string(), ExprTypeKind::number());
        let strict2 = ExprTypeKind::strict_object(b);
        assert!(strict.assignable(&strict2));

        // A strict object missing a prop is not assignable.
        let mut c = BTreeMap::new();
        c.insert("y".to_string(), ExprTypeKind::number());
        let strict3 = ExprTypeKind::strict_object(c);
        assert!(!strict.assignable(&strict3));
    }

    #[test]
    fn json_value_types() {
        assert_eq!(
            type_of_json_value(&serde_json::json!(1)).to_string(),
            "number"
        );
        assert_eq!(
            type_of_json_value(&serde_json::json!("s")).to_string(),
            "string"
        );
        assert_eq!(
            type_of_json_value(&serde_json::json!(true)).to_string(),
            "bool"
        );
        assert_eq!(
            type_of_json_value(&serde_json::Value::Null).to_string(),
            "null"
        );
        assert_eq!(
            type_of_json_value(&serde_json::json!([1, 2])).to_string(),
            "array<number>"
        );
        assert_eq!(
            type_of_json_value(&serde_json::json!({"a": 1})).to_string(),
            "{a: number}"
        );
    }
}
