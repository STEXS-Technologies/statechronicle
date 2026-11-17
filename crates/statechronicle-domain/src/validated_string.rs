//! Shared trait shells for string newtypes whose parsing delegates to `new`.
//!
//! Each invocation stays beside its type and constructor. This macro owns no
//! validation rule: prefixes, limits, error variants and field visibility remain
//! with the individual type. Serialization deliberately preserves the raw string,
//! including values constructed through a public tuple field; deserialization
//! always validates through the constructor.
macro_rules! impl_validated_string {
    ($name:ident) => {
        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl ::core::str::FromStr for $name {
            type Err = $crate::error::DomainError;

            /// Parses the string using this type's validated constructor.
            ///
            /// # Errors
            ///
            /// Returns the validation errors documented by [`Self::new`].
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(String::from(s))
            }
        }
        impl TryFrom<String> for $name {
            type Error = $crate::error::DomainError;

            /// Converts an owned string using this type's validated constructor.
            ///
            /// # Errors
            ///
            /// Returns the validation errors documented by [`Self::new`].
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
        impl TryFrom<&str> for $name {
            type Error = $crate::error::DomainError;

            /// Converts a borrowed string using this type's validated constructor.
            ///
            /// # Errors
            ///
            /// Returns the validation errors documented by [`Self::new`].
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(String::from(value))
            }
        }
        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }
        impl<'de> ::serde::Deserialize<'de> for $name {
            /// Deserializes the wire string using this type's validated constructor.
            ///
            /// # Errors
            ///
            /// Returns a serde error for a non-string value or any validation
            /// error documented by [`Self::new`].
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                let value = <String as ::serde::Deserialize>::deserialize(deserializer)?;
                Self::new(value).map_err(::serde::de::Error::custom)
            }
        }
    };
}
pub(crate) use impl_validated_string;
